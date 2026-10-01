//! Telling clients about changes: the payload-less `openDotsChanged` event (clients refetch
//! `snapshot`), and the dot facts every dot project row carries (CONTRACT §3), republished as a
//! `projectUpdated` presentation delta when they change.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use serde_json::{json, Map, Value};

use super::{store, OPEN_DOTS_CHANGED_EVENT_TYPE};
use crate::constants::GXSERVER_PROTOCOL_VERSION;
use crate::domain::DomainRepository;
use crate::paths::GxserverPaths;
use crate::server::AppState;
use crate::storage::open_gxserver_database;

/// What a dot project's presentation row shows about its dot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DotRowFacts {
    pub(crate) project_id: String,
    pub(crate) name: String,
    pub(crate) look: Value,
    pub(crate) agent_id: String,
    pub(crate) paused: bool,
    pub(crate) needs_you: u64,
}

static FACTS: Mutex<Option<BTreeMap<String, DotRowFacts>>> = Mutex::new(None);
static PATHS: OnceLock<GxserverPaths> = OnceLock::new();

/// Remembers where OpenDots' file is, for a projection that runs before anything else read it.
pub(crate) fn remember_paths(paths: &GxserverPaths) {
    let _ = PATHS.set(paths.clone());
}

fn read_facts(paths: &GxserverPaths) -> BTreeMap<String, DotRowFacts> {
    let Ok(db) = store::open(paths) else {
        return BTreeMap::new();
    };
    let pending = store::pending_approvals(&db).unwrap_or_default();
    store::list_dots(&db)
        .unwrap_or_default()
        .into_iter()
        .map(|dot| {
            let needs_you = pending
                .iter()
                .filter(|approval| approval.dot_id == dot.id)
                .count() as u64;
            (
                dot.id.clone(),
                DotRowFacts {
                    project_id: dot.project_id,
                    name: dot.name,
                    look: dot.look,
                    agent_id: dot.agent_id,
                    paused: dot.paused,
                    needs_you,
                },
            )
        })
        .collect()
}

/// Every dot's row facts, read now (`rowFacts`, for the phone's session summary).
pub(crate) fn row_facts_value(paths: &GxserverPaths) -> Value {
    Value::Array(
        read_facts(paths)
            .into_iter()
            .map(|(dot_id, facts)| {
                json!({
                    "dotId": dot_id,
                    "projectId": facts.project_id,
                    "dotLook": facts.look,
                    "dotAgentId": facts.agent_id,
                    "dotPaused": facts.paused,
                    "dotNeedsYou": facts.needs_you,
                })
            })
            .collect(),
    )
}

/// The facts as last published; the first read loads them once.
pub(crate) fn dot_row_facts(dot_id: &str) -> Option<DotRowFacts> {
    let mut facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
    if facts.is_none() {
        let paths = PATHS.get()?;
        *facts = Some(read_facts(paths));
    }
    facts.as_ref()?.get(dot_id).cloned()
}

/// The presentation row fields of a dot project (`dotId`, `dotLook`, `dotAgentId`, `dotPaused`,
/// `dotNeedsYou`). Empty for every other project.
pub(crate) fn presentation_fields(project: &Value) -> Map<String, Value> {
    let mut fields = Map::new();
    let Some(dot_id) = super::dot_id_of_project(project) else {
        return fields;
    };
    fields.insert("dotId".to_string(), Value::String(dot_id.to_string()));
    if let Some(facts) = dot_row_facts(dot_id) {
        fields.insert("dotLook".to_string(), facts.look);
        fields.insert("dotAgentId".to_string(), Value::String(facts.agent_id));
        fields.insert("dotPaused".to_string(), Value::Bool(facts.paused));
        fields.insert("dotNeedsYou".to_string(), Value::from(facts.needs_you));
    }
    fields
}

/// Rereads the facts without announcing anything (a new dot's project is announced next).
pub(crate) fn reload_facts(paths: &GxserverPaths) {
    let fresh = read_facts(paths);
    *FACTS.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh);
}

pub(crate) fn broadcast_changed(state: &AppState) {
    state.event_hub.broadcast(json!({
        "protocolVersion": GXSERVER_PROTOCOL_VERSION,
        "serverId": state.metadata.server_id.clone(),
        "type": OPEN_DOTS_CHANGED_EVENT_TYPE,
    }));
}

/// After any change: announce it, and republish every dot project whose row facts moved.
/// Blocking (reads both databases); call it off the async runtime.
pub(crate) fn publish_changes(state: &AppState) {
    broadcast_changed(state);
    let fresh = read_facts(&state.paths);
    let changed: Vec<String> = {
        let mut facts = FACTS.lock().unwrap_or_else(PoisonError::into_inner);
        let before = facts.take().unwrap_or_default();
        let changed = fresh
            .iter()
            .filter(|(dot_id, row)| before.get(*dot_id) != Some(*row))
            .map(|(_, row)| row.project_id.clone())
            .filter(|project_id| !project_id.is_empty())
            .collect();
        *facts = Some(fresh);
        changed
    };
    if changed.is_empty() {
        return;
    }
    let Ok(db) = open_gxserver_database(&state.paths) else {
        return;
    };
    let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
    for project_id in changed {
        if repository.get_project(&project_id).ok().flatten().is_some() {
            let _ = crate::server::schedule_presentation_project_delta(
                state,
                &db,
                &repository,
                &project_id,
                "projectUpdated",
            );
        }
    }
}
