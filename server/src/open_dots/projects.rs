//! A dot's home: its folder `<app data>/dots/<slug>` and the ordinary gxserver project flagged
//! `launchSettings { isDot, dotId }` that holds its conversations, made like a Hermes bot's project.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::model::{new_id, normalize_look, now_ms, Dot};
use super::{approvals, browser, instructions, publish, store, triggers};
use crate::coordinators::{classify_thread_session, ThreadProgress, ThreadState};
use crate::domain::{DomainRepository, DomainStateError};
use crate::server::AppState;
use crate::storage::open_gxserver_database;

const NAME_MAX_CHARS: usize = 40;

fn gx_error(error: impl std::fmt::Display) -> DomainStateError {
    DomainStateError {
        code: "internalError",
        message: format!("SQLite gxserver state error: {error}"),
    }
}

/// `[a-z0-9-]`, from the name.
pub(crate) fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').chars().take(48).collect::<String>();
    if slug.is_empty() {
        "dot".to_string()
    } else {
        slug
    }
}

fn dots_root(state: &AppState) -> PathBuf {
    state.paths.app_data_dir.join("dots")
}

fn folder_error(folder: &std::path::Path, error: std::io::Error) -> DomainStateError {
    DomainStateError {
        code: "internalError",
        message: format!(
            "Could not make the dot's folder {}: {error}",
            folder.display()
        ),
    }
}

/// Claims the first `<slug>`, `<slug>-2`, … that no dot or project uses, by creating its folder:
/// `create_dir` fails when the folder exists, so two creates at once never get the same one.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The folder used to be checked first and made later, so two quick creates of the same name both passed the check and shared one folder; claiming it with an atomic `create_dir`, plus unique indexes on the slug and the name in the OpenDots file, makes the second create pick the next folder or fail with the name message.
fn claim_folder(
    state: &AppState,
    db: &rusqlite::Connection,
    projects: &[Value],
    base: &str,
) -> Result<(String, PathBuf), DomainStateError> {
    let dots = store::list_dots(db)?;
    let root = dots_root(state);
    std::fs::create_dir_all(&root).map_err(|error| folder_error(&root, error))?;
    for attempt in 1..1000 {
        let slug = if attempt == 1 {
            base.to_string()
        } else {
            format!("{base}-{attempt}")
        };
        let folder = root.join(&slug);
        let folder_text = folder.to_string_lossy().to_string();
        let taken = dots
            .iter()
            .any(|dot| dot.slug == slug || dot.folder == folder_text)
            || projects.iter().any(|project| {
                project.get("path").and_then(Value::as_str) == Some(folder_text.as_str())
            });
        if taken {
            continue;
        }
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok((slug, folder)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(folder_error(&folder, error)),
        }
    }
    Err(DomainStateError::bad_request(
        "Pick a different name for the dot.",
    ))
}

fn clean_name(name: Option<&str>) -> Result<String, DomainStateError> {
    let name = name.map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Err(DomainStateError::bad_request("Give your dot a name."));
    }
    if name.chars().count() > NAME_MAX_CHARS {
        return Err(DomainStateError::bad_request(format!(
            "Keep the name under {NAME_MAX_CHARS} characters."
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(DomainStateError::bad_request(
            "The name has characters a dot can't use.",
        ));
    }
    Ok(name.to_string())
}

fn check_name_free(
    db: &rusqlite::Connection,
    name: &str,
    except: Option<&str>,
) -> Result<(), DomainStateError> {
    let taken = store::list_dots(db)?
        .into_iter()
        .any(|dot| Some(dot.id.as_str()) != except && dot.name.trim().eq_ignore_ascii_case(name));
    if taken {
        return Err(DomainStateError::bad_request(format!(
            "You already have a dot named {name}. Dots find each other by name, so pick another."
        )));
    }
    Ok(())
}

fn clean_model(value: Option<&Value>) -> Result<Option<Option<String>>, DomainStateError> {
    match value {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(model)) if model.trim().is_empty() => Ok(Some(None)),
        Some(Value::String(model)) => {
            let model = model.trim();
            if model.len() > 160
                || !model
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-._[]():/".contains(&byte))
            {
                return Err(DomainStateError::bad_request(format!(
                    "\"{model}\" is not a model name."
                )));
            }
            Ok(Some(Some(model.to_string())))
        }
        Some(_) => Err(DomainStateError::bad_request("model must be text or null.")),
    }
}

fn text_param(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|text| text.trim().to_string())
}

/// `createDot`. Blocking.
pub(crate) fn create_dot(
    state: &AppState,
    params: &Map<String, Value>,
) -> Result<Dot, DomainStateError> {
    let name = clean_name(params.get("name").and_then(Value::as_str))?;
    let db = store::open(&state.paths)?;
    check_name_free(&db, &name, None)?;
    let agent_id = text_param(params, "agentId")
        .filter(|agent| !agent.is_empty())
        .unwrap_or_else(|| {
            super::snapshot::settings_info(state)
                .get("defaultAgentId")
                .and_then(Value::as_str)
                .unwrap_or("claude")
                .to_string()
        });
    let model = clean_model(params.get("model"))?.flatten();
    let gx_db = open_gxserver_database(&state.paths).map_err(gx_error)?;
    let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
    let projects = repository.list_projects()?;
    let (slug, folder) = claim_folder(state, &db, &projects, &slugify(&name))?;
    let now = now_ms();
    let mut dot = Dot {
        id: new_id("dot"),
        name: name.clone(),
        purpose: text_param(params, "purpose").unwrap_or_default(),
        instructions: text_param(params, "instructions").unwrap_or_default(),
        look: normalize_look(params.get("look")),
        agent_id,
        model,
        paused: false,
        status: "idle".to_string(),
        activity: None,
        project_id: String::new(),
        folder: folder.to_string_lossy().to_string(),
        created_at: now,
        updated_at: now,
        slug,
    };
    if let Err(error) = store::insert_dot(&db, &dot) {
        // Another create took the name between the check and here: give the folder back.
        let _ = std::fs::remove_dir(&folder);
        return Err(error);
    }
    let _ = instructions::write_dot_files(&state.paths, &db, &dot);
    let project_params = json!({
        "path": dot.folder,
        "name": dot.name,
        "launchSettings": { "isDot": true, "dotId": dot.id },
        // None of the Dev, Build, Test and Setup placeholder Actions a repo gets.
        "deletedDefaultCommandIds": crate::sidebar_hud::default_sidebar_command_ids().collect::<Vec<_>>(),
    });
    let project =
        match repository.add_project_path(project_params.as_object().unwrap_or(&Map::new())) {
            Ok(project) => project,
            Err(error) => {
                let _ = store::delete_dot_records(&db, &dot.id);
                return Err(error);
            }
        };
    dot.project_id = project
        .get("projectId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    store::save_dot(&db, &dot)?;
    crate::external_sessions::rescan_on_next_discovery(&state.paths);
    // The row's dot facts are in place before the project is announced.
    publish::reload_facts(&state.paths);
    crate::server::schedule_presentation_project_delta(
        state,
        &gx_db,
        &repository,
        &dot.project_id,
        "projectAdded",
    )?;
    instructions::refresh_all(&state.paths);
    publish::broadcast_changed(state);
    Ok(dot)
}

/// `updateDot`. Blocking.
pub(crate) fn update_dot(
    state: &AppState,
    params: &Map<String, Value>,
) -> Result<Dot, DomainStateError> {
    let dot_id = text_param(params, "dotId").unwrap_or_default();
    let db = store::open(&state.paths)?;
    let mut dot = store::require_dot(&db, &dot_id)?;
    let mut renamed = false;
    let mut everyone_reads_it = false;
    if params.contains_key("name") {
        let name = clean_name(params.get("name").and_then(Value::as_str))?;
        if name != dot.name {
            check_name_free(&db, &name, Some(&dot.id))?;
            dot.name = name;
            renamed = true;
            everyone_reads_it = true;
        }
    }
    if let Some(purpose) = text_param(params, "purpose") {
        everyone_reads_it |= purpose != dot.purpose;
        dot.purpose = purpose;
    }
    if let Some(instructions) = text_param(params, "instructions") {
        dot.instructions = instructions;
    }
    if params.contains_key("look") {
        dot.look = normalize_look(params.get("look"));
    }
    if let Some(agent_id) = text_param(params, "agentId").filter(|agent| !agent.is_empty()) {
        if agent_id != dot.agent_id {
            // A model belongs to one agent: the old agent's model would fail to launch the new one.
            dot.model = None;
        }
        dot.agent_id = agent_id;
    }
    if let Some(model) = clean_model(params.get("model"))? {
        dot.model = model;
    }
    store::save_dot(&db, &dot)?;
    if renamed && !dot.project_id.is_empty() {
        let gx_db = open_gxserver_database(&state.paths).map_err(gx_error)?;
        let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
        let mut update = Map::new();
        update.insert("projectId".to_string(), json!(dot.project_id));
        update.insert("name".to_string(), json!(dot.name));
        if repository.update_project(&update).is_ok() {
            let _ = crate::server::schedule_presentation_project_delta(
                state,
                &gx_db,
                &repository,
                &dot.project_id,
                "projectUpdated",
            );
        }
    }
    if everyone_reads_it {
        instructions::refresh_all(&state.paths);
    } else {
        let _ = instructions::write_dot_files(&state.paths, &db, &dot);
    }
    publish::publish_changes(state);
    Ok(dot)
}

/// The dot's open conversations with what each is doing.
pub(crate) fn dot_sessions(state: &AppState, dot: &Dot) -> Vec<(String, String, ThreadState)> {
    let Ok(gx_db) = open_gxserver_database(&state.paths) else {
        return Vec::new();
    };
    let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
    let now_iso = crate::presentation::now_iso();
    repository
        .list_sessions(Some(&dot.project_id))
        .unwrap_or_default()
        .iter()
        .filter_map(|session| {
            let state_now = classify_thread_session(
                Some(session),
                ThreadProgress {
                    resolved: false,
                    has_run: true,
                },
                &now_iso,
                false,
            );
            Some((
                session.get("projectId")?.as_str()?.to_string(),
                session.get("sessionId")?.as_str()?.to_string(),
                state_now,
            ))
        })
        .collect()
}

/// Expires every card a dot is waiting on (it was stopped, paused or deleted). Blocking.
fn expire_dot_approvals(state: &AppState, dot_id: &str) {
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    for approval in store::pending_approvals(&db).unwrap_or_default() {
        if approval.dot_id == dot_id {
            approvals::expire(state, &approval.id);
        }
    }
}

/// Interrupts every conversation of the dot that is working, the chat's Stop.
pub(crate) async fn stop_dot_work(state: &Arc<AppState>, dot: &Dot) {
    let sessions = {
        let state = state.clone();
        let dot = dot.clone();
        tokio::task::spawn_blocking(move || {
            expire_dot_approvals(&state, &dot.id);
            dot_sessions(&state, &dot)
        })
        .await
        .unwrap_or_default()
    };
    for (project_id, session_id, state_now) in sessions {
        if state_now != ThreadState::Working {
            continue;
        }
        let body = json!({ "params": { "projectId": project_id, "sessionId": session_id } });
        let _ = crate::session_chat_send::handle_interrupt_session_chat_http(
            state,
            "/api/interruptSessionChat".to_string(),
            uuid::Uuid::new_v4().to_string(),
            &body,
        )
        .await;
    }
}

/// `pauseDot` / `resumeDot`: a paused dot's routines, triggers and hand-offs are skipped, and its
/// current work stops.
pub(crate) async fn set_paused(
    state: &Arc<AppState>,
    dot_id: &str,
    paused: bool,
) -> Result<Dot, DomainStateError> {
    let dot = {
        let state = state.clone();
        let dot_id = dot_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Dot, DomainStateError> {
            let db = store::open(&state.paths)?;
            let mut dot = store::require_dot(&db, &dot_id)?;
            dot.paused = paused;
            store::save_dot(&db, &dot)?;
            Ok(dot)
        })
        .await
        .map_err(|_| DomainStateError::bad_request("The dot could not be changed."))??
    };
    if paused {
        stop_dot_work(state, &dot).await;
    }
    let publish_state = state.clone();
    let _ = tokio::task::spawn_blocking(move || publish::publish_changes(&publish_state)).await;
    Ok(dot)
}

/// `deleteDot`: its records, triggers and browser profile go; its conversations are closed and its
/// project removed; its folder stays.
pub(crate) async fn delete_dot(
    state: &Arc<AppState>,
    dot_id: &str,
) -> Result<(), DomainStateError> {
    let (dot, sessions) = {
        let state = state.clone();
        let dot_id = dot_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<_, DomainStateError> {
            let db = store::open(&state.paths)?;
            let dot = store::require_dot(&db, &dot_id)?;
            let sessions = dot_sessions(&state, &dot);
            Ok((dot, sessions))
        })
        .await
        .map_err(|_| DomainStateError::bad_request("The dot could not be read."))??
    };
    for (project_id, session_id, state_now) in sessions {
        if matches!(state_now, ThreadState::Closed | ThreadState::Done) {
            continue;
        }
        let body = json!({ "params": { "projectId": project_id, "sessionId": session_id } });
        let _ = crate::server::handle_zmx_lifecycle_http(
            state,
            "/api/killSession".to_string(),
            uuid::Uuid::new_v4().to_string(),
            &body,
        )
        .await;
    }
    let state = state.clone();
    tokio::task::spawn_blocking(move || -> Result<(), DomainStateError> {
        expire_dot_approvals(&state, &dot.id);
        triggers::delete_for_dot(&state, &dot.id);
        browser::set_taken_over(&dot.id, false);
        let _ = browser::reset(&state.paths, &dot.id);
        let db = store::open(&state.paths)?;
        store::delete_dot_records(&db, &dot.id)?;
        if !dot.project_id.is_empty() {
            let gx_db = open_gxserver_database(&state.paths).map_err(gx_error)?;
            let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
            if repository.get_project(&dot.project_id)?.is_some() {
                repository.remove_project(&dot.project_id)?;
                crate::server::schedule_presentation_project_delta(
                    &state,
                    &gx_db,
                    &repository,
                    &dot.project_id,
                    "projectUpdated",
                )?;
            }
        }
        instructions::refresh_all(&state.paths);
        publish::publish_changes(&state);
        Ok(())
    })
    .await
    .map_err(|_| DomainStateError::bad_request("The dot could not be deleted."))?
}
