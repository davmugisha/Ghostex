//! The `snapshot` every client draws from: the records, each dot's derived status, and the settings
//! the OpenDots view shows (apps, vault, browser, the agents a dot can run on).

use serde_json::{json, Map, Value};

use super::model::{to_value, Dot};
use super::{browser, composio, store, vault};
use crate::coordinators::{classify_thread_session, ThreadProgress, ThreadState};
use crate::domain::{DomainRepository, DomainStateError};
use crate::paths::GxserverPaths;
use crate::server::AppState;
use crate::storage::open_gxserver_database;

/// CONTRACT §1 Dot.status: `paused > waiting > working > idle`, with what it is doing.
pub(crate) fn derive_status(
    repository: &DomainRepository<'_>,
    dot: &Dot,
    pending: usize,
    now_iso: &str,
) -> (String, Option<String>) {
    if dot.paused {
        return ("paused".to_string(), None);
    }
    let sessions = repository
        .list_sessions(Some(&dot.project_id))
        .unwrap_or_default();
    let mut working = None;
    let mut waiting = None;
    for session in &sessions {
        let title = session
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("a chat")
            .to_string();
        match classify_thread_session(
            Some(session),
            ThreadProgress {
                resolved: false,
                has_run: true,
            },
            now_iso,
            false,
        ) {
            ThreadState::Waiting if waiting.is_none() => waiting = Some(title),
            ThreadState::Working if working.is_none() => working = Some(title),
            _ => {}
        }
    }
    if pending > 0 {
        let activity = if pending == 1 {
            "Needs your approval".to_string()
        } else {
            format!("{pending} things need you")
        };
        return ("waiting".to_string(), Some(activity));
    }
    if let Some(title) = waiting {
        return (
            "waiting".to_string(),
            Some(format!("Waiting for you in “{title}”")),
        );
    }
    if let Some(title) = working {
        return ("working".to_string(), Some(format!("Working in “{title}”")));
    }
    ("idle".to_string(), None)
}

/// Every dot with its status and activity filled in.
pub(crate) fn dots_with_status(
    state: &AppState,
    db: &rusqlite::Connection,
) -> Result<Vec<Dot>, DomainStateError> {
    let mut dots = store::list_dots(db)?;
    let pending = store::pending_approvals(db)?;
    let gx_db = open_gxserver_database(&state.paths).map_err(|error| DomainStateError {
        code: "internalError",
        message: format!("SQLite gxserver state error: {error}"),
    })?;
    let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
    let now_iso = crate::presentation::now_iso();
    for dot in &mut dots {
        let count = pending
            .iter()
            .filter(|approval| approval.dot_id == dot.id)
            .count();
        let (status, activity) = derive_status(&repository, dot, count, &now_iso);
        dot.status = status;
        dot.activity = activity;
    }
    Ok(dots)
}

/// The executable a launcher row runs.
fn command_binary(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .find(|word| !word.contains('='))
        .and_then(|word| {
            std::path::Path::new(word)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
}

/// The agent catalog family a launcher row belongs to (`claude`, `codex`, …).
fn catalog_family(row: &Value) -> String {
    let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default();
    let agent_id = text("agentId");
    if !agent_id.starts_with("custom-") {
        return agent_id.to_string();
    }
    let icon = text("icon");
    let mut config = Map::new();
    if !icon.is_empty() {
        config.insert("icon".to_string(), json!(icon));
    }
    crate::agents::resume_agent_family_id(Some(agent_id.to_string()), &config, &Map::new())
        .unwrap_or_else(|| agent_id.to_string())
}

/// The agents a dot can run on: the launchers Ghostex shows whose CLI is installed, each with
/// its catalog models.
fn agent_choices(state: &AppState) -> (Vec<Value>, String) {
    let projects = open_gxserver_database(&state.paths)
        .ok()
        .and_then(|db| {
            DomainRepository::new(&db, state.metadata.server_id.as_str())
                .list_projects()
                .ok()
        })
        .unwrap_or_default();
    let rows = crate::sidebar_hud::sidebar_agent_buttons_from_projects(&projects)
        .as_array()
        .cloned()
        .unwrap_or_default();
    let catalog = crate::agent_model_catalog::current();
    let choice = |row: &Value| {
        let family = catalog_family(row);
        let models: Vec<Value> = catalog
            .pointer(&format!("/agents/{family}/models"))
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| {
                        let id = model.get("value").and_then(Value::as_str)?;
                        let label = model.get("label").and_then(Value::as_str).unwrap_or(id);
                        Some(json!({ "id": id, "label": label }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "id": row.get("agentId").cloned().unwrap_or(Value::Null),
            "name": row.get("name").cloned().unwrap_or(Value::Null),
            "models": models,
        })
    };
    let installed: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            row.get("command")
                .and_then(Value::as_str)
                .and_then(command_binary)
                .is_some_and(|binary| crate::platform::live_path::find(&binary, &[]).is_some())
        })
        .collect();
    let listed: Vec<&Value> = if installed.is_empty() {
        rows.iter().collect()
    } else {
        installed
    };
    let agents: Vec<Value> = listed.into_iter().map(choice).collect();
    let default_agent = agents
        .iter()
        .find(|agent| agent["id"] == "claude")
        .or_else(|| agents.first())
        .and_then(|agent| agent["id"].as_str())
        .unwrap_or("claude")
        .to_string();
    (agents, default_agent)
}

/// `snapshot.settings`.
pub(crate) fn settings_info(state: &AppState) -> Value {
    let paths: &GxserverPaths = &state.paths;
    let (agents, default_agent) = agent_choices(state);
    json!({
        "composio": {
            "signedIn": composio::signed_in(paths),
            "apps": composio::apps(paths),
        },
        "triggersKey": composio::triggers_key_source(paths),
        "vault": vault::vault_backend(paths),
        "chrome": browser::chrome_path().map(|path| path.to_string_lossy().to_string()),
        "agents": agents,
        "defaultAgentId": default_agent,
    })
}

/// The snapshot while OpenDots is off: nothing but the switch.
pub(crate) fn disabled_snapshot(state: &AppState) -> Value {
    json!({
        "enabled": false,
        "dots": [], "rules": [], "memories": [], "skills": [], "routines": [], "triggers": [],
        "passwords": [], "approvals": [], "updates": [], "handoffs": [], "teams": [],
        "settings": settings_info(state),
    })
}

pub(crate) fn snapshot(state: &AppState) -> Result<Value, DomainStateError> {
    let db = store::open(&state.paths)?;
    let list = |values: Vec<Value>| Value::Array(values);
    Ok(json!({
        "enabled": true,
        "dots": list(dots_with_status(state, &db)?.iter().map(to_value).collect()),
        "rules": list(store::list_rules(&db)?.iter().map(to_value).collect()),
        "memories": list(store::list_memories(&db, None)?.iter().map(to_value).collect()),
        "skills": list(store::list_skills(&db, None)?.iter().map(to_value).collect()),
        "routines": list(store::list_routines(&db, None)?.iter().map(to_value).collect()),
        "triggers": list(store::list_triggers(&db, None)?.iter().map(to_value).collect()),
        "passwords": list(store::list_passwords(&db)?.iter().map(to_value).collect()),
        "approvals": list(store::list_approvals_for_snapshot(&db)?.iter().map(to_value).collect()),
        "updates": list(store::list_updates(&db, 100)?.iter().map(to_value).collect()),
        "handoffs": list(store::list_handoffs(&db, 50)?.iter().map(to_value).collect()),
        "teams": list(store::list_teams(&db)?.iter().map(to_value).collect()),
        "settings": settings_info(state),
    }))
}

/// `listFiles`: the dot's folder to depth 3, at most 500 entries, dotfiles skipped.
pub(crate) fn list_files(dot: &Dot) -> Value {
    fn walk(root: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<Value>) {
        if depth > 3 || out.len() >= 500 {
            return;
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if out.len() >= 500 {
                return;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map(|relative| relative.to_string_lossy().to_string())
                .unwrap_or_default();
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0);
            out.push(json!({
                "path": relative,
                "size": if metadata.is_dir() { 0 } else { metadata.len() },
                "isDir": metadata.is_dir(),
                "modifiedAt": modified,
            }));
            if metadata.is_dir() {
                walk(root, &path, depth + 1, out);
            }
        }
    }
    let root = std::path::Path::new(&dot.folder);
    let mut files = Vec::new();
    walk(root, root, 1, &mut files);
    json!({ "files": files })
}
