use serde_json::{json, Map, Value};

use crate::ghostex_cli::args::{parse_args, Flags};
use crate::ghostex_cli::output::{is_failed_cli_result, print_json};
use crate::ghostex_cli::rpc::{call_gxserver_rpc, CliError, CliResult};
use crate::ghostex_cli::{actions, sessions_brief, set_exit_code};

use super::*;

// ---------------------------------------------------------------------------
// sessions command
// ---------------------------------------------------------------------------

pub fn sessions_command(args: &[String]) -> CliResult<()> {
    let parsed = parse_args(args);
    let flags = &parsed.flags;
    /*
     * CDXC:AgentLauncher 2026-07-12-00:00:
     * The mobile summary is the one poll React Native Android makes, so it also carries
     * the gxserver-owned agent launcher rows and per-project quick actions.
     * HUD fetch failures must never break the session list; mobile simply
     * hides the launcher rows until the next poll.
     */
    if flags.truthy("json") && flags.truthy("mobileSummary") {
        let result = fetch_session_list_result(flags, true)?;
        let hud = fetch_mobile_sidebar_hud(flags).ok();
        let mut merged = match to_mobile_session_list(&result) {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        if let Some(Value::Object(hud_map)) = hud {
            for (key, value) in hud_map {
                merged.insert(key, value);
            }
        }
        apply_open_dots_summary(&mut merged, flags);
        print_json(&Value::Object(merged));
        return Ok(());
    }
    let result = fetch_session_list_result(flags, true)?;
    if flags.truthy("json") {
        if flags.truthy("full") {
            print_json(&result);
        } else {
            print_json(&sessions_brief::brief_session_list(&result, flags));
        }
        return Ok(());
    }
    let empty = Vec::new();
    let sessions = result
        .get("sessions")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let grouped = !strict_true(flags, "ungrouped") && !strict_true(flags, "u");
    print_session_list(sessions, grouped);
    Ok(())
}

fn fetch_mobile_sidebar_hud(flags: &Flags) -> CliResult<Value> {
    let hud = call_gxserver_rpc(
        "/api/readSidebarHud",
        &json!({ "includeAllProjectCommands": true }),
        flags,
    )?;
    let mut agents: Vec<Value> = Vec::new();
    if let Some(list) = hud.get("agents").and_then(Value::as_array) {
        for agent in list {
            let has_agent_id =
                matches!(agent.get("agentId"), Some(Value::String(text)) if !text.is_empty());
            if !has_agent_id {
                continue;
            }
            let mut map = Map::new();
            insert_present(&mut map, "agentId", agent.get("agentId"));
            insert_present(&mut map, "icon", agent.get("icon"));
            insert_present(&mut map, "name", agent.get("name"));
            agents.push(Value::Object(map));
        }
    }
    let mut quick_actions_by_project = Map::new();
    if let Some(by_project) = hud.get("commandsByProject").and_then(Value::as_object) {
        for (project_id, commands) in by_project {
            let mut actions_list: Vec<Value> = Vec::new();
            if let Some(commands) = commands.as_array() {
                for command in commands {
                    if !is_configured_mobile_quick_action(command) {
                        continue;
                    }
                    let mut map = Map::new();
                    insert_present(&mut map, "actionType", command.get("actionType"));
                    insert_present(&mut map, "commandId", command.get("commandId"));
                    insert_present(&mut map, "icon", command.get("icon"));
                    insert_present(&mut map, "name", command.get("name"));
                    if command.get("actionType").and_then(Value::as_str) == Some("browser") {
                        insert_present(&mut map, "url", command.get("url"));
                    }
                    actions_list.push(Value::Object(map));
                }
            }
            if !actions_list.is_empty() {
                quick_actions_by_project.insert(project_id.clone(), Value::Array(actions_list));
            }
        }
    }
    Ok(json!({
        "agents": agents,
        "quickActionsByProject": quick_actions_by_project,
    }))
}

pub(super) fn is_configured_mobile_quick_action(command: &Value) -> bool {
    let has_command_id =
        matches!(command.get("commandId"), Some(Value::String(text)) if !text.is_empty());
    if !has_command_id {
        return false;
    }
    if command.get("actionType").and_then(Value::as_str) == Some("browser") {
        return matches!(command.get("url"), Some(Value::String(text)) if !text.trim().is_empty());
    }
    matches!(command.get("command"), Some(Value::String(text)) if !text.trim().is_empty())
}

pub fn run_quick_action_command(args: &[String]) -> CliResult<()> {
    /*
     * CDXC:Mobile 2026-07-12-00:00:
     * Mobile quick actions cannot reuse `run-command`: that routes through
     * gxserver renderer commands into the desktop app's command pane, which
     * the phone cannot see. `run-action` resolves the trusted HUD command on
     * the Mac (mobile sends only ids) and materializes a normal gxserver
     * terminal session running it, so the phone can attach like any other
     * session. Browser actions return the URL for the phone to open locally.
     */
    let parsed = parse_args(args);
    let flags = &parsed.flags;
    let command_id = flags
        .text("commandId")
        .or_else(|| parsed.rest.first().cloned())
        .unwrap_or_default()
        .trim()
        .to_string();
    let project_id = normalize_required_project_id(flags.text("projectId"), "run-action")?;
    if command_id.is_empty() {
        return Err(CliError::Other(
            "run-action requires a command id.".to_string(),
        ));
    }
    let hud = call_gxserver_rpc(
        "/api/readSidebarHud",
        &json!({ "activeProjectId": project_id }),
        flags,
    )?;
    let action = hud
        .get("commands")
        .and_then(Value::as_array)
        .and_then(|commands| {
            commands.iter().find(|command| {
                command.get("commandId").and_then(Value::as_str) == Some(command_id.as_str())
            })
        });
    let action = match action {
        Some(action) if is_configured_mobile_quick_action(action) => action,
        _ => {
            print_json(&json!({
                "commandId": command_id,
                "error": "Quick action is not configured for this project.",
                "ok": false,
            }));
            set_exit_code(1);
            return Ok(());
        }
    };
    if action.get("actionType").and_then(Value::as_str) == Some("browser") {
        let mut map = Map::new();
        map.insert("actionType".to_string(), json!("browser"));
        map.insert("commandId".to_string(), json!(command_id));
        insert_present(&mut map, "name", action.get("name"));
        map.insert("ok".to_string(), json!(true));
        insert_present(&mut map, "url", action.get("url"));
        print_json(&Value::Object(map));
        return Ok(());
    }
    let payload = {
        let mut map = Map::new();
        insert_present(&mut map, "command", action.get("command"));
        map.insert("projectId".to_string(), json!(project_id));
        map.insert("start".to_string(), json!(true));
        map.insert(
            "title".to_string(),
            match action.get("name") {
                Some(name) if js_truthy(Some(name)) => name.clone(),
                _ => json!("Action"),
            },
        );
        Value::Object(map)
    };
    let created = match actions::send_gxserver_cli_action("createSession", &payload, flags) {
        Ok(created) => created,
        Err(error) => {
            print_json(&json!({
                "actionType": "terminal",
                "commandId": command_id,
                "error": error.to_string(),
                "ok": false,
            }));
            set_exit_code(1);
            return Ok(());
        }
    };
    let session_id = created
        .get("session")
        .and_then(|session| session.get("sessionId"));
    if is_failed_cli_result(&created) || !js_truthy(session_id) {
        print_json(&json!({
            "actionType": "terminal",
            "commandId": command_id,
            "error": "Could not start the quick action session.",
            "ok": false,
        }));
        set_exit_code(1);
        return Ok(());
    }
    let mut merged = created.as_object().cloned().unwrap_or_default();
    merged.insert("actionType".to_string(), json!("terminal"));
    merged.insert("commandId".to_string(), json!(command_id));
    merged.insert("ok".to_string(), json!(true));
    print_json(&Value::Object(merged));
    Ok(())
}

fn normalize_required_project_id(value: Option<String>, command_name: &str) -> CliResult<String> {
    let project_id = value.unwrap_or_default().trim().to_string();
    if project_id.is_empty() {
        return Err(CliError::Other(format!(
            "{command_name} requires --project-id until gxserver active-project routing lands."
        )));
    }
    Ok(project_id)
}
