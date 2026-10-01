//! Open Dot's read/write rule for Composio calls (composio.ts `executeDecision`, `describeExecute`,
//! `executeDetail`): reads run on their own, anything that sends, posts, creates, changes or
//! deletes asks first.

use serde_json::Value;

use super::names::pretty_name;
use super::ComposioDecision;

/// Open Dot's `READ_VERB` words: a tool whose action starts with one of these only reads.
const READ_VERBS: &[&str] = &[
    "GET", "LIST", "SEARCH", "FETCH", "FIND", "READ", "RETRIEVE", "QUERY", "DESCRIBE", "LOOKUP",
    "VIEW", "CHECK", "COUNT", "EXPORT", "DOWNLOAD",
];

/// Words that send, change or delete something. A slug with any of them asks, even after a read
/// verb (`*_GET_OR_CREATE_*`, `*_FIND_AND_REPLACE_*`). Words that are also common nouns in read
/// slugs (RUN, ORDER, DRAFT, LABEL, UPDATES) are left out; a slug that starts with them still asks
/// because it does not start with a read verb.
const WRITE_VERBS: &[&str] = &[
    "SEND",
    "SENDS",
    "POST",
    "CREATE",
    "CREATES",
    "CREATION",
    "ADD",
    "ADDS",
    "INSERT",
    "UPSERT",
    "UPDATE",
    "EDIT",
    "MODIFY",
    "PATCH",
    "PUT",
    "SET",
    "CHANGE",
    "REPLACE",
    "RENAME",
    "DELETE",
    "DELETES",
    "REMOVE",
    "REMOVES",
    "TRASH",
    "ARCHIVE",
    "UNARCHIVE",
    "MOVE",
    "MERGE",
    "CLOSE",
    "REOPEN",
    "REPLY",
    "FORWARD",
    "SHARE",
    "INVITE",
    "ASSIGN",
    "UNASSIGN",
    "APPROVE",
    "REJECT",
    "PUBLISH",
    "UPLOAD",
    "IMPORT",
    "SCHEDULE",
    "RESCHEDULE",
    "CANCEL",
    "ACCEPT",
    "DECLINE",
    "SUBMIT",
    "EXECUTE",
    "TRIGGER",
    "ENABLE",
    "DISABLE",
    "REVOKE",
    "GRANT",
    "TRANSFER",
    "PAY",
    "PURCHASE",
    "SUBSCRIBE",
    "UNSUBSCRIBE",
    "FOLLOW",
    "UNFOLLOW",
    "BLOCK",
    "UNBLOCK",
    "APPEND",
    "WRITE",
    "SAVE",
    "RESTORE",
    "PURGE",
    "DESTROY",
    "TERMINATE",
    "MARK",
    "STAR",
    "UNSTAR",
    "PIN",
    "UNPIN",
    "CLEAR",
    "RESET",
    "DUPLICATE",
    "COMPOSE",
    "LOCK",
    "UNLOCK",
    "DEPLOY",
    "START",
    "STOP",
    "JOIN",
    "LEAVE",
    "KICK",
    "BAN",
    "MUTE",
    "UNMUTE",
    "LIKE",
    "UNLIKE",
    "REACT",
    "TOGGLE",
];

/// Composio toolkits whose slug is more than one word (`MICROSOFT_TEAMS_LIST_CHATS`); every other
/// tool slug starts with a one-word toolkit (`GMAIL_FETCH_EMAILS`).
const MULTI_WORD_TOOLKITS: &[&str] = &[
    "GOOGLE_SEARCH_CONSOLE",
    "GOOGLE_ANALYTICS",
    "GOOGLE_MAPS",
    "GOOGLE_ADS",
    "MICROSOFT_TEAMS",
    "MICROSOFT_CLARITY",
    "ONE_DRIVE",
    "SHARE_POINT",
    "ZOHO_BOOKS",
    "ZOHO_CRM",
    "ZOHO_DESK",
    "ZOHO_INVOICE",
];

/// The action words of a tool slug, without its toolkit prefix.
fn action_words(slug: &str) -> Vec<String> {
    let upper = slug.trim().to_uppercase();
    let rest = MULTI_WORD_TOOLKITS
        .iter()
        .find_map(|toolkit| upper.strip_prefix(&format!("{toolkit}_")))
        .map(str::to_string)
        .unwrap_or_else(|| {
            upper
                .split_once('_')
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_default()
        });
    rest.split('_')
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// A tool that only reads: its action starts with a read verb and no write verb appears anywhere.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Open Dot treated a slug as a read when any word after the first was a read verb, so `GOOGLECALENDAR_DELETE_TASK_LIST` and `GITHUB_CREATE_A_CHECK_RUN` ran without asking. Here the toolkit prefix is stripped, the first remaining word must be a read verb, and any write verb anywhere asks; an unknown shape asks.
pub(crate) fn is_read_slug(slug: &str) -> bool {
    let words = action_words(slug);
    let Some(first) = words.first() else {
        return false;
    };
    READ_VERBS.contains(&first.as_str())
        && !words
            .iter()
            .any(|word| WRITE_VERBS.contains(&word.as_str()))
}

/// `COMPOSIO_MULTI_EXECUTE_TOOL`'s `tools: [{ tool_slug, arguments }]`.
fn execute_items(arguments: &Value) -> Vec<&Value> {
    arguments
        .get("tools")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn item_slug(item: &Value) -> String {
    match item.get("tool_slug") {
        Some(Value::String(slug)) => slug.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Reads run automatically; anything else asks first (Open Dot's `executeDecision`).
pub(crate) fn execute_decision(arguments: &Value) -> ComposioDecision {
    let items = execute_items(arguments);
    if !items.is_empty() && items.iter().all(|item| is_read_slug(&item_slug(item))) {
        ComposioDecision::Allow
    } else {
        ComposioDecision::Ask
    }
}

/// The action the rules reviewer reads (Open Dot's `describeExecute`): the model's `thought` when it
/// gave one, else "use Gmail to send email".
pub(crate) fn describe_execute(arguments: &Value) -> String {
    let items = execute_items(arguments);
    let mut apps: Vec<String> = Vec::new();
    for item in &items {
        let slug = item_slug(item);
        let app = pretty_name(&slug.split('_').next().unwrap_or_default().to_lowercase());
        if !apps.contains(&app) {
            apps.push(app);
        }
    }
    let actions = items
        .iter()
        .map(|item| {
            item_slug(item)
                .split('_')
                .skip(1)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .filter(|action| !action.is_empty())
        .collect::<Vec<_>>();
    let thought = arguments
        .get("thought")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|thought| !thought.is_empty())
        .map(|thought| thought.strip_suffix('.').unwrap_or(thought));
    match thought {
        Some(thought) => {
            let mut chars = thought.chars();
            let first = chars
                .next()
                .map(|c| c.to_lowercase().collect::<String>())
                .unwrap_or_default();
            format!("{first}{} (using {})", chars.as_str(), apps.join(", "))
        }
        None => format!("use {} to {}", apps.join(", "), actions.join(", ")),
    }
}

/// `JSON.stringify(value, null, 1)`.
fn stringify_one_space(value: &Value) -> String {
    use serde::Serialize;
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    if value.serialize(&mut serializer).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The approval card's detail (Open Dot's `executeDetail`): each tool slug and the first 700
/// characters of its arguments.
pub(crate) fn execute_detail(arguments: &Value) -> String {
    execute_items(arguments)
        .iter()
        .map(|item| {
            let arguments = match item.get("arguments") {
                Some(Value::Null) | None => Value::Object(Default::default()),
                Some(arguments) => arguments.clone(),
            };
            let text = stringify_one_space(&arguments);
            let clipped = match text.char_indices().nth(700) {
                Some((index, _)) => &text[..index],
                None => &text,
            };
            format!("{}\n{clipped}", item_slug(item))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
