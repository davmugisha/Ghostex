//! Triggers: a Composio event (a new email, a calendar change, …) wakes a dot with an instruction
//! (Open Dot's `triggers.ts` + `runTrigger`). Each trigger keeps one conversation,
//! "Trigger · <name>", and every event is a new message in it.

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};

use super::composio::{self, ComposioTriggerEvent, TriggerEventSink};
use super::conversations::continue_or_start;
use super::model::{new_id, now_ms, Trigger};
use super::{publish, store};
use crate::domain::DomainStateError;
use crate::server::AppState;

const EVENT_DATA_MAX_CHARS: usize = 6_000;

static SINK: OnceLock<TriggerEventSink> = OnceLock::new();

/// The sink the live subscription delivers events to; built once at startup.
pub(crate) fn install_sink(state: &Arc<AppState>) -> TriggerEventSink {
    SINK.get_or_init(|| {
        let state = state.clone();
        let runtime = tokio::runtime::Handle::current();
        Arc::new(move |event: ComposioTriggerEvent| {
            let state = state.clone();
            runtime.spawn(async move { on_event(state, event).await });
        })
    })
    .clone()
}

/// (Re)starts the live subscription; idempotent. Blocking.
pub(crate) fn start_events(state: &AppState) {
    if let Some(sink) = SINK.get() {
        composio::start_trigger_events(&state.paths, sink.clone());
    }
}

fn humanize(slug: &str) -> String {
    let words: Vec<String> = slug
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let mut characters = lower.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect();
    words.join(" ")
}

/// `addTrigger`. Blocking.
pub(crate) fn add_trigger(
    state: &AppState,
    params: &Map<String, Value>,
) -> Result<Trigger, DomainStateError> {
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let (dot_id, toolkit, slug, instruction) = (
        text("dotId"),
        text("toolkit").to_ascii_lowercase(),
        text("slug"),
        text("instruction"),
    );
    if slug.is_empty() || toolkit.is_empty() {
        return Err(DomainStateError::bad_request("Pick an app and an event."));
    }
    if instruction.is_empty() {
        return Err(DomainStateError::bad_request(
            "Say what the dot should do when it happens.",
        ));
    }
    let config = params
        .get("config")
        .filter(|config| config.is_object())
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let db = store::open(&state.paths)?;
    let dot = store::require_dot(&db, &dot_id)?;
    let name = composio::trigger_types(&state.paths, &toolkit)
        .ok()
        .and_then(|types| {
            types.into_iter().find_map(|kind| {
                (kind.get("slug").and_then(Value::as_str) == Some(slug.as_str()))
                    .then(|| kind.get("name").and_then(Value::as_str).map(str::to_string))
                    .flatten()
            })
        })
        .unwrap_or_else(|| humanize(&slug));
    let composio_id = composio::create_trigger(&state.paths, &slug, &config)
        .map_err(DomainStateError::bad_request)?;
    let trigger = Trigger {
        id: new_id("trg"),
        dot_id: dot.id,
        composio_id,
        slug,
        toolkit,
        name,
        config,
        instruction,
        enabled: true,
        last_fired_at: None,
        last_error: None,
        session_id: None,
        created_at: now_ms(),
    };
    store::insert_trigger(&db, &trigger)?;
    start_events(state);
    Ok(trigger)
}

/// The other local triggers on the same Composio trigger as `trigger` (another dot added the same
/// event with the same settings, and Composio handed back the same trigger).
fn sharing(db: &rusqlite::Connection, trigger: &Trigger) -> Vec<Trigger> {
    store::triggers_by_composio_id(db, &trigger.composio_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|other| other.id != trigger.id)
        .collect()
}

/// `setTriggerEnabled`. Composio's trigger is only switched off when no other dot's trigger on it
/// is still on. Blocking.
pub(crate) fn set_enabled(
    state: &AppState,
    trigger_id: &str,
    enabled: bool,
) -> Result<Trigger, DomainStateError> {
    let db = store::open(&state.paths)?;
    let mut trigger = store::get_trigger(&db, trigger_id)?
        .ok_or_else(|| DomainStateError::not_found("That trigger no longer exists."))?;
    let still_needed = sharing(&db, &trigger).iter().any(|other| other.enabled);
    if enabled || !still_needed {
        composio::set_trigger_enabled(&state.paths, &trigger.composio_id, enabled)
            .map_err(DomainStateError::bad_request)?;
    }
    trigger.enabled = enabled;
    trigger.last_error = None;
    store::save_trigger(&db, &trigger)?;
    Ok(trigger)
}

/// `deleteTrigger`: gone on Composio's side already is fine; a Composio trigger another dot's
/// trigger still uses is kept. Blocking.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Composio returns the same trigger when two dots add the same event with the same settings, so deleting one dot's trigger (or the dot) used to delete the other dot's events too, and only the first dot ever heard them.
pub(crate) fn delete(state: &AppState, trigger_id: &str) -> Result<(), DomainStateError> {
    let db = store::open(&state.paths)?;
    if let Some(trigger) = store::get_trigger(&db, trigger_id)? {
        if sharing(&db, &trigger).is_empty() {
            let _ = composio::delete_trigger(&state.paths, &trigger.composio_id);
        }
        store::delete_trigger(&db, trigger_id)?;
    }
    Ok(())
}

/// Removes a deleted dot's triggers from Composio too, except the ones another dot still uses.
/// Blocking.
pub(crate) fn delete_for_dot(state: &AppState, dot_id: &str) {
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    for trigger in store::list_triggers(&db, Some(dot_id)).unwrap_or_default() {
        let others = sharing(&db, &trigger);
        if others.iter().all(|other| other.dot_id == dot_id) {
            let _ = composio::delete_trigger(&state.paths, &trigger.composio_id);
        }
    }
}

async fn on_event(state: Arc<AppState>, event: ComposioTriggerEvent) {
    if !super::open_dots_enabled(&state.paths) {
        return;
    }
    // Every dot whose trigger is on this Composio trigger hears the event.
    let found = {
        let paths = state.paths.clone();
        let ids = event.composio_ids.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(db) = store::open(&paths) else {
                return Vec::new();
            };
            let Some(triggers) = ids
                .iter()
                .filter(|id| !id.is_empty())
                .map(|id| store::triggers_by_composio_id(&db, id).unwrap_or_default())
                .find(|triggers| !triggers.is_empty())
            else {
                return Vec::new();
            };
            triggers
                .into_iter()
                .filter_map(|trigger| {
                    let dot = store::get_dot(&db, &trigger.dot_id).ok().flatten()?;
                    Some((trigger, dot))
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    };
    for (trigger, dot) in found {
        run_trigger(state.clone(), trigger, dot, &event).await;
    }
}

async fn run_trigger(
    state: Arc<AppState>,
    trigger: Trigger,
    dot: super::model::Dot,
    event: &ComposioTriggerEvent,
) {
    if !trigger.enabled || dot.paused {
        return;
    }
    let mut data = serde_json::to_string_pretty(&event.payload).unwrap_or_default();
    if data.chars().count() > EVENT_DATA_MAX_CHARS {
        data = data.chars().take(EVENT_DATA_MAX_CHARS).collect();
    }
    let text = format!(
        "[Trigger: {}] {}\n\nWhat happened ({} event data):\n{data}",
        trigger.name, trigger.instruction, trigger.toolkit
    );
    let ran = continue_or_start(
        &state,
        &dot,
        &format!("trigger:{}", trigger.id),
        &format!("Trigger · {}", trigger.name),
        &text,
    )
    .await;
    // Only what the run produced is written, so a trigger the user switched off or edited while
    // its conversation started stays the way they left it.
    let (session_id, error) = match ran {
        Ok((_, session_id)) => (Some(session_id), None),
        Err(error) => (None, Some(error.message)),
    };
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(db) = store::open(&state.paths) {
            let _ = store::record_trigger_fire(
                &db,
                &trigger.id,
                now_ms(),
                session_id.as_deref(),
                error.as_deref(),
            );
        }
        publish::publish_changes(&state);
    })
    .await;
}
