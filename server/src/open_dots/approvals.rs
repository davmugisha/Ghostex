//! Approvals, questions and app-connect cards (Open Dot's cards): a tool call that needs the user
//! blocks here until it is answered from the session's chat card, the OpenDots view, the phone or
//! `ghostex dots approve`, or until 30 minutes pass.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! A pending approval is written as the session's stored chat prompt (a question card whose `toolUseId` is `dotapproval:<id>`), so the desktop and phone chats draw it with no new card type; answering it never types into the terminal, because the agent is blocked inside an MCP tool call, not on a screen prompt.
//! SEE-ALSO: server/src/session_chat_send/answer_http.rs (the early `dotapproval:` branch), docs/2026-10-01/opendots/CONTRACT.md §6.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::{Transaction, TransactionBehavior};
use serde_json::{json, Map, Value};
use tokio::sync::oneshot;

use super::model::{new_id, now_ms, Approval, Dot};
use super::{composio, instructions, publish, store};
use crate::domain::{DomainRepository, DomainStateError};
use crate::server::AppState;
use crate::session_chat_interactive::{
    parse_stored_session_chat_prompt, SessionChatInteractivePrompt, SessionChatQuestion,
    SessionChatQuestionOption,
};
use crate::storage::open_gxserver_database;

pub(crate) const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub(crate) const DOT_APPROVAL_TOOL_USE_PREFIX: &str = "dotapproval:";
const DOT_APPROVAL_TOOL_NAME: &str = "ghostex_dot_approval";
pub(crate) const APPROVE_LABEL: &str = "Approve";
pub(crate) const ALWAYS_LABEL: &str = "Always allow";
pub(crate) const DENY_LABEL: &str = "Deny";
pub(crate) const CONNECTED_LABEL: &str = "I've connected it";
pub(crate) const NOT_NOW_LABEL: &str = "Not now";

/// How a card was answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Approved,
    Denied(Option<String>),
    Answered(String),
    Expired,
    /// gxserver is shutting down: the card stays pending (the next start expires it) and the tool
    /// call returns so the shutdown is not held up.
    Restarting,
}

static WAITERS: Mutex<Option<HashMap<String, oneshot::Sender<Resolution>>>> = Mutex::new(None);
/// MCP call id → the approval it is waiting on, so a cancelled call expires its card.
static CALLS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn with_waiters<T>(f: impl FnOnce(&mut HashMap<String, oneshot::Sender<Resolution>>) -> T) -> T {
    let mut guard = WAITERS.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

fn with_calls<T>(f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
    let mut guard = CALLS.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// A card to raise.
pub(crate) struct NewApproval {
    pub(crate) kind: &'static str,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) tool: Option<String>,
    pub(crate) options: Vec<String>,
    pub(crate) rule_action: Option<String>,
    pub(crate) toolkit: Option<String>,
    pub(crate) url: Option<String>,
}

fn header_for(kind: &str) -> &'static str {
    match kind {
        "question" => "Question",
        "connect" => "Connect an app",
        _ => "Needs your approval",
    }
}

fn chat_prompt(approval: &Approval) -> SessionChatInteractivePrompt {
    let mut question = approval.title.trim().to_string();
    if let Some(detail) = approval
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
    {
        question.push_str("\n\n");
        question.push_str(detail);
    }
    if let Some(url) = approval.url.as_deref().filter(|url| !url.is_empty()) {
        question.push_str("\n\nSign in here: ");
        question.push_str(url);
    }
    let (labels, allow_custom): (Vec<String>, Option<bool>) = match approval.kind.as_str() {
        "question" => (approval.options.clone(), None),
        "connect" => (
            vec![CONNECTED_LABEL.to_string(), NOT_NOW_LABEL.to_string()],
            Some(false),
        ),
        _ => (
            vec![
                APPROVE_LABEL.to_string(),
                ALWAYS_LABEL.to_string(),
                DENY_LABEL.to_string(),
            ],
            Some(false),
        ),
    };
    SessionChatInteractivePrompt::Question {
        questions: vec![SessionChatQuestion {
            question,
            header: Some(header_for(&approval.kind).to_string()),
            multi_select: false,
            allow_custom,
            tool_name: Some(DOT_APPROVAL_TOOL_NAME.to_string()),
            recommended: None,
            preview_layout: false,
            options: labels
                .into_iter()
                .map(|label| SessionChatQuestionOption {
                    label,
                    description: None,
                    preview: None,
                })
                .collect(),
        }],
        tool_use_id: Some(format!("{DOT_APPROVAL_TOOL_USE_PREFIX}{}", approval.id)),
    }
}

/// The approval a stored chat prompt stands for, when it is a dot card.
pub(crate) fn approval_id_of_prompt(prompt: &SessionChatInteractivePrompt) -> Option<String> {
    prompt
        .tool_use_id()?
        .strip_prefix(DOT_APPROVAL_TOOL_USE_PREFIX)
        .map(str::to_string)
}

/// Whether a dot card is still waiting for an answer (a card shown in a chat whose approval was
/// settled or deleted is stale and may be replaced).
fn still_pending(state: &AppState, approval_id: &str) -> bool {
    store::open(&state.paths)
        .and_then(|db| store::get_approval(&db, approval_id))
        .ok()
        .flatten()
        .is_some_and(|approval| approval.status == "pending")
}

/// Writes (`Some`) or clears (`None`) the session's stored chat prompt and pushes it to open chats.
/// Clearing only removes a dot card, never an agent's own question; writing never replaces the
/// agent's own question or another dot card that is still pending (cards queue and
/// `show_next_in_chat` shows the oldest).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The read, change and write of `runtimeSettings` happen in one immediate transaction, because the update writes the whole object back and a hook or chat write landing between a separate read and write would be erased. A pending dot card is never replaced by a newer one, so an answer to the card the user is looking at can only settle that card.
fn set_session_prompt(
    state: &AppState,
    project_id: &str,
    session_id: &str,
    prompt: Option<&SessionChatInteractivePrompt>,
    clear_only: Option<&str>,
) {
    let Ok(db) = open_gxserver_database(&state.paths) else {
        return;
    };
    let server_id = state.metadata.server_id.as_str();
    let updated = {
        let Ok(transaction) = Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        else {
            return;
        };
        let repository = DomainRepository::new(&transaction, server_id);
        let Ok(Some(session)) = repository.get_session(project_id, session_id) else {
            return;
        };
        let stored = crate::agents::session_chat_prompt_setting(&session)
            .as_deref()
            .and_then(parse_stored_session_chat_prompt);
        let stored_card = stored.as_ref().and_then(approval_id_of_prompt);
        if let Some(approval_id) = clear_only {
            if stored_card.as_deref() != Some(approval_id) {
                return;
            }
        } else if stored.is_some() {
            let Some(stored_card) = stored_card else {
                // The agent's own question is on screen; the dot card waits in the OpenDots view
                // and comes back here once that question is answered.
                return;
            };
            let replacing_itself = prompt
                .and_then(approval_id_of_prompt)
                .is_some_and(|new_card| new_card == stored_card);
            if !replacing_itself && still_pending(state, &stored_card) {
                return;
            }
        }
        let mut runtime = session
            .get("runtimeSettings")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let activity = runtime
            .entry("agentActivity".to_string())
            .or_insert_with(|| json!({}));
        let Some(activity) = activity.as_object_mut() else {
            return;
        };
        match prompt {
            Some(prompt) => {
                activity.insert(
                    "sessionChatPrompt".to_string(),
                    Value::String(serde_json::to_string(prompt).unwrap_or_default()),
                );
            }
            None => {
                activity.remove("sessionChatPrompt");
            }
        }
        let mut update = Map::new();
        update.insert("projectId".to_string(), json!(project_id));
        update.insert("sessionId".to_string(), json!(session_id));
        update.insert("runtimeSettings".to_string(), Value::Object(runtime));
        // `update_session` joins the open transaction, so the read above and this write are one.
        let Ok(updated) = repository.update_session(&update) else {
            return;
        };
        if transaction.commit().is_err() {
            return;
        }
        updated
    };
    let repository = DomainRepository::new(&db, server_id);
    crate::session_chat_interactive::emit_session_chat_prompt_state_frame(state, &updated);
    let _ = crate::server::schedule_presentation_session_delta(
        state,
        &db,
        &repository,
        project_id,
        session_id,
    );
}

/// Puts a pending card back in its chat when nothing is shown there (a card that waited behind the
/// agent's own question, or one a hook cleared) or a stale card is. Called by the runtime's slow
/// tick. Blocking.
pub(crate) fn reshow_in_chat(state: &AppState, pending: &[Approval]) {
    let Ok(db) = open_gxserver_database(&state.paths) else {
        return;
    };
    let repository = DomainRepository::new(&db, state.metadata.server_id.as_str());
    let mut seen = std::collections::HashSet::new();
    for approval in pending {
        let Some(session_id) = approval.session_id.as_deref() else {
            continue;
        };
        if !seen.insert((approval.project_id.clone(), session_id.to_string())) {
            continue;
        }
        let Ok(Some(session)) = repository.get_session(&approval.project_id, session_id) else {
            continue;
        };
        let stored = crate::agents::session_chat_prompt_setting(&session)
            .as_deref()
            .and_then(parse_stored_session_chat_prompt);
        let needs_card = match stored.as_ref() {
            None => true,
            Some(stored) => {
                approval_id_of_prompt(stored).is_some_and(|card| !still_pending(state, &card))
            }
        };
        if needs_card {
            show_next_in_chat(state, &approval.project_id, session_id);
        }
    }
}

/// Shows the oldest pending card of a session in its chat (after one was answered, or when the
/// agent's own question went away).
pub(crate) fn show_next_in_chat(state: &AppState, project_id: &str, session_id: &str) {
    let Ok(db) = store::open(&state.paths) else {
        return;
    };
    let next = store::pending_approvals(&db)
        .unwrap_or_default()
        .into_iter()
        .find(|approval| approval.session_id.as_deref() == Some(session_id));
    if let Some(next) = next {
        set_session_prompt(
            state,
            project_id,
            session_id,
            Some(&chat_prompt(&next)),
            None,
        );
    }
}

/// Raises a card and waits for its answer. Blocking parts run off the async runtime.
pub(crate) async fn ask_and_wait(
    state: &Arc<AppState>,
    dot: &Dot,
    project_id: &str,
    session_id: Option<&str>,
    call_id: Option<&str>,
    new: NewApproval,
) -> Resolution {
    let approval = Approval {
        id: new_id("apv"),
        dot_id: dot.id.clone(),
        project_id: project_id.to_string(),
        session_id: session_id.map(str::to_string),
        kind: new.kind.to_string(),
        status: "pending".to_string(),
        title: new.title,
        detail: new.detail,
        tool: new.tool,
        options: new.options,
        rule_action: new.rule_action,
        toolkit: new.toolkit,
        url: new.url,
        answer: None,
        created_at: now_ms(),
        resolved_at: None,
    };
    // Subscribed before anything can block, so a shutdown that starts while the card waits wakes it.
    let mut shutdown = state.shutdown_tx.subscribe();
    let (sender, mut receiver) = oneshot::channel();
    with_waiters(|waiters| waiters.insert(approval.id.clone(), sender));
    if let Some(call_id) = call_id {
        with_calls(|calls| calls.insert(call_id.to_string(), approval.id.clone()));
    }
    let _guard = WaitGuard {
        approval_id: approval.id.clone(),
        call_id: call_id.map(str::to_string),
    };
    let raised = {
        let state = state.clone();
        let approval = approval.clone();
        tokio::task::spawn_blocking(move || -> Result<(), DomainStateError> {
            let db = store::open(&state.paths)?;
            store::insert_approval(&db, &approval)?;
            if let Some(session_id) = approval.session_id.as_deref() {
                set_session_prompt(
                    &state,
                    &approval.project_id,
                    session_id,
                    Some(&chat_prompt(&approval)),
                    None,
                );
            }
            publish::publish_changes(&state);
            Ok(())
        })
        .await
        .unwrap_or_else(|_| {
            Err(DomainStateError::bad_request(
                "The card could not be shown.",
            ))
        })
    };
    if raised.is_err() {
        return Resolution::Denied(Some("The card could not be shown to the user.".to_string()));
    }
    let deadline = tokio::time::Instant::now() + APPROVAL_TIMEOUT;
    loop {
        let poll = if approval.kind == "connect" {
            Duration::from_secs(3)
        } else {
            Duration::from_secs(60)
        };
        let wait_until = (tokio::time::Instant::now() + poll).min(deadline);
        let waited = tokio::select! {
            waited = tokio::time::timeout_at(wait_until, &mut receiver) => waited,
            // The card stays pending; the next gxserver start expires it (`startup_pass`).
            _ = shutdown.recv() => return Resolution::Restarting,
        };
        match waited {
            Ok(Ok(resolution)) => return resolution,
            Ok(Err(_)) => return stored_resolution(state, &approval.id).await,
            Err(_) if tokio::time::Instant::now() >= deadline => {
                return settle_and_read(state, &approval.id, "expired").await;
            }
            Err(_) => {
                // An app connection finishes in the user's browser; resume as soon as it is live.
                if approval.kind == "connect" {
                    let toolkit = approval.toolkit.clone().unwrap_or_default();
                    let paths = state.paths.clone();
                    let connected = tokio::task::spawn_blocking(move || {
                        composio::is_connected(&paths, &toolkit)
                    })
                    .await
                    .unwrap_or(false);
                    if connected {
                        return settle_and_read(state, &approval.id, "approved").await;
                    }
                }
            }
        }
    }
}

/// Removes a waiting tool call's entries when its wait ends in any way, including when the MCP
/// process exits and the HTTP handler is dropped mid-wait: the runtime's orphan sweep then expires
/// the card within one slow tick instead of keeping it for 30 minutes.
struct WaitGuard {
    approval_id: String,
    call_id: Option<String>,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        forget_waiter(&self.approval_id, self.call_id.as_deref());
    }
}

/// Settles a card from the waiting side (deadline, finished app connection) and returns how it
/// actually ended: when the user answered at the same moment, their answer wins.
async fn settle_and_read(
    state: &Arc<AppState>,
    approval_id: &str,
    status: &'static str,
) -> Resolution {
    let settled = {
        let state = state.clone();
        let id = approval_id.to_string();
        tokio::task::spawn_blocking(move || settle(&state, &id, status, None, false)).await
    };
    if let Ok(Ok(Some(_))) = settled {
        return if status == "approved" {
            Resolution::Approved
        } else {
            Resolution::Expired
        };
    }
    stored_resolution(state, approval_id).await
}

/// How a settled card ended, read from its row.
async fn stored_resolution(state: &Arc<AppState>, approval_id: &str) -> Resolution {
    let paths = state.paths.clone();
    let id = approval_id.to_string();
    let approval = tokio::task::spawn_blocking(move || {
        store::open(&paths)
            .and_then(|db| store::get_approval(&db, &id))
            .ok()
            .flatten()
    })
    .await
    .ok()
    .flatten();
    match approval {
        Some(approval) => match approval.status.as_str() {
            "approved" => Resolution::Approved,
            "answered" => Resolution::Answered(approval.answer.unwrap_or_default()),
            "denied" => Resolution::Denied(approval.answer),
            _ => Resolution::Expired,
        },
        None => Resolution::Expired,
    }
}

fn forget_waiter(approval_id: &str, call_id: Option<&str>) {
    with_waiters(|waiters| waiters.remove(approval_id));
    if let Some(call_id) = call_id {
        with_calls(|calls| {
            // A newer call may reuse the id only in theory; remove it only while it points here.
            if calls.get(call_id).map(String::as_str) == Some(approval_id) {
                calls.remove(call_id);
            }
        });
    }
}

/// Moves a pending card to `status`, wakes its tool call, clears it from the chat and shows the
/// session's next one. `always` also makes the dot's rule for that action an allow rule. Blocking.
fn settle(
    state: &AppState,
    approval_id: &str,
    status: &str,
    answer: Option<&str>,
    always: bool,
) -> Result<Option<Approval>, DomainStateError> {
    let db = store::open(&state.paths)?;
    let Some(approval) = store::settle_approval(&db, approval_id, status, answer)? else {
        return Ok(None);
    };
    if always {
        let action = approval
            .rule_action
            .clone()
            .unwrap_or_else(|| approval.title.clone());
        store::allow_rule_for_dot(&db, &approval.dot_id, &action)?;
        instructions::refresh_dot(&state.paths, &approval.dot_id);
    }
    let resolution = match status {
        "approved" => Resolution::Approved,
        "answered" => Resolution::Answered(answer.unwrap_or_default().to_string()),
        "denied" => Resolution::Denied(answer.map(str::to_string)),
        _ => Resolution::Expired,
    };
    if let Some(sender) = with_waiters(|waiters| waiters.remove(approval_id)) {
        let _ = sender.send(resolution);
    }
    if let Some(session_id) = approval.session_id.as_deref() {
        set_session_prompt(
            state,
            &approval.project_id,
            session_id,
            None,
            Some(&approval.id),
        );
        show_next_in_chat(state, &approval.project_id, session_id);
    }
    publish::publish_changes(state);
    Ok(Some(approval))
}

/// `resolveApproval` and the CLI: `approve|deny|always|answer`.
pub(crate) fn resolve(
    state: &AppState,
    approval_id: &str,
    choice: &str,
    answer: Option<&str>,
) -> Result<Approval, DomainStateError> {
    let db = store::open(&state.paths)?;
    let approval = store::get_approval(&db, approval_id)?
        .ok_or_else(|| DomainStateError::not_found("That request no longer exists."))?;
    if approval.status != "pending" {
        return Ok(approval);
    }
    let answer = answer.map(str::trim).filter(|answer| !answer.is_empty());
    let (status, always) = match (choice, approval.kind.as_str()) {
        ("answer", _) if answer.is_none() => {
            return Err(DomainStateError::bad_request("Type an answer first."));
        }
        ("answer", "question") => ("answered", false),
        ("approve", "question") if answer.is_some() => ("answered", false),
        ("approve" | "answer", _) => ("approved", false),
        ("always", "question") => {
            return Err(DomainStateError::bad_request(
                "A question needs an answer, not a rule.",
            ));
        }
        ("always", _) => ("approved", true),
        ("deny", _) => ("denied", false),
        (other, _) => {
            return Err(DomainStateError::bad_request(format!(
                "\"{other}\" is not a choice; use approve, deny, always or answer."
            )));
        }
    };
    if status == "approved" && approval.kind == "question" {
        return Err(DomainStateError::bad_request("Type an answer first."));
    }
    Ok(settle(state, approval_id, status, answer, always)?.unwrap_or(approval))
}

/// Expires a card nobody can answer any more (its session closed, its tool call was cancelled,
/// gxserver restarted). Blocking.
pub(crate) fn expire(state: &AppState, approval_id: &str) {
    let _ = settle(state, approval_id, "expired", None, false);
}

/// An MCP call the agent cancelled (the user pressed Stop): its card expires.
pub(crate) fn cancel_call(state: &AppState, call_id: &str) -> bool {
    let Some(approval_id) = with_calls(|calls| calls.remove(call_id)) else {
        return false;
    };
    expire(state, &approval_id);
    true
}

/// Whether a tool call is still waiting on this approval in this gxserver.
pub(crate) fn has_waiter(approval_id: &str) -> bool {
    with_waiters(|waiters| waiters.contains_key(approval_id))
}

/// The chat card's answer for a dot card (`answerSessionChatPrompt` with `kind: "question"`, the
/// only kind a question card sends). `None` when the session's stored prompt is not a dot card or
/// the answer is another kind (a terminal dialog, an approval key, an async question), which then
/// goes the ordinary way. Blocking.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Only a question answer can be meant for a dot card: other kinds (a terminal dialog's Escape, an approval key) are answers to the agent's own screen and used to deny the dot card and drop the user's real answer. A `toolUseId` in the answer that is not this card's means the user answered a card that is gone, so it is refused instead of settling the card that replaced it.
pub(crate) fn answer_from_chat(
    state: &AppState,
    session: &Value,
    params: &Map<String, Value>,
) -> Option<Result<Value, DomainStateError>> {
    if params.get("kind").and_then(Value::as_str) != Some("question") {
        return None;
    }
    let stored = crate::agents::session_chat_prompt_setting(session)
        .as_deref()
        .and_then(parse_stored_session_chat_prompt)?;
    let approval_id = approval_id_of_prompt(&stored)?;
    if let Some(answered_card) = params
        .get("toolUseId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        if Some(answered_card) != stored.tool_use_id() {
            return Some(Err(DomainStateError::bad_request(
                "That question changed before your answer arrived. Look at the chat again.",
            )));
        }
    }
    let project_id = session
        .get("projectId")
        .and_then(Value::as_str)?
        .to_string();
    let session_id = session
        .get("sessionId")
        .and_then(Value::as_str)?
        .to_string();
    let db = match store::open(&state.paths) {
        Ok(db) => db,
        Err(error) => return Some(Err(error)),
    };
    let approval = match store::get_approval(&db, &approval_id) {
        Ok(Some(approval)) if approval.status == "pending" => approval,
        Ok(_) => {
            // Already answered elsewhere: the card is stale, so it simply goes away.
            set_session_prompt(state, &project_id, &session_id, None, Some(&approval_id));
            show_next_in_chat(state, &project_id, &session_id);
            return Some(Ok(json!({ "answered": true })));
        }
        Err(error) => return Some(Err(error)),
    };
    let selection = params
        .get("selections")
        .and_then(Value::as_array)
        .and_then(|selections| selections.first());
    let picked = selection
        .and_then(|selection| selection.get("indices"))
        .and_then(Value::as_array)
        .and_then(|indices| indices.first())
        .and_then(Value::as_u64)
        .map(|index| index as usize);
    let typed = selection
        .and_then(|selection| selection.get("other"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let result = match approval.kind.as_str() {
        "question" => {
            let label = picked.and_then(|index| approval.options.get(index).cloned());
            match typed.or(label) {
                Some(answer) => settle(state, &approval.id, "answered", Some(&answer), false),
                None => settle(state, &approval.id, "denied", None, false),
            }
        }
        "connect" => {
            let connected = picked == Some(0);
            settle(
                state,
                &approval.id,
                if connected { "approved" } else { "denied" },
                None,
                false,
            )
        }
        _ => match picked {
            Some(0) => settle(state, &approval.id, "approved", None, false),
            Some(1) => settle(state, &approval.id, "approved", None, true),
            _ => settle(state, &approval.id, "denied", typed.as_deref(), false),
        },
    };
    Some(result.map(|_| json!({ "answered": true })))
}
