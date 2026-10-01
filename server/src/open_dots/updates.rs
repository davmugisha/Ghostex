//! What a dot posts to the user on its own (Open Dot's `send_update` and `share_file`): an Update
//! row for the OpenDots "Latest from your dots" feed, and a row in Ghostex's notification feed so
//! the bell shows it.

use serde_json::Value;

use super::model::{new_id, now_ms, Dot, Update, UpdateFile};
use super::{publish, store};
use crate::domain::{DomainRepository, DomainStateError};
use crate::notification_feed::store::{
    bounded_notification_body, insert_notification_feed_row, notification_agent_icon,
    NewNotificationFeedRow, NOTIFICATION_FEED_KIND_CUSTOM,
};
use crate::server::AppState;
use crate::storage::open_gxserver_database;

/// Stores the update and rings the bell. Blocking.
pub(crate) fn post_update(
    state: &AppState,
    dot: &Dot,
    project_id: &str,
    session_id: Option<&str>,
    title: Option<&str>,
    text: &str,
    files: Vec<UpdateFile>,
) -> Result<Update, DomainStateError> {
    let update = Update {
        id: new_id("upd"),
        dot_id: dot.id.clone(),
        project_id: project_id.to_string(),
        session_id: session_id.map(str::to_string),
        title: title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        text: text.to_string(),
        files,
        read: false,
        created_at: now_ms(),
    };
    let db = store::open(&state.paths)?;
    store::insert_update(&db, &update)?;
    if let Some(session_id) = session_id {
        ring_bell(state, dot, project_id, session_id, &update);
    }
    publish::publish_changes(state);
    Ok(update)
}

/// A feed row needs a live session to open, so an update from outside any conversation only shows
/// in OpenDots.
fn ring_bell(state: &AppState, dot: &Dot, project_id: &str, session_id: &str, update: &Update) {
    let Ok(db) = open_gxserver_database(&state.paths) else {
        return;
    };
    let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
    let Ok(Some(session)) = repository.get_session(project_id, session_id) else {
        return;
    };
    let title = match (&update.title, update.files.first()) {
        (Some(title), _) => title.clone(),
        (None, Some(file)) => format!("{} sent {}", dot.name, file.name),
        (None, None) => dot.name.clone(),
    };
    let inserted = insert_notification_feed_row(
        &db,
        NewNotificationFeedRow {
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
            kind: NOTIFICATION_FEED_KIND_CUSTOM,
            title,
            subtitle: dot.name.clone(),
            body: bounded_notification_body(&update.text),
            agent_name: notification_agent_icon(&session),
            attention_event_id: None,
        },
    );
    if inserted.is_ok() {
        crate::notification_feed::broadcast_notification_feed_changed(state);
    }
}

/// A file a dot shares: a path in its folder (or an absolute path), which must be a file.
pub(crate) fn shared_file(dot: &Dot, path: &str) -> Result<UpdateFile, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Give the path of the file to share.".to_string());
    }
    let candidate = std::path::Path::new(path);
    let full = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        std::path::Path::new(&dot.folder).join(candidate)
    };
    let full = std::fs::canonicalize(&full)
        .map_err(|_| format!("No such file: {path}. Write it in your folder first."))?;
    let metadata = std::fs::metadata(&full).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{path} is a folder, not a file."));
    }
    Ok(UpdateFile {
        name: full
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string()),
        path: full.to_string_lossy().to_string(),
        size: metadata.len(),
    })
}

/// `markUpdatesRead`.
pub(crate) fn mark_read(
    state: &AppState,
    params: &serde_json::Map<String, Value>,
) -> Result<(), DomainStateError> {
    let db = store::open(&state.paths)?;
    let dot_id = params
        .get("dotId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let update_ids: Option<Vec<String>> =
        params
            .get("updateIds")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            });
    store::mark_updates_read(&db, dot_id, update_ids.as_deref())?;
    publish::broadcast_changed(state);
    Ok(())
}
