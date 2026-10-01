//! Conversations with a dot: ordinary agent sessions in the dot's project. `startChat`, routines,
//! triggers, hand-offs and team chats all start one here, and send follow-up messages into one
//! they keep reusing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::to_bytes;
use serde_json::{json, Map, Value};

use super::model::Dot;
use super::store;
use crate::domain::{DomainRepository, DomainStateError};
use crate::server::AppState;
use crate::storage::open_gxserver_database;

fn internal(message: impl Into<String>) -> DomainStateError {
    DomainStateError {
        code: "internalError",
        message: message.into(),
    }
}

/// `{ agentCommand, icon }` of the dot's agent from the launcher list, the way the desktop and the
/// CLI send it, so a custom agent launches even in a project that does not list it.
fn launch_settings_for(projects: &[Value], agent_id: &str) -> Value {
    let rows = crate::sidebar_hud::sidebar_agent_buttons_from_projects(projects);
    let mut settings = Map::new();
    if let Some(row) = rows.as_array().and_then(|rows| {
        rows.iter()
            .find(|row| row.get("agentId").and_then(Value::as_str) == Some(agent_id))
    }) {
        for (from, to) in [("command", "agentCommand"), ("icon", "icon")] {
            if let Some(value) = row
                .get(from)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                settings.insert(to.to_string(), json!(value));
            }
        }
    }
    Value::Object(settings)
}

/// Creates a session with the dot's agent in its project, queues `message` as its first prompt
/// and starts the agent. Returns `(projectId, sessionId)`. A `title` ("Routine · …", "From …") is
/// the conversation's name until Claude or Codex names it themselves: Ghostex adopts an agent's own
/// session name (CDXC:SessionTitles), so OpenDots finds its conversations by record, not by title.
pub(crate) async fn start_conversation(
    state: &Arc<AppState>,
    dot: &Dot,
    title: Option<&str>,
    message: Option<&str>,
) -> Result<(String, String), DomainStateError> {
    let created = {
        let state = state.clone();
        let dot = dot.clone();
        let title = title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string);
        let message = message
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(str::to_string);
        tokio::task::spawn_blocking(move || -> Result<(String, String), DomainStateError> {
            let db = open_gxserver_database(&state.paths)
                .map_err(|error| internal(format!("SQLite gxserver state error: {error}")))?;
            let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
            if repository.get_project(&dot.project_id)?.is_none() {
                return Err(DomainStateError::not_found(format!(
                    "{}'s project is gone. Delete the dot and make it again.",
                    dot.name
                )));
            }
            let projects = repository.list_projects()?;
            let mut params = Map::new();
            params.insert("projectId".to_string(), json!(dot.project_id));
            params.insert("agentId".to_string(), json!(dot.agent_id));
            params.insert(
                "launchSettings".to_string(),
                launch_settings_for(&projects, &dot.agent_id),
            );
            if let Some(title) = &title {
                params.insert("title".to_string(), json!(title));
                params.insert(
                    "runtimeSettings".to_string(),
                    json!({ "titleSource": "user" }),
                );
            }
            let project = repository.resolve_create_session_project(&params)?;
            let create_params =
                crate::agents::create_agent_session_params_for_project(&db, &project, &params)?;
            let created = repository.create_session(&create_params, false)?;
            let session = crate::agents::apply_created_session_identity(
                &repository,
                &created,
                &create_params,
            )?;
            let project_id = crate::server::value_text(&session, "projectId")?;
            let session_id = crate::server::value_text(&session, "sessionId")?;
            crate::server::restore_parked_project_for_new_session(
                &state,
                &db,
                &repository,
                &project_id,
            )?;
            crate::server::schedule_presentation_session_delta(
                &state,
                &db,
                &repository,
                &project_id,
                &session_id,
            )?;
            if let Some(message) = &message {
                let mut queue = Map::new();
                queue.insert("projectId".to_string(), json!(project_id));
                queue.insert("sessionId".to_string(), json!(session_id));
                queue.insert("text".to_string(), json!(message));
                queue.insert("startupSend".to_string(), json!(true));
                crate::session_chat_queue::handle_session_chat_queue_endpoint(
                    &state.paths,
                    state.metadata.server_id.as_str(),
                    "/api/queueSessionChatPrompt",
                    &queue,
                )?;
                crate::session_chat_queue_runtime::broadcast_session_chat_queue_state(
                    &state,
                    &project_id,
                    &session_id,
                );
            }
            Ok((project_id, session_id))
        })
        .await
        .map_err(|_| internal("The conversation could not be created."))??
    };
    let (project_id, session_id) = created;
    let body = json!({ "params": { "projectId": project_id, "sessionId": session_id } });
    let started = crate::server::handle_zmx_lifecycle_http(
        state,
        "/api/startSessionProvider".to_string(),
        uuid::Uuid::new_v4().to_string(),
        &body,
    )
    .await;
    if !started.response.status().is_success() {
        let bytes = to_bytes(started.response.into_body(), 64 * 1024)
            .await
            .unwrap_or_default();
        let detail = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|body| {
                body.get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string());
        discard_session(state, &project_id, &session_id).await;
        return Err(DomainStateError {
            code: "dependencyUnavailable",
            message: format!("{}'s agent did not start: {detail}", dot.name),
        });
    }
    Ok((project_id, session_id))
}

/// Removes a conversation whose agent never started, the way a failed worktree session is rolled
/// back: kill in case a provider did come up after the failure, then drop the row, so a routine
/// or trigger that keeps failing does not leave one dead session per run in the sidebar.
async fn discard_session(state: &Arc<AppState>, project_id: &str, session_id: &str) {
    let body = json!({ "params": { "projectId": project_id, "sessionId": session_id } });
    let _ = crate::server::handle_zmx_lifecycle_http(
        state,
        "/api/killSession".to_string(),
        uuid::Uuid::new_v4().to_string(),
        &body,
    )
    .await;
    let state = state.clone();
    let (project_id, session_id) = (project_id.to_string(), session_id.to_string());
    let _ = tokio::task::spawn_blocking(move || -> Result<(), DomainStateError> {
        let db = open_gxserver_database(&state.paths)
            .map_err(|error| internal(format!("SQLite gxserver state error: {error}")))?;
        let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
        let mut params = Map::new();
        params.insert("projectId".to_string(), json!(project_id));
        params.insert("sessionId".to_string(), json!(session_id));
        repository.remove_session(&params)?;
        crate::server::schedule_presentation_session_delta(
            &state,
            &db,
            &repository,
            &project_id,
            &session_id,
        )?;
        Ok(())
    })
    .await;
}

/// Sends a message into a session the way `ghostex agents send` does: typed now, picked up by a
/// busy agent at its next input boundary, and a sleeping session is woken for it.
pub(crate) async fn send_to_session(
    state: &AppState,
    project_id: &str,
    session_id: &str,
    text: &str,
) -> Result<(), String> {
    let body = json!({
        "params": { "projectId": project_id, "sessionId": session_id, "text": text }
    });
    let routed = crate::session_chat_send::handle_send_session_chat_message_http(
        state,
        "/api/sendSessionChatMessage".to_string(),
        uuid::Uuid::new_v4().to_string(),
        &body,
    )
    .await;
    if routed.response.status().is_success() {
        return Ok(());
    }
    let bytes = to_bytes(routed.response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    Err(serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|body| {
            body.get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string()))
}

/// Whether a session can still take messages (it exists and was not closed).
pub(crate) fn session_is_open(state: &AppState, project_id: &str, session_id: &str) -> bool {
    let Ok(db) = open_gxserver_database(&state.paths) else {
        return false;
    };
    let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
    repository
        .get_session(project_id, session_id)
        .ok()
        .flatten()
        .is_some_and(|session| {
            matches!(
                crate::presentation::effective_lifecycle_state(&session).as_str(),
                "running" | "sleeping"
            )
        })
}

/// One lock per kept conversation (`<dotId> <reference>`).
static CONVERSATION_LOCKS: Mutex<Option<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Mutex::new(None);

fn conversation_lock(dot_id: &str, reference: &str) -> Arc<tokio::sync::Mutex<()>> {
    CONVERSATION_LOCKS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .entry(format!("{dot_id} {reference}"))
        .or_default()
        .clone()
}

/// Sends `text` into the conversation a dot keeps for `reference` (a routine, a trigger, another
/// dot's hand-offs), or starts it titled `title` when it is gone. Returns `(projectId, sessionId)`.
/// Two overlapping calls for the same conversation run one after the other, so the second reuses
/// the conversation the first started instead of starting a duplicate.
pub(crate) async fn continue_or_start(
    state: &Arc<AppState>,
    dot: &Dot,
    reference: &str,
    title: &str,
    text: &str,
) -> Result<(String, String), DomainStateError> {
    let lock = conversation_lock(&dot.id, reference);
    let _held = lock.lock().await;
    let existing = {
        let paths = state.paths.clone();
        let dot_id = dot.id.clone();
        let reference = reference.to_string();
        tokio::task::spawn_blocking(move || {
            store::open(&paths).and_then(|db| store::conversation(&db, &dot_id, &reference))
        })
        .await
        .map_err(|_| internal("The conversation could not be read."))??
    };
    if let Some((project_id, session_id)) = existing {
        let open = {
            let state = state.clone();
            let project_id = project_id.clone();
            let session_id = session_id.clone();
            tokio::task::spawn_blocking(move || session_is_open(&state, &project_id, &session_id))
                .await
                .unwrap_or(false)
        };
        if open
            && send_to_session(state, &project_id, &session_id, text)
                .await
                .is_ok()
        {
            return Ok((project_id, session_id));
        }
    }
    let (project_id, session_id) = start_conversation(state, dot, Some(title), Some(text)).await?;
    let paths = state.paths.clone();
    let dot_id = dot.id.clone();
    let reference = reference.to_string();
    let (stored_project, stored_session) = (project_id.clone(), session_id.clone());
    let _ = tokio::task::spawn_blocking(move || {
        store::open(&paths).and_then(|db| {
            store::set_conversation(&db, &dot_id, &reference, &stored_project, &stored_session)
        })
    })
    .await;
    Ok((project_id, session_id))
}
