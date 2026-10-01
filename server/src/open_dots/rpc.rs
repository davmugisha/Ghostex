//! `/api/openDotsRpc`: every method in CONTRACT §2. The desktop view, the CLI (`ghostex dots`) and
//! the phone (through `ghostex dots rpc`) share this one list.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{json, Map, Value};

use super::model::{to_value, Dot};
use super::{
    approvals, browser, composio, conversations, handoffs, instructions, projects, publish,
    routines, snapshot, store, tools, triggers, updates, vault,
};
use crate::domain::DomainStateError;
use crate::server::AppState;

pub(crate) const OPEN_DOTS_OFF_MESSAGE: &str =
    "OpenDots is off. Turn it on in Settings > Extensions.";

fn text(params: &Map<String, Value>, key: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn required(params: &Map<String, Value>, key: &str) -> Result<String, DomainStateError> {
    let value = text(params, key);
    if value.is_empty() {
        return Err(DomainStateError::bad_request(format!("{key} is required.")));
    }
    Ok(value)
}

/// Runs blocking work (SQLite, files, Chrome, Composio) off the async runtime.
async fn blocking<T: Send + 'static>(
    state: &Arc<AppState>,
    work: impl FnOnce(&AppState) -> Result<T, DomainStateError> + Send + 'static,
) -> Result<T, DomainStateError> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || work(&state))
        .await
        .map_err(|_| DomainStateError {
            code: "internalError",
            message: "OpenDots stopped unexpectedly while doing that.".to_string(),
        })?
}

fn require_dot(state: &AppState, dot_id: &str) -> Result<Dot, DomainStateError> {
    let db = store::open(&state.paths)?;
    store::require_dot(&db, dot_id)
}

fn redirect_base(state: &AppState) -> String {
    format!(
        "http://{}:{}",
        state.config.listeners.local.host, state.config.listeners.local.port
    )
}

/// Every change ends here: dots that read the change get fresh files, and clients hear about it.
fn changed(state: &AppState, dot_id: Option<&str>) {
    match dot_id {
        Some(dot_id) => instructions::refresh_dot(&state.paths, dot_id),
        None => instructions::refresh_all(&state.paths),
    }
    publish::publish_changes(state);
}

pub(crate) async fn dispatch(
    state: &Arc<AppState>,
    method: &str,
    params: Map<String, Value>,
) -> Result<Value, DomainStateError> {
    let enabled = {
        let paths = state.paths.clone();
        tokio::task::spawn_blocking(move || super::open_dots_enabled(&paths))
            .await
            .unwrap_or(false)
    };
    if method == "snapshot" {
        return blocking(state, move |state| {
            if enabled {
                snapshot::snapshot(state)
            } else {
                Ok(snapshot::disabled_snapshot(state))
            }
        })
        .await;
    }
    // Internal: what `ghostex sessions --json --mobile-summary` adds to dot project rows.
    if method == "rowFacts" {
        return blocking(state, move |state| {
            Ok(json!({ "enabled": enabled, "dots": publish::row_facts_value(&state.paths) }))
        })
        .await;
    }
    if !enabled {
        return Err(DomainStateError::bad_request(OPEN_DOTS_OFF_MESSAGE));
    }
    let p = params;
    match method {
        // ------------------------------------------------------------ dots
        "createDot" => {
            blocking(state, move |state| {
                projects::create_dot(state, &p).map(|dot| json!({ "dot": to_value(&dot) }))
            })
            .await
        }
        "updateDot" => {
            blocking(state, move |state| {
                projects::update_dot(state, &p).map(|dot| json!({ "dot": to_value(&dot) }))
            })
            .await
        }
        "deleteDot" => {
            projects::delete_dot(state, &required(&p, "dotId")?).await?;
            Ok(json!({}))
        }
        "pauseDot" | "resumeDot" => {
            let dot = projects::set_paused(state, &required(&p, "dotId")?, method == "pauseDot")
                .await?;
            Ok(json!({ "dot": to_value(&dot) }))
        }
        "stopDot" => {
            let dot_id = required(&p, "dotId")?;
            let dot = blocking(state, move |state| require_dot(state, &dot_id)).await?;
            projects::stop_dot_work(state, &dot).await;
            blocking(state, |state| {
                publish::publish_changes(state);
                Ok(())
            })
            .await?;
            Ok(json!({ "dot": to_value(&dot) }))
        }
        "startChat" => {
            let dot_id = required(&p, "dotId")?;
            let dot = blocking(state, move |state| require_dot(state, &dot_id)).await?;
            let message = text(&p, "message");
            let title = text(&p, "title");
            let (project_id, session_id) = conversations::start_conversation(
                state,
                &dot,
                (!title.is_empty()).then_some(title.as_str()),
                (!message.is_empty()).then_some(message.as_str()),
            )
            .await?;
            blocking(state, |state| {
                publish::publish_changes(state);
                Ok(())
            })
            .await?;
            Ok(json!({ "projectId": project_id, "sessionId": session_id }))
        }
        "startTeamChat" => {
            let member_ids: Vec<String> = p
                .get("memberIds")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            handoffs::start_team_chat(
                state,
                &text(&p, "name"),
                &required(&p, "leadId")?,
                &member_ids,
                &text(&p, "message"),
            )
            .await
        }
        "deleteTeam" => {
            let team_id = required(&p, "teamId")?;
            blocking(state, move |state| {
                let db = store::open(&state.paths)?;
                store::delete_team(&db, &team_id)?;
                publish::broadcast_changed(state);
                Ok(json!({}))
            })
            .await
        }
        // ------------------------------------------------------------ rules, memory, skills
        "addRule" => {
            blocking(state, move |state| {
                let action = required(&p, "action")?;
                let decision = required(&p, "decision")?;
                if !matches!(decision.as_str(), "allow" | "ask" | "never") {
                    return Err(DomainStateError::bad_request(
                        "decision must be allow, ask or never.",
                    ));
                }
                let dot_id = Some(text(&p, "dotId")).filter(|id| !id.is_empty());
                let db = store::open(&state.paths)?;
                if let Some(dot_id) = &dot_id {
                    store::require_dot(&db, dot_id)?;
                }
                let rule = store::add_rule(&db, dot_id.as_deref(), &action, &decision)?;
                changed(state, dot_id.as_deref());
                Ok(json!({ "rule": to_value(&rule) }))
            })
            .await
        }
        "deleteRule" => {
            blocking(state, move |state| {
                let rule_id = required(&p, "ruleId")?;
                let db = store::open(&state.paths)?;
                let owner = store::get_rule(&db, &rule_id)?.and_then(|rule| rule.dot_id);
                store::delete_rule(&db, &rule_id)?;
                changed(state, owner.as_deref());
                Ok(json!({}))
            })
            .await
        }
        "addMemory" => {
            blocking(state, move |state| {
                let dot_id = required(&p, "dotId")?;
                let fact = required(&p, "text")?;
                let db = store::open(&state.paths)?;
                store::require_dot(&db, &dot_id)?;
                let memory = store::add_memory(&db, &dot_id, &fact)?;
                changed(state, Some(&dot_id));
                Ok(json!({ "memory": to_value(&memory) }))
            })
            .await
        }
        "deleteMemory" => {
            blocking(state, move |state| {
                let memory_id = required(&p, "memoryId")?;
                let db = store::open(&state.paths)?;
                let owner = store::get_memory(&db, &memory_id)?.map(|memory| memory.dot_id);
                store::delete_memory(&db, &memory_id)?;
                if let Some(owner) = owner {
                    changed(state, Some(&owner));
                }
                Ok(json!({}))
            })
            .await
        }
        "saveSkill" => {
            blocking(state, move |state| {
                let dot_id = required(&p, "dotId")?;
                let name = required(&p, "name")?;
                let db = store::open(&state.paths)?;
                store::require_dot(&db, &dot_id)?;
                let skill = store::upsert_skill(
                    &db,
                    &dot_id,
                    &name,
                    &text(&p, "description"),
                    p.get("body").and_then(Value::as_str).unwrap_or_default(),
                )?;
                changed(state, Some(&dot_id));
                Ok(json!({ "skill": to_value(&skill) }))
            })
            .await
        }
        "deleteSkill" => {
            blocking(state, move |state| {
                let skill_id = required(&p, "skillId")?;
                let db = store::open(&state.paths)?;
                let owner = store::get_skill(&db, &skill_id)?.map(|skill| skill.dot_id);
                store::delete_skill(&db, &skill_id)?;
                if let Some(owner) = owner {
                    changed(state, Some(&owner));
                }
                Ok(json!({}))
            })
            .await
        }
        // ------------------------------------------------------------ routines
        "addRoutine" => {
            blocking(state, move |state| {
                let routine = routines::add_routine(
                    &state.paths,
                    &required(&p, "dotId")?,
                    &text(&p, "name"),
                    &text(&p, "instruction"),
                    &text(&p, "schedule"),
                )?;
                publish::publish_changes(state);
                Ok(json!({ "routine": to_value(&routine) }))
            })
            .await
        }
        "updateRoutine" => {
            blocking(state, move |state| {
                let routine =
                    routines::update_routine(&state.paths, &required(&p, "routineId")?, &p)?;
                publish::publish_changes(state);
                Ok(json!({ "routine": to_value(&routine) }))
            })
            .await
        }
        "deleteRoutine" => {
            blocking(state, move |state| {
                let routine_id = required(&p, "routineId")?;
                let db = store::open(&state.paths)?;
                let owner = store::get_routine(&db, &routine_id)?.map(|routine| routine.dot_id);
                store::delete_routine(&db, &routine_id)?;
                if let Some(owner) = owner {
                    changed(state, Some(&owner));
                }
                Ok(json!({}))
            })
            .await
        }
        "runRoutineNow" => {
            let (project_id, session_id) =
                routines::run_routine(state, &required(&p, "routineId")?, true).await?;
            Ok(json!({ "projectId": project_id, "sessionId": session_id }))
        }
        // ------------------------------------------------------------ saved logins
        "addPassword" => {
            blocking(state, move |state| {
                let site = required(&p, "site")?;
                let username = required(&p, "username")?;
                let password = p
                    .get("password")
                    .and_then(Value::as_str)
                    .filter(|password| !password.is_empty())
                    .ok_or_else(|| DomainStateError::bad_request("password is required."))?;
                let sealed =
                    vault::seal(&state.paths, password).map_err(DomainStateError::bad_request)?;
                let db = store::open(&state.paths)?;
                let saved = store::insert_password(&db, &site, &username, &sealed)?;
                changed(state, None);
                Ok(json!({ "password": to_value(&saved) }))
            })
            .await
        }
        "deletePassword" => {
            blocking(state, move |state| {
                let db = store::open(&state.paths)?;
                store::delete_password(&db, &required(&p, "passwordId")?)?;
                changed(state, None);
                Ok(json!({}))
            })
            .await
        }
        // ------------------------------------------------------------ approvals, updates
        "resolveApproval" => {
            blocking(state, move |state| {
                let answer = p.get("answer").and_then(Value::as_str);
                let approval = approvals::resolve(
                    state,
                    &required(&p, "approvalId")?,
                    &required(&p, "choice")?,
                    answer,
                )?;
                Ok(json!({ "approval": to_value(&approval) }))
            })
            .await
        }
        "markUpdatesRead" => {
            blocking(state, move |state| {
                updates::mark_read(state, &p)?;
                Ok(json!({}))
            })
            .await
        }
        "listFiles" => {
            blocking(state, move |state| {
                Ok(snapshot::list_files(&require_dot(state, &required(&p, "dotId")?)?))
            })
            .await
        }
        // ------------------------------------------------------------ the dot's browser
        "browserScreenshot" => {
            blocking(state, move |state| {
                let dot = require_dot(state, &required(&p, "dotId")?)?;
                let wake = p.get("wake").and_then(Value::as_bool).unwrap_or(false);
                let shot = browser::live_screenshot(&state.paths, &dot.id, wake)
                    .map_err(DomainStateError::bad_request)?;
                Ok(json!({
                    "image": shot.jpeg.map(|jpeg| base64::engine::general_purpose::STANDARD.encode(jpeg)),
                    "mime": "image/jpeg",
                    "url": shot.url,
                    "title": shot.title,
                    "width": browser::SCREEN_WIDTH,
                    "height": browser::SCREEN_HEIGHT,
                    "live": shot.live,
                    "takenOver": shot.taken_over,
                }))
            })
            .await
        }
        "browserInput" => {
            blocking(state, move |state| {
                let dot = require_dot(state, &required(&p, "dotId")?)?;
                browser::user_input(&state.paths, &dot.id, &Value::Object(p))
                    .map_err(DomainStateError::bad_request)?;
                Ok(json!({}))
            })
            .await
        }
        "browserTakeOver" | "browserHandBack" => {
            let taken_over = method == "browserTakeOver";
            blocking(state, move |state| {
                let dot = require_dot(state, &required(&p, "dotId")?)?;
                browser::set_taken_over(&dot.id, taken_over);
                publish::broadcast_changed(state);
                Ok(json!({ "takenOver": browser::is_taken_over(&dot.id) }))
            })
            .await
        }
        "browserReset" => {
            blocking(state, move |state| {
                let dot = require_dot(state, &required(&p, "dotId")?)?;
                browser::set_taken_over(&dot.id, false);
                browser::reset(&state.paths, &dot.id).map_err(DomainStateError::bad_request)?;
                publish::broadcast_changed(state);
                Ok(json!({}))
            })
            .await
        }
        // ------------------------------------------------------------ apps (Composio)
        "composioSignIn" => {
            blocking(state, move |state| {
                let url = composio::sign_in(&state.paths, &redirect_base(state))
                    .map_err(DomainStateError::bad_request)?;
                if url.is_none() {
                    changed(state, None);
                }
                Ok(json!({ "url": url }))
            })
            .await
        }
        "composioSignOut" | "composioRefresh" => {
            let sign_out = method == "composioSignOut";
            blocking(state, move |state| {
                if sign_out {
                    composio::sign_out(&state.paths);
                } else {
                    composio::refresh(&state.paths).map_err(DomainStateError::bad_request)?;
                }
                changed(state, None);
                Ok(json!({ "settings": snapshot::settings_info(state) }))
            })
            .await
        }
        "setTriggersKey" => {
            blocking(state, move |state| {
                let key = p.get("key").and_then(Value::as_str).unwrap_or_default();
                let error = composio::save_triggers_key(&state.paths, key.trim());
                if error.is_none() {
                    triggers::start_events(state);
                    publish::broadcast_changed(state);
                }
                Ok(json!({ "error": error }))
            })
            .await
        }
        "triggerApps" => {
            blocking(state, move |state| {
                let apps =
                    composio::trigger_apps(&state.paths).map_err(DomainStateError::bad_request)?;
                Ok(json!({ "apps": apps }))
            })
            .await
        }
        "connectTriggerApp" => {
            blocking(state, move |state| {
                let url = composio::connect_trigger_app(&state.paths, &required(&p, "toolkit")?)
                    .map_err(DomainStateError::bad_request)?;
                Ok(json!({ "url": url }))
            })
            .await
        }
        // The dots' own app connection (Composio For You): Settings' "Connect more" (workstream E).
        "composioConnectApp" => {
            blocking(state, move |state| {
                let start = composio::start_connect(&state.paths, &required(&p, "toolkit")?)
                    .map_err(DomainStateError::bad_request)?;
                if start.already_connected {
                    changed(state, None);
                }
                Ok(json!({ "url": start.url }))
            })
            .await
        }
        "triggerTypes" => {
            blocking(state, move |state| {
                let types = composio::trigger_types(&state.paths, &required(&p, "toolkit")?)
                    .map_err(DomainStateError::bad_request)?;
                Ok(json!({ "types": types }))
            })
            .await
        }
        "addTrigger" => {
            blocking(state, move |state| {
                let trigger = triggers::add_trigger(state, &p)?;
                publish::publish_changes(state);
                Ok(json!({ "trigger": to_value(&trigger) }))
            })
            .await
        }
        "setTriggerEnabled" => {
            blocking(state, move |state| {
                let enabled = p
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| DomainStateError::bad_request("enabled is required."))?;
                let trigger = triggers::set_enabled(state, &required(&p, "triggerId")?, enabled)?;
                publish::publish_changes(state);
                Ok(json!({ "trigger": to_value(&trigger) }))
            })
            .await
        }
        "deleteTrigger" => {
            blocking(state, move |state| {
                triggers::delete(state, &required(&p, "triggerId")?)?;
                publish::publish_changes(state);
                Ok(json!({}))
            })
            .await
        }
        // ------------------------------------------------------------ the MCP server
        "toolList" => {
            blocking(state, move |state| {
                require_dot(state, &required(&p, "dotId")?)?;
                Ok(json!({ "tools": tools::tool_list(&state.paths) }))
            })
            .await
        }
        // Internal: `ghostex dots mcp` tells gxserver the agent cancelled a waiting call.
        "cancelToolCall" => {
            blocking(state, move |state| {
                let cancelled = approvals::cancel_call(state, &required(&p, "callId")?);
                Ok(json!({ "cancelled": cancelled }))
            })
            .await
        }
        other => Err(DomainStateError::bad_request(format!(
            "Unknown OpenDots method \"{other}\"."
        ))),
    }
}
