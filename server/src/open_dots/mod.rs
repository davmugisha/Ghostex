//! OpenDots: personal agents ("dots") that work on their own, ported from Open Dot
//! (github.com/composio-community/open-dot) onto Ghostex's agent sessions.
//!
//! A dot is an ordinary gxserver project flagged in `launchSettings` (`isDot`, `dotId`), and every
//! conversation with it is an ordinary agent session in that project, so chat, approvals, sleep,
//! resume and the phone work unchanged. gxserver owns everything Open Dot's runtime gave the model:
//! rules and approvals, memory, skills, routines, triggers, the password vault, the dot's own
//! browser, Composio apps and dot-to-dot hand-offs. The agent reaches them through the stdio MCP
//! server `ghostex dots mcp`, which forwards every tool call to `/api/openDotsTool`.
//!
//! CDXC:OpenDots 2026-10-01 DECISION:
//! User: build Open Dot's personal agents into Ghostex as "OpenDots", the way the Hermes Bots were done, in the desktop app and in the phone app (not Open Dot's own desktop app), as a new extension on Settings > Extensions that is built into Ghostex for now, shown as a new Space in the sidebar when enabled, bringing every Open Dot feature that makes sense.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! A dot runs on the agent CLIs Ghostex already runs (Claude Code by default) instead of Open Dot's own OpenAI Responses-API loop: the chat, approval cards, sleep/wake, resume, notifications and the phone chat then work unchanged, and the user's existing agent subscriptions power the dots without an OpenAI key.
//! SEE-ALSO: docs/2026-10-01/opendots/CONTRACT.md (local), server/src/agents/launch_plan.rs (`apply_dot_launch_flags`), server/src/ghostex_cli/dots/ (CLI + MCP server), packages/gx-core/src/sidebar_view/inputs.rs (`SidebarMode::Dots`), apps/desktop/src/app/native_open_dots/, apps/mobile/app/src/dots/.

pub(crate) mod browser;
pub(crate) mod composio;
pub(crate) mod vault;

mod approvals;
mod conversations;
mod cron;
mod handoffs;
mod instructions;
pub(crate) mod launch;
mod model;
mod projects;
mod publish;
mod review;
mod routines;
mod rpc;
mod runtime;
mod snapshot;
mod store;
mod tools;
mod triggers;
mod updates;

pub(crate) use publish::presentation_fields as dot_presentation_fields;
pub(crate) use store::{get_setting, open_open_dots_database, set_setting};

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Map, Value};

use crate::domain::{read_domain_rpc_params, DomainStateError};
use crate::paths::GxserverPaths;
use crate::protocol::rpc_success;
use crate::server::{domain_error_response, routed_json, AppState, RoutedResponse};

/// The settings key the Official extension switch writes (inverted: `false` = on).
pub(crate) const OPEN_DOTS_HIDDEN_SETTINGS_KEY: &str = "openDotsHidden";
/// The MCP server name every dot session sees (`mcp__ghostex-dots__<tool>` in Claude Code).
pub(crate) const OPEN_DOTS_MCP_SERVER_NAME: &str = "ghostex-dots";
/// The event every change broadcasts; clients refetch `snapshot`.
pub(crate) const OPEN_DOTS_CHANGED_EVENT_TYPE: &str = "openDotsChanged";

/// `openDotsHidden` is an inverted Official-extension key that defaults to hidden, so only an
/// explicit `false` in the desktop's settings file turns OpenDots on.
pub(crate) fn open_dots_enabled(paths: &GxserverPaths) -> bool {
    crate::session_lifecycle::read_sidebar_settings(paths).and_then(|settings| {
        settings
            .get(OPEN_DOTS_HIDDEN_SETTINGS_KEY)
            .and_then(Value::as_bool)
    }) == Some(false)
}

/// The dot a project stands for: `launchSettings.isDot` + `launchSettings.dotId`. `None` for every
/// other project.
pub(crate) fn dot_id_of_project(project: &Value) -> Option<&str> {
    let launch_settings = project.get("launchSettings")?;
    if launch_settings.get("isDot") != Some(&Value::Bool(true)) {
        return None;
    }
    launch_settings
        .get("dotId")?
        .as_str()
        .filter(|id| id.starts_with("dot_"))
}

/// `/api/openDotsRpc`: `{ method, params }`.
pub(crate) async fn handle_open_dots_rpc_http(
    state: &Arc<AppState>,
    endpoint_path: String,
    request_id: String,
    body: &Value,
) -> RoutedResponse {
    let params = match read_domain_rpc_params(body) {
        Ok(params) => params,
        Err(error) => return domain_error_response(endpoint_path, request_id, error),
    };
    let Some(method) = params
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(str::to_string)
    else {
        return domain_error_response(
            endpoint_path,
            request_id,
            DomainStateError::bad_request("method is required."),
        );
    };
    let method_params = params
        .get("params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    match rpc::dispatch(state, &method, method_params).await {
        Ok(result) => routed_json(
            Some(endpoint_path),
            StatusCode::OK,
            rpc_success(request_id, result),
        ),
        Err(error) => domain_error_response(endpoint_path, request_id, error),
    }
}

fn tool_text(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// `/api/openDotsTool`: one MCP tool call from `ghostex dots mcp`
/// (`{ dotId, projectId?, sessionId?, callId?, tool, arguments }`). Waits while an approval card is
/// pending (up to 30 minutes).
pub(crate) async fn handle_open_dots_tool_http(
    state: &Arc<AppState>,
    endpoint_path: String,
    request_id: String,
    body: &Value,
) -> RoutedResponse {
    let params = match read_domain_rpc_params(body) {
        Ok(params) => params,
        Err(error) => return domain_error_response(endpoint_path, request_id, error),
    };
    let reply = |result: Value| {
        routed_json(
            Some(endpoint_path.clone()),
            StatusCode::OK,
            rpc_success(request_id.clone(), result),
        )
    };
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let (Some(dot_id), Some(tool)) = (text("dotId"), text("tool")) else {
        return domain_error_response(
            endpoint_path.clone(),
            request_id.clone(),
            DomainStateError::bad_request("dotId and tool are required."),
        );
    };
    let looked_up = {
        let paths = state.paths.clone();
        tokio::task::spawn_blocking(move || {
            if !open_dots_enabled(&paths) {
                return Err(rpc::OPEN_DOTS_OFF_MESSAGE.to_string());
            }
            store::open(&paths)
                .and_then(|db| store::require_dot(&db, &dot_id))
                .map_err(|error| error.message)
        })
        .await
        .unwrap_or_else(|_| Err("OpenDots stopped unexpectedly.".to_string()))
    };
    let dot = match looked_up {
        Ok(dot) => dot,
        Err(message) => return reply(tool_text(&message)),
    };
    let context = tools::ToolContext {
        project_id: text("projectId").unwrap_or_else(|| dot.project_id.clone()),
        session_id: text("sessionId"),
        call_id: text("callId"),
        dot,
    };
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    let output = tools::call_tool(state, context, &tool, arguments).await;
    reply(output.to_value())
}

/// The chat card's answer for a dot approval card (CONTRACT §6). `None` when the session's stored
/// prompt is not one, or the answer is not a question answer (`kind` other than `"question"`), so
/// the caller answers it the ordinary way.
pub(crate) async fn resolve_approval_from_chat(
    state: &AppState,
    session: &Value,
    params: &Map<String, Value>,
) -> Option<Result<Value, DomainStateError>> {
    if params.get("kind").and_then(Value::as_str) != Some("question") {
        return None;
    }
    let stored = crate::agents::session_chat_prompt_setting(session)?;
    let prompt = crate::session_chat_interactive::parse_stored_session_chat_prompt(&stored)?;
    approvals::approval_id_of_prompt(&prompt)?;
    let state = state.clone();
    let session = session.clone();
    let params = params.clone();
    tokio::task::spawn_blocking(move || approvals::answer_from_chat(&state, &session, &params))
        .await
        .ok()
        .flatten()
}

/// Starts the OpenDots background work (routine scheduler, hand-off supervisor, trigger events).
pub(crate) fn start_open_dots_runtime(state: Arc<AppState>) {
    runtime::start(state);
}
