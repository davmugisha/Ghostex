//! Routines: recurring work a dot does on its own on a five-field cron (Open Dot's `scheduler.ts`).
//! Each routine keeps one conversation, "Routine · <name>", and every run is a new message in it,
//! so its runs read like a log; a conversation that was closed is started again.

use std::sync::Arc;

use super::conversations::continue_or_start;
use super::model::{new_id, now_ms, Routine};
use super::{cron, instructions, publish, store};
use crate::domain::DomainStateError;
use crate::paths::GxserverPaths;
use crate::server::AppState;

fn invalid_schedule(schedule: &str, reason: &str) -> DomainStateError {
    DomainStateError::bad_request(format!(
        "Invalid schedule \"{schedule}\": {reason}. Use five fields (minute hour day month weekday), for example \"0 8 * * 1-5\" for weekdays at 8:00."
    ))
}

pub(crate) fn check_schedule(schedule: &str) -> Result<(), DomainStateError> {
    cron::validate(schedule).map_err(|reason| invalid_schedule(schedule, &reason))
}

/// `addRoutine` and the `create_routine` tool. Blocking.
pub(crate) fn add_routine(
    paths: &GxserverPaths,
    dot_id: &str,
    name: &str,
    instruction: &str,
    schedule: &str,
) -> Result<Routine, DomainStateError> {
    let (name, instruction, schedule) = (name.trim(), instruction.trim(), schedule.trim());
    if name.is_empty() {
        return Err(DomainStateError::bad_request("Name the routine."));
    }
    if instruction.is_empty() {
        return Err(DomainStateError::bad_request(
            "Say what the routine should do each time.",
        ));
    }
    check_schedule(schedule)?;
    let db = store::open(paths)?;
    store::require_dot(&db, dot_id)?;
    let now = now_ms();
    let routine = Routine {
        id: new_id("rtn"),
        dot_id: dot_id.to_string(),
        name: name.to_string(),
        instruction: instruction.to_string(),
        schedule: schedule.to_string(),
        enabled: true,
        last_run_at: None,
        next_run_at: cron::next_run_ms(schedule, now),
        session_id: None,
        created_at: now,
        last_error: None,
    };
    store::insert_routine(&db, &routine)?;
    instructions::refresh_dot(paths, dot_id);
    Ok(routine)
}

/// `updateRoutine`. Blocking.
pub(crate) fn update_routine(
    paths: &GxserverPaths,
    routine_id: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<Routine, DomainStateError> {
    let db = store::open(paths)?;
    let mut routine = store::get_routine(&db, routine_id)?
        .ok_or_else(|| DomainStateError::not_found("That routine no longer exists."))?;
    let text = |key: &str| {
        params
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    if let Some(name) = text("name") {
        routine.name = name;
    }
    if let Some(instruction) = text("instruction") {
        routine.instruction = instruction;
    }
    if let Some(schedule) = text("schedule") {
        check_schedule(&schedule)?;
        routine.schedule = schedule;
    }
    if let Some(enabled) = params.get("enabled").and_then(serde_json::Value::as_bool) {
        routine.enabled = enabled;
    }
    routine.next_run_at = routine
        .enabled
        .then(|| cron::next_run_ms(&routine.schedule, now_ms()))
        .flatten();
    store::save_routine(&db, &routine)?;
    instructions::refresh_dot(paths, &routine.dot_id);
    Ok(routine)
}

/// The routines due now whose dot is not paused. Each is claimed by moving its next run forward
/// with a compare-and-set, so a slow launch is not started twice and a user's edit or pause made
/// since the read is never written over. Blocking.
pub(crate) fn take_due(paths: &GxserverPaths) -> Vec<Routine> {
    let Ok(db) = store::open(paths) else {
        return Vec::new();
    };
    let now = now_ms();
    let paused: std::collections::HashSet<String> = store::list_dots(&db)
        .unwrap_or_default()
        .into_iter()
        .filter(|dot| dot.paused)
        .map(|dot| dot.id)
        .collect();
    let mut due = Vec::new();
    for mut routine in store::list_routines(&db, None).unwrap_or_default() {
        if !routine.enabled || routine.next_run_at.is_none_or(|at| at > now) {
            continue;
        }
        let next = cron::next_run_ms(&routine.schedule, now);
        let claimed =
            store::move_routine_next_run(&db, &routine.id, routine.next_run_at, next, true)
                .unwrap_or(false);
        if !claimed {
            continue;
        }
        routine.next_run_at = next;
        // A paused dot skips its runs; the schedule moves on.
        if !paused.contains(&routine.dot_id) {
            due.push(routine);
        }
    }
    due
}

/// Restamps every routine's next run from now: runs missed while gxserver was not running (or
/// while OpenDots was off) are skipped, as a cron job's are. Blocking.
pub(crate) fn reschedule_all(paths: &GxserverPaths) {
    let Ok(db) = store::open(paths) else {
        return;
    };
    let now = now_ms();
    for routine in store::list_routines(&db, None).unwrap_or_default() {
        let next = routine
            .enabled
            .then(|| cron::next_run_ms(&routine.schedule, now))
            .flatten();
        let missed = routine.next_run_at.is_some_and(|at| at <= now);
        if routine.next_run_at.is_none() || missed || !routine.enabled {
            let _ =
                store::move_routine_next_run(&db, &routine.id, routine.next_run_at, next, false);
        }
    }
}

/// Runs a routine now: a new message in its conversation. `manual` is `runRoutineNow`, which a
/// paused dot refuses instead of skipping.
pub(crate) async fn run_routine(
    state: &Arc<AppState>,
    routine_id: &str,
    manual: bool,
) -> Result<(String, String), DomainStateError> {
    let (routine, dot) = {
        let paths = state.paths.clone();
        let routine_id = routine_id.to_string();
        tokio::task::spawn_blocking(move || {
            let db = store::open(&paths)?;
            let routine = store::get_routine(&db, &routine_id)?
                .ok_or_else(|| DomainStateError::not_found("That routine no longer exists."))?;
            let dot = store::require_dot(&db, &routine.dot_id)?;
            Ok::<_, DomainStateError>((routine, dot))
        })
        .await
        .map_err(|_| DomainStateError::bad_request("The routine could not be read."))??
    };
    if dot.paused {
        return Err(DomainStateError::bad_request(format!(
            "{} is paused. Resume it to run its routines.",
            dot.name
        )));
    }
    let _ = manual;
    let text = format!("[Routine: {}] {}", routine.name, routine.instruction);
    let ran = continue_or_start(
        state,
        &dot,
        &format!("routine:{}", routine.id),
        &format!("Routine · {}", routine.name),
        &text,
    )
    .await;
    // The run is recorded either way; a run that could not start keeps why, for the Setup tab.
    let state_for_store = state.clone();
    let routine_id = routine.id.clone();
    let stored_session = ran.as_ref().ok().map(|(_, session_id)| session_id.clone());
    let stored_error = ran.as_ref().err().map(|error| error.message.clone());
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(db) = store::open(&state_for_store.paths) {
            let _ = store::record_routine_run(
                &db,
                &routine_id,
                now_ms(),
                stored_session.as_deref(),
                stored_error.as_deref(),
            );
        }
        publish::publish_changes(&state_for_store);
    })
    .await;
    ran
}
