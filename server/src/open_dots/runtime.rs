//! OpenDots' background work while it is on: delivering hand-off replies, running due routines,
//! listening for Composio trigger events, expiring cards nobody can answer any more, and telling
//! clients when a dot's status changes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::handoffs::{self, HandoffMemory};
use super::{
    approvals, browser, composio, instructions, publish, routines, snapshot, store, triggers,
};
use crate::domain::DomainRepository;
use crate::server::AppState;
use crate::storage::open_gxserver_database;

const TICK: Duration = Duration::from_secs(2);
/// Routines, card expiry and status changes run every fifth tick.
const SLOW_TICKS: u64 = 5;

/// What the startup pass does once: cards from before a restart can never be answered (their
/// tool calls died with the old gxserver), missed routine runs are skipped, and every dot's files
/// are rewritten so they name this build's `ghostex` CLI.
fn startup_pass(state: &AppState) {
    publish::reload_facts(&state.paths);
    // Dot Chromes an earlier gxserver left running (it crashed or was killed) are closed.
    browser::sweep_orphans(&state.paths);
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    if store::list_dots(&db)
        .map(|dots| dots.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    for approval in store::pending_approvals(&db).unwrap_or_default() {
        approvals::expire(state, &approval.id);
    }
    routines::reschedule_all(&state.paths);
    instructions::refresh_all(&state.paths);
}

/// Cards whose session closed, or whose tool call is no longer waiting in this gxserver (its MCP
/// process exited); then every card still waiting goes back into its chat if nothing shows it.
fn expire_orphaned_cards(state: &AppState) {
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    let pending = store::pending_approvals(&db).unwrap_or_default();
    if pending.is_empty() {
        return;
    }
    let Ok(gx_db) = open_gxserver_database(&state.paths) else {
        return;
    };
    let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
    let mut still_waiting = Vec::with_capacity(pending.len());
    for approval in pending {
        let session_closed = approval.session_id.as_deref().is_some_and(|session_id| {
            repository
                .get_session(&approval.project_id, session_id)
                .ok()
                .flatten()
                .is_none_or(|session| {
                    crate::presentation::effective_lifecycle_state(&session) == "stopped"
                })
        });
        if session_closed || !approvals::has_waiter(&approval.id) {
            approvals::expire(state, &approval.id);
        } else {
            still_waiting.push(approval);
        }
    }
    approvals::reshow_in_chat(state, &still_waiting);
}

type Statuses = HashMap<String, (String, Option<String>)>;

/// Announces a change when any dot's derived status moved, so open views update without polling.
fn check_statuses(state: &AppState, last: &Mutex<Statuses>) {
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    let Ok(dots) = snapshot::dots_with_status(state, &db) else {
        return;
    };
    let fresh: Statuses = dots
        .into_iter()
        .map(|dot| (dot.id, (dot.status, dot.activity)))
        .collect();
    let mut last = last
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *last != fresh {
        *last = fresh;
        publish::broadcast_changed(state);
    }
}

pub(crate) fn start(state: Arc<AppState>) {
    publish::remember_paths(&state.paths);
    triggers::install_sink(&state);
    {
        // A finished app sign-in or a new connection changes what every dot's AGENTS.md lists.
        let state = state.clone();
        composio::set_changed_listener(Arc::new(move || {
            let state = state.clone();
            std::thread::spawn(move || {
                instructions::refresh_all(&state.paths);
                publish::publish_changes(&state);
            });
        }));
    }
    let mut shutdown = state.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let startup_state = state.clone();
        let _ = tokio::task::spawn_blocking(move || startup_pass(&startup_state)).await;
        let memory = Arc::new(Mutex::new(HandoffMemory::default()));
        let statuses = Arc::new(Mutex::new(Statuses::new()));
        let mut clock = tokio::time::interval(TICK);
        let mut ticks: u64 = 0;
        let mut events_on = false;
        let mut was_enabled: Option<bool> = None;
        loop {
            tokio::select! {
                _ = shutdown.recv() => break,
                _ = clock.tick() => {}
            }
            ticks += 1;
            let paths = state.paths.clone();
            let enabled = tokio::task::spawn_blocking(move || super::open_dots_enabled(&paths))
                .await
                .unwrap_or(false);
            let turned_on = was_enabled == Some(false) && enabled;
            let turned_off = was_enabled == Some(true) && !enabled;
            was_enabled = Some(enabled);
            if !enabled {
                if events_on {
                    composio::stop_trigger_events();
                    events_on = false;
                }
                if turned_off {
                    // Nothing can drive a dot's browser while OpenDots is off.
                    let _ = tokio::task::spawn_blocking(browser::shutdown_all).await;
                }
                continue;
            }
            if turned_on {
                // Runs missed while OpenDots was off are skipped, as after a restart, instead of
                // all starting at once.
                let paths = state.paths.clone();
                let _ = tokio::task::spawn_blocking(move || routines::reschedule_all(&paths)).await;
            }
            if !events_on {
                let events_state = state.clone();
                let _ = tokio::task::spawn_blocking(move || triggers::start_events(&events_state))
                    .await;
                events_on = true;
            }
            let tick_state = state.clone();
            let tick_memory = memory.clone();
            let deliveries =
                tokio::task::spawn_blocking(move || handoffs::tick(&tick_state, &tick_memory))
                    .await
                    .unwrap_or_default();
            for delivery in deliveries {
                tokio::spawn(handoffs::deliver(state.clone(), memory.clone(), delivery));
            }
            if ticks % SLOW_TICKS != 1 {
                continue;
            }
            let slow_state = state.clone();
            let slow_statuses = statuses.clone();
            let due = tokio::task::spawn_blocking(move || {
                expire_orphaned_cards(&slow_state);
                check_statuses(&slow_state, &slow_statuses);
                routines::take_due(&slow_state.paths)
            })
            .await
            .unwrap_or_default();
            for routine in due {
                let state = state.clone();
                tokio::spawn(async move {
                    let _ = routines::run_routine(&state, &routine.id, false).await;
                });
            }
        }
        composio::stop_trigger_events();
        let _ = tokio::task::spawn_blocking(browser::shutdown_all).await;
    });
}
