//! Dot-to-dot hand-offs (Open Dot's `message_dot`) and team chats (Open Dot's channels).
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! A hand-off does not block the asking agent: an agent CLI turn can take minutes and an MCP call would time out, so the message starts (or continues) a "From <dot>" conversation with the other dot and the supervisor below delivers that dot's reply back into the asking conversation when its turn ends, the way coordinators hear from their threads.
//! SEE-ALSO: server/src/server/coordinator_runtime.rs (the same finished-turn rule), docs/2026-10-01/opendots/DECISIONS.md D9 and D10.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use super::conversations::{continue_or_start, send_to_session, start_conversation};
use super::model::{new_id, now_ms, Dot, Handoff, Team};
use super::{publish, store, updates};
use crate::coordinators::{classify_thread_session, ThreadProgress, ThreadState};
use crate::domain::{DomainRepository, DomainStateError};
use crate::server::AppState;
use crate::storage::open_gxserver_database;

/// A conversation must stay out of work this long before its turn counts as finished: hooks and
/// title observation can dip between tool calls.
const FINISH_STABILITY_MS: i64 = 4_000;
const REPLY_MAX_CHARS: usize = 12_000;

/// The other dot's conversation for hand-offs from one asking conversation (`from:<dotId>:<session>`),
/// or from a dot with no conversation (`from:<dotId>`).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Keyed by the asking session, not only the asking dot: when two of A's chats asked B at once they shared one B conversation, both picked up B's first reply and the second answer was lost.
fn handoff_reference(from: &Dot, from_session_id: Option<&str>) -> String {
    match from_session_id {
        Some(session_id) => format!("from:{}:{session_id}", from.id),
        None => format!("from:{}", from.id),
    }
}

/// Refuses hand-offs that would loop: answering the dot that asked, or a third level of nesting.
fn nesting_refusal(
    db: &rusqlite::Connection,
    from: &Dot,
    to: &Dot,
    from_session: Option<&str>,
) -> Result<Option<String>, DomainStateError> {
    let Some(from_session) = from_session else {
        return Ok(None);
    };
    let open = store::open_handoffs(db)?;
    let serving = open
        .iter()
        .find(|handoff| handoff.to_session_id.as_deref() == Some(from_session));
    let Some(serving) = serving else {
        return Ok(None);
    };
    if serving.from_dot_id == to.id {
        return Ok(Some(format!(
            "You're answering {} in this conversation: just reply here, and your final message goes back to them.",
            to.name
        )));
    }
    let nested = serving.from_session_id.as_deref().is_some_and(|asker| {
        open.iter()
            .any(|handoff| handoff.to_session_id.as_deref() == Some(asker))
    });
    if nested {
        return Ok(Some(format!(
            "Too many nested hand-offs; do this part yourself instead of asking {}.",
            to.name
        )));
    }
    let _ = from;
    Ok(None)
}

/// `message_dot`: sends the message to the other dot and returns at once.
pub(crate) async fn send_handoff(
    state: &Arc<AppState>,
    from: &Dot,
    from_project_id: Option<&str>,
    from_session_id: Option<&str>,
    to: &Dot,
    message: &str,
) -> Result<String, String> {
    let refusal = {
        let paths = state.paths.clone();
        let (from_dot, to_dot) = (from.clone(), to.clone());
        let from_session = from_session_id.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            store::open(&paths)
                .and_then(|db| nesting_refusal(&db, &from_dot, &to_dot, from_session.as_deref()))
        })
        .await
        .map_err(|_| "The hand-off could not be checked.".to_string())?
        .map_err(|error| error.message)?
    };
    if let Some(refusal) = refusal {
        return Ok(refusal);
    }
    let text = format!("[From {}] {}", from.name, message.trim());
    let (to_project_id, to_session_id) = continue_or_start(
        state,
        to,
        &handoff_reference(from, from_session_id),
        &format!("From {}", from.name),
        &text,
    )
    .await
    .map_err(|error| error.message)?;
    let handoff = Handoff {
        id: new_id("hof"),
        from_dot_id: from.id.clone(),
        to_dot_id: to.id.clone(),
        message: message.trim().to_string(),
        status: "sent".to_string(),
        reply: None,
        created_at: now_ms(),
        replied_at: None,
        from_project_id: from_project_id.map(str::to_string),
        from_session_id: from_session_id.map(str::to_string),
        to_project_id: Some(to_project_id),
        to_session_id: Some(to_session_id),
    };
    let state_for_store = state.clone();
    tokio::task::spawn_blocking(move || {
        let stored =
            store::open(&state_for_store.paths).and_then(|db| store::insert_handoff(&db, &handoff));
        publish::publish_changes(&state_for_store);
        stored
    })
    .await
    .map_err(|_| "The hand-off could not be saved.".to_string())?
    .map_err(|error| error.message)?;
    Ok(if from_session_id.is_some() {
        format!(
            "Sent to {name}. Their reply will arrive in this conversation as a new message starting with \"[Reply from {name}]\" when they finish. Carry on with other work meanwhile, or end your turn if you need their answer first.",
            name = to.name
        )
    } else {
        format!(
            "Sent to {}. Their reply will be posted to the user's OpenDots feed when they finish.",
            to.name
        )
    })
}

// ------------------------------------------------------------------ supervisor

/// How many times a reply is offered to the asking conversation before it goes to the feed.
const DELIVERY_TRIES: u32 = 5;
/// A turn that ended without any text gets this long for its transcript to catch up.
const NO_REPLY_GRACE_MS: i64 = 30_000;
/// A hand-off the other dot never started working on is given up after this long.
const NEVER_STARTED_MS: i64 = 10 * 60_000;

#[derive(Default)]
pub(crate) struct HandoffMemory {
    not_working_since: HashMap<String, i64>,
    in_flight: HashSet<String>,
    /// Deliveries that failed: tries so far, and when the next try may run (ms).
    retries: HashMap<String, (u32, i64)>,
}

/// How the other dot's side of a hand-off ended.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Reply(String),
    /// Its conversation was closed first.
    Closed,
    /// Its turn ended without a written reply (or it has no transcript to read one from).
    NoReply,
    /// It never started working on the message.
    NeverStarted,
}

pub(crate) struct Delivery {
    handoff: Handoff,
    outcome: Outcome,
    /// An older hand-off from the same asking conversation that the same reply answers: it is
    /// recorded as replied without sending the reply a second time.
    quiet: bool,
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max).collect();
    clipped.push('…');
    clipped
}

/// One supervisor pass: what changed in each open hand-off. Blocking.
pub(crate) fn tick(state: &AppState, memory: &Mutex<HandoffMemory>) -> Vec<Delivery> {
    let Ok(db) = store::open(&state.paths) else {
        return Vec::new();
    };
    let Ok(open) = store::open_handoffs(&db) else {
        return Vec::new();
    };
    let mut memory = memory.lock().unwrap_or_else(PoisonError::into_inner);
    let open_ids: HashSet<String> = open.iter().map(|handoff| handoff.id.clone()).collect();
    memory
        .not_working_since
        .retain(|id, _| open_ids.contains(id));
    memory.retries.retain(|id, _| open_ids.contains(id));
    if open.is_empty() {
        return Vec::new();
    }
    let Ok(gx_db) = open_gxserver_database(&state.paths) else {
        return Vec::new();
    };
    let repository = DomainRepository::new(&gx_db, state.metadata.server_id.as_str());
    let now = now_ms();
    let now_iso = crate::presentation::now_iso();
    let mut deliveries: Vec<Delivery> = Vec::new();
    for mut handoff in open {
        if memory.in_flight.contains(&handoff.id) {
            continue;
        }
        if memory
            .retries
            .get(&handoff.id)
            .is_some_and(|(_, next_at)| now < *next_at)
        {
            continue;
        }
        let session = match (&handoff.to_project_id, &handoff.to_session_id) {
            (Some(project_id), Some(session_id)) => repository
                .get_session(project_id, session_id)
                .ok()
                .flatten(),
            _ => None,
        };
        let state_now = classify_thread_session(
            session.as_ref(),
            ThreadProgress {
                resolved: false,
                has_run: true,
            },
            &now_iso,
            false,
        );
        match state_now {
            ThreadState::Closed => {
                memory.in_flight.insert(handoff.id.clone());
                deliveries.push(Delivery {
                    handoff,
                    outcome: Outcome::Closed,
                    quiet: false,
                });
            }
            ThreadState::Working => {
                memory.not_working_since.remove(&handoff.id);
                if handoff.status == "sent" {
                    handoff.status = "working".to_string();
                    let _ = store::save_handoff(&db, &handoff);
                    publish::broadcast_changed(state);
                }
            }
            ThreadState::Finished | ThreadState::Sleeping => {
                let since = *memory
                    .not_working_since
                    .entry(handoff.id.clone())
                    .or_insert(now);
                if now - since < FINISH_STABILITY_MS {
                    continue;
                }
                let Some(session) = session.as_ref() else {
                    continue;
                };
                let observed = handoff.status == "working";
                let reply = crate::notification_feed::body::last_assistant_message(session).filter(
                    |(_, timestamp)| match timestamp {
                        Some(at) => *at > handoff.created_at,
                        // Without a timestamp only a turn we saw run is this hand-off's answer.
                        None => observed,
                    },
                );
                let outcome = match reply {
                    Some((text, _)) => Outcome::Reply(text),
                    // It worked and stopped, but wrote nothing we can read (no text in its last
                    // turn, or no transcript): finish instead of waiting forever.
                    None if observed && now - since >= NO_REPLY_GRACE_MS => Outcome::NoReply,
                    None if !observed && now - handoff.created_at >= NEVER_STARTED_MS => {
                        Outcome::NeverStarted
                    }
                    // Its first message has not been answered yet (still starting).
                    None => continue,
                };
                memory.in_flight.insert(handoff.id.clone());
                deliveries.push(Delivery {
                    handoff,
                    outcome,
                    quiet: false,
                });
            }
            ThreadState::Waiting | ThreadState::Done => {
                memory.not_working_since.remove(&handoff.id);
            }
        }
    }
    // One asking conversation that asked the same dot twice before it answered shares that dot's
    // conversation, so one reply answers both: only the newest hand-off sends it.
    let mut newest: HashMap<(Option<String>, Option<String>), (i64, usize)> = HashMap::new();
    for (index, delivery) in deliveries.iter().enumerate() {
        if !matches!(delivery.outcome, Outcome::Reply(_)) {
            continue;
        }
        let key = (
            delivery.handoff.from_session_id.clone(),
            delivery.handoff.to_session_id.clone(),
        );
        let entry = newest
            .entry(key)
            .or_insert((delivery.handoff.created_at, index));
        if delivery.handoff.created_at > entry.0 {
            *entry = (delivery.handoff.created_at, index);
        }
    }
    for (index, delivery) in deliveries.iter_mut().enumerate() {
        if !matches!(delivery.outcome, Outcome::Reply(_)) {
            continue;
        }
        let key = (
            delivery.handoff.from_session_id.clone(),
            delivery.handoff.to_session_id.clone(),
        );
        delivery.quiet = newest.get(&key).is_some_and(|(_, keep)| *keep != index);
    }
    deliveries
}

/// Posts a hand-off's reply to the asking dot's feed (nobody waits in a conversation, or the
/// conversation kept refusing it). Blocking.
fn post_to_feed(state: &AppState, from: &Dot, to: &Dot, text: &str) -> bool {
    updates::post_update(
        state,
        from,
        &from.project_id,
        None,
        Some(&format!("Reply from {}", to.name)),
        text,
        Vec::new(),
    )
    .is_ok()
}

/// Sends the reply (or the failure) back to the asking conversation. A delivery that fails is
/// tried again with a growing pause; after `DELIVERY_TRIES` the reply goes to the feed instead.
pub(crate) async fn deliver(
    state: Arc<AppState>,
    memory: Arc<Mutex<HandoffMemory>>,
    delivery: Delivery,
) {
    let Delivery {
        mut handoff,
        outcome,
        quiet,
    } = delivery;
    let done = |memory: &Mutex<HandoffMemory>, id: &str| {
        let mut memory = memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.in_flight.remove(id);
        memory.retries.remove(id);
    };
    let names = {
        let paths = state.paths.clone();
        let (from_id, to_id) = (handoff.from_dot_id.clone(), handoff.to_dot_id.clone());
        tokio::task::spawn_blocking(move || {
            let db = store::open(&paths).ok()?;
            Some((
                store::get_dot(&db, &from_id).ok().flatten()?,
                store::get_dot(&db, &to_id).ok().flatten()?,
            ))
        })
        .await
        .ok()
        .flatten()
    };
    let Some((from, to)) = names else {
        // One of the dots was deleted; nothing to deliver to.
        handoff.status = "failed".to_string();
        settle(&state, &handoff).await;
        done(&memory, &handoff.id);
        return;
    };
    let reply = match &outcome {
        Outcome::Reply(text) => Some(clip(text.trim(), REPLY_MAX_CHARS)),
        _ => None,
    };
    let finish = |handoff: &mut Handoff| {
        handoff.status = if reply.is_some() { "replied" } else { "failed" }.to_string();
        handoff.reply = reply.clone();
        handoff.replied_at = Some(now_ms());
    };
    if quiet {
        finish(&mut handoff);
        settle(&state, &handoff).await;
        done(&memory, &handoff.id);
        return;
    }
    let body = match &outcome {
        Outcome::Reply(_) => format!(
            "[Reply from {}] {}",
            to.name,
            reply.as_deref().unwrap_or_default()
        ),
        Outcome::Closed => format!(
            "[Reply from {}] (Their conversation was closed before they replied.)",
            to.name
        ),
        Outcome::NoReply => format!(
            "[Reply from {}] (They finished without writing a reply. Their conversation with you shows what they did.)",
            to.name
        ),
        Outcome::NeverStarted => format!(
            "[Reply from {}] (They did not start on it. Do it yourself, or ask them again later.)",
            to.name
        ),
    };
    let feed_text = reply.clone().unwrap_or_else(|| {
        body.split_once("] ")
            .map(|(_, rest)| rest)
            .unwrap_or_default()
            .to_string()
    });
    let asker_open = match (&handoff.from_project_id, &handoff.from_session_id) {
        (Some(project_id), Some(session_id)) => {
            let state = state.clone();
            let (project_id, session_id) = (project_id.clone(), session_id.clone());
            tokio::task::spawn_blocking(move || {
                super::conversations::session_is_open(&state, &project_id, &session_id)
            })
            .await
            .unwrap_or(false)
        }
        _ => false,
    };
    let to_feed = |state: &Arc<AppState>| {
        let state = state.clone();
        let (from, to, text) = (from.clone(), to.clone(), feed_text.clone());
        async move {
            tokio::task::spawn_blocking(move || post_to_feed(&state, &from, &to, &text))
                .await
                .unwrap_or(false)
        }
    };
    let delivered = if asker_open {
        send_to_session(
            &state,
            handoff.from_project_id.as_deref().unwrap_or_default(),
            handoff.from_session_id.as_deref().unwrap_or_default(),
            &body,
        )
        .await
        .is_ok()
    } else {
        // Nobody is waiting in a conversation: the reply goes to the feed instead.
        to_feed(&state).await
    };
    if delivered {
        finish(&mut handoff);
        settle(&state, &handoff).await;
        done(&memory, &handoff.id);
        return;
    }
    let tries = {
        let mut memory = memory.lock().unwrap_or_else(PoisonError::into_inner);
        memory.in_flight.remove(&handoff.id);
        let entry = memory.retries.entry(handoff.id.clone()).or_insert((0, 0));
        entry.0 += 1;
        // 4s, 8s, 16s, 32s between tries.
        entry.1 = now_ms() + 2_000 * (1_i64 << entry.0.min(5));
        entry.0
    };
    if tries >= DELIVERY_TRIES {
        // The asking conversation keeps refusing it: the reply still reaches the user, in the
        // feed, and the hand-off stops retrying.
        if asker_open {
            let _ = to_feed(&state).await;
        }
        finish(&mut handoff);
        settle(&state, &handoff).await;
        done(&memory, &handoff.id);
    }
}

async fn settle(state: &Arc<AppState>, handoff: &Handoff) {
    let state = state.clone();
    let handoff = handoff.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(db) = store::open(&state.paths) {
            let _ = store::save_handoff(&db, &handoff);
        }
        publish::publish_changes(&state);
    })
    .await;
}

// ------------------------------------------------------------------ teams

fn team_brief(team_name: &str, lead: &Dot, members: &[Dot], message: &str) -> String {
    let roster = members
        .iter()
        .filter(|member| member.id != lead.id)
        .map(|member| {
            if member.purpose.trim().is_empty() {
                member.name.clone()
            } else {
                format!("{} ({})", member.name, member.purpose.trim())
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "[Team chat #{team_name}] You lead this team. Members: {}. Hand focused subtasks to members with message_dot; their replies arrive in this conversation as messages that start with \"[Reply from <name>]\". When you have what you need, give the user the combined result.\n\nThe user's request:\n{}",
        if roster.is_empty() { "just you".to_string() } else { roster },
        message.trim()
    )
}

/// `startTeamChat`.
pub(crate) async fn start_team_chat(
    state: &Arc<AppState>,
    name: &str,
    lead_id: &str,
    member_ids: &[String],
    message: &str,
) -> Result<Value, DomainStateError> {
    let name = name.trim().trim_start_matches('#').trim().to_string();
    if name.is_empty() {
        return Err(DomainStateError::bad_request("Name the team chat."));
    }
    if message.trim().is_empty() {
        return Err(DomainStateError::bad_request(
            "Say what the team should do.",
        ));
    }
    let (lead, members) = {
        let paths = state.paths.clone();
        let lead_id = lead_id.to_string();
        let member_ids = member_ids.to_vec();
        tokio::task::spawn_blocking(move || -> Result<(Dot, Vec<Dot>), DomainStateError> {
            let db = store::open(&paths)?;
            let lead = store::require_dot(&db, &lead_id)?;
            let mut members = Vec::new();
            for id in member_ids {
                if id == lead.id {
                    continue;
                }
                members.push(store::require_dot(&db, &id)?);
            }
            Ok((lead, members))
        })
        .await
        .map_err(|_| DomainStateError::bad_request("The team could not be read."))??
    };
    if lead.paused {
        return Err(DomainStateError::bad_request(format!(
            "{} is paused. Resume it to lead a team chat.",
            lead.name
        )));
    }
    let brief = team_brief(&name, &lead, &members, message);
    let (project_id, session_id) =
        start_conversation(state, &lead, Some(&format!("Team · {name}")), Some(&brief)).await?;
    let team = Team {
        id: new_id("team"),
        name,
        lead_id: lead.id.clone(),
        member_ids: std::iter::once(lead.id.clone())
            .chain(members.iter().map(|member| member.id.clone()))
            .collect(),
        project_id: project_id.clone(),
        session_id: session_id.clone(),
        created_at: now_ms(),
    };
    let stored = {
        let state = state.clone();
        let team = team.clone();
        tokio::task::spawn_blocking(move || -> Result<(), DomainStateError> {
            let db = store::open(&state.paths)?;
            store::insert_team(&db, &team)?;
            store::set_conversation(
                &db,
                &team.lead_id,
                &format!("team:{}", team.id),
                &team.project_id,
                &team.session_id,
            )?;
            publish::publish_changes(&state);
            Ok(())
        })
        .await
        .map_err(|_| DomainStateError::bad_request("The team could not be saved."))?
    };
    stored?;
    Ok(serde_json::json!({
        "team": super::model::to_value(&team),
        "projectId": project_id,
        "sessionId": session_id,
    }))
}
