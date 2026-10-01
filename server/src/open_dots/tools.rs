//! The dot's tools (Open Dot's `agent/tools.ts`), served to every dot session by the
//! `ghostex dots mcp` server and run here, where the records, the rules, the browser, the vault and
//! Composio live. Every call passes Open Dot's gate (`processCalls`): precheck, then the user's
//! rules against a plain-language description of the action (never refuses, ask waits for an
//! approval card), then the tool runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use serde_json::{json, Value};

use super::approvals::{self, NewApproval, Resolution};
use super::browser::{self, ClickTarget, FieldTarget};
use super::model::{Dot, Routine};
use super::{
    composio, cron, handoffs, instructions, publish, review, routines, store, updates, vault,
};
use crate::domain::DomainStateError;
use crate::paths::GxserverPaths;
use crate::server::AppState;

/// Who is calling: the dot, and the conversation the call came from (absent when the MCP server
/// runs outside a Ghostex session).
#[derive(Clone, Debug)]
pub(crate) struct ToolContext {
    pub(crate) dot: Dot,
    pub(crate) project_id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) call_id: Option<String>,
}

/// An MCP `tools/call` result.
pub(crate) struct ToolOutput {
    content: Vec<Value>,
    is_error: bool,
}

impl ToolOutput {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![json!({ "type": "text", "text": text.into() })],
            is_error: false,
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![json!({ "type": "text", "text": text.into() })],
            is_error: true,
        }
    }

    pub(crate) fn to_value(&self) -> Value {
        json!({ "content": self.content, "isError": self.is_error })
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

/// The tools every dot has (CONTRACT §5), with Open Dot's descriptions.
fn own_tools() -> Vec<Value> {
    vec![
        tool("remember", "Save a durable fact or preference about the user or their work to your memory, so you know it in future conversations.", object(json!({ "fact": string("The fact, written as a short standalone sentence") }), &["fact"])),
        tool("forget", "Delete a memory that is wrong or outdated.", object(json!({ "memory_id": string("The id shown in your memory list") }), &["memory_id"])),
        tool("save_skill", "Save a reusable skill: step-by-step markdown instructions for a task you'll repeat. Updates the skill if the name exists.", object(json!({
            "name": string("Short skill name"),
            "description": string("One line: when to use it"),
            "instructions": string("Markdown instructions"),
        }), &["name", "description", "instructions"])),
        tool("use_skill", "Load the full instructions of one of your saved skills.", object(json!({ "name": string("Skill name") }), &["name"])),
        tool("create_routine", "Create a recurring task you'll run on a schedule on your own, e.g. a morning briefing. Results reach the user via send_update.", object(json!({
            "name": string("Short name"),
            "instruction": string("What to do each time, written as a full instruction to yourself"),
            "schedule": string("5-field cron expression in the user's local timezone, e.g. '0 8 * * 1-5' for weekdays at 8am"),
        }), &["name", "instruction", "schedule"])),
        tool("delete_routine", "Delete one of your routines.", object(json!({ "routine_id": string("Routine id") }), &["routine_id"])),
        tool("list_routines", "List your routines with their schedules and next runs.", object(json!({}), &[])),
        tool("send_update", "Proactively message the user with a notification: for progress on long work, or to deliver results of background or routine work. Give finished deliverables a short title like 'Your research is ready'.", object(json!({
            "title": { "type": ["string", "null"], "description": "Short notification title, or null" },
            "text": string("The message (markdown)"),
        }), &["title", "text"])),
        tool("share_file", "Send a file from your folder to the user (reports, spreadsheets, images, exports, code). They get a notification and can open it. Write the file first, then share it.", object(json!({
            "path": string("Path of the file, relative to your folder or absolute"),
            "note": string("A short message to go with it"),
        }), &["path", "note"])),
        tool("message_dot", "Ask another of the user's dots for help or hand off a sub-task. Their reply arrives later in this conversation as a message starting with \"[Reply from <name>]\".", object(json!({
            "dot_name": string("The other dot's name"),
            "message": string("Your message to them, with all needed context"),
        }), &["dot_name", "message"])),
        tool("request_approval", "Ask the user to approve an action before you take it (sending messages on their behalf, purchases, deleting things, submitting forms, anything irreversible or public). Waits for their decision.", object(json!({
            "action": string("What you want to do, one line"),
            "details": string("Exactly what will happen: recipients, amounts, content, etc."),
        }), &["action", "details"])),
        tool("ask_user", "Ask the user a question and wait for the answer. Offer 2-4 likely answers as options when possible.", object(json!({
            "question": string("The question"),
            "options": { "type": "array", "items": { "type": "string" }, "description": "Suggested answers (may be empty)" },
        }), &["question", "options"])),
        tool("sign_in", "Fill the login form on the current browser page with the user's saved password for that page's site (an https page whose address matches a saved login; the page's own address picks the login). You never see the password. Open the site's sign-in page first, then submit the form yourself afterwards.", object(json!({ "site": string("The site you expect to sign in to, e.g. github.com") }), &["site"])),
        tool("open_url", "Open a URL in your browser. Your browser keeps its logins.", object(json!({ "url": string("URL to open") }), &["url"])),
        tool("read_page", "Get the visible text of the page currently open in your browser.", object(json!({}), &[])),
        tool("page_snapshot", "Get an outline of the interactive elements on the page (buttons, links, fields) with refs like [e12] you can pass to click and type_text.", object(json!({}), &[])),
        tool("click", "Click something on the page open in your browser: by a ref from page_snapshot, or by its visible text (a button, link, tab, option, checkbox or label), e.g. \"Continue\". Read the page first so you use the exact text.", object(json!({
            "ref": string("A ref from page_snapshot, e.g. e12"),
            "text": string("The visible text of what to click"),
        }), &[])),
        tool("type_text", "Type into a field on the page open in your browser, found by a ref from page_snapshot or by its label, placeholder or name. Set submit to press Enter afterwards. Never use it for passwords (use sign_in).", object(json!({
            "ref": string("A ref from page_snapshot, e.g. e7"),
            "field": string("Label, placeholder or name of the field"),
            "text": string("What to type"),
            "submit": { "type": "boolean", "description": "Press Enter after typing" },
        }), &["text"])),
        tool("press_key", "Press a key in your browser, e.g. Enter, Escape, Tab, ArrowDown.", object(json!({ "key": string("The key to press") }), &["key"])),
        tool("scroll", "Scroll the page open in your browser.", object(json!({
            "direction": { "type": "string", "enum": ["up", "down"], "description": "Which way to scroll" },
            "amount": { "type": "number", "description": "How far, in pixels (default 600)" },
        }), &["direction"])),
        tool("screenshot", "See the page open in your browser as an image.", object(json!({}), &[])),
        tool("go_back", "Go back to the previous page in your browser.", object(json!({}), &[])),
    ]
}

fn app_connect_tool() -> Value {
    tool("app_connect", "Ask the user to connect one of their apps to Composio (shows a Connect card with a sign-in link) and wait until they finish. Use the exact toolkit slug from COMPOSIO_SEARCH_TOOLS.", object(json!({ "toolkit": string("Toolkit slug, e.g. gmail, googlecalendar, slack, notion, github") }), &["toolkit"]))
}

/// The MCP tool list a dot's session sees. Blocking (asks Composio for its tools).
pub(crate) fn tool_list(paths: &GxserverPaths) -> Vec<Value> {
    let mut tools = own_tools();
    if composio::signed_in(paths) {
        tools.push(app_connect_tool());
        tools.extend(composio::mcp_tools(paths));
    }
    tools
}

// ------------------------------------------------------------------ helpers

fn text_arg(args: &Value, key: &str) -> String {
    match args.get(key) {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
        clipped.push('…');
        clipped
    }
}

fn capitalize(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// Open Dot's RISKY_CLICK: clicks that spend money, send things or delete things ask first unless
/// a rule says otherwise.
fn is_risky_click(label: &str) -> bool {
    const WORDS: [&str; 13] = [
        "pay",
        "buy",
        "purchase",
        "checkout",
        "confirm",
        "book",
        "reserve",
        "subscribe",
        "send",
        "submit",
        "delete",
        "remove",
        "cancel",
    ];
    const PHRASES: [[&str; 2]; 3] = [["place", "order"], ["order", "now"], ["check", "out"]];
    let words: Vec<String> = label
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    words.iter().any(|word| WORDS.contains(&word.as_str()))
        || words.windows(2).any(|pair| {
            PHRASES
                .iter()
                .any(|phrase| pair[0] == phrase[0] && pair[1] == phrase[1])
        })
}

fn normalize_ref(reference: &str) -> String {
    reference
        .trim()
        .trim_start_matches('@')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string()
}

/// The last `page_snapshot` each dot read: the label a ref had when the dot saw it, for when the
/// live page cannot be asked (its browser is not running).
static LAST_SNAPSHOT: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn remember_snapshot(dot_id: &str, snapshot: &str) {
    LAST_SNAPSHOT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(dot_id.to_string(), snapshot.to_string());
}

/// The rest of the snapshot line that shows `[<ref>]`, e.g. `button "Continue"`.
fn snapshot_ref_label(dot_id: &str, reference: &str) -> Option<String> {
    let marker = format!("[{}]", normalize_ref(reference));
    let guard = LAST_SNAPSHOT.lock().unwrap_or_else(PoisonError::into_inner);
    let snapshot = guard.as_ref()?.get(dot_id)?;
    snapshot.lines().find_map(|line| {
        let index = line.find(&marker)?;
        let label = line[index + marker.len()..].trim();
        (!label.is_empty()).then(|| label.to_string())
    })
}

/// How the gate sees a call.
struct Gate {
    /// A message that answers the call without running it.
    precheck: Option<String>,
    /// The action in plain words, checked against the user's rules; `None` always runs.
    describe: Option<String>,
    default_decision: &'static str,
    detail: Option<String>,
}

const BROWSER_TOOLS: [&str; 10] = [
    "open_url",
    "read_page",
    "page_snapshot",
    "click",
    "type_text",
    "press_key",
    "scroll",
    "screenshot",
    "go_back",
    "sign_in",
];

/// Composio's own tools that only find tools and read their schemas: they touch none of the
/// user's apps, so they run without a rule check. Every other proxied Composio tool asks first
/// unless a rule allows it (`COMPOSIO_MULTI_EXECUTE_TOOL` and `COMPOSIO_MANAGE_CONNECTIONS` have
/// their own read rules below).
const FREE_COMPOSIO_TOOLS: [&str; 2] = ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_GET_TOOL_SCHEMAS"];

fn is_own_tool(name: &str) -> bool {
    name == "app_connect"
        || own_tools()
            .iter()
            .any(|tool| tool["name"].as_str() == Some(name))
}

/// What a click would click, judged before it runs: the label of the element the click resolves to
/// on the live page (by ref, or by text with the click's own matching), else the label the dot's
/// last snapshot showed for that ref. `None` when neither can be read.
fn click_target_label(paths: &GxserverPaths, dot_id: &str, args: &Value) -> Option<String> {
    let reference = text_arg(args, "ref");
    if !reference.is_empty() {
        // `click` uses the ref whenever one is given, whatever the text says.
        let reference = normalize_ref(&reference);
        return browser::ref_label(paths, dot_id, &reference)
            .or_else(|| snapshot_ref_label(dot_id, &reference));
    }
    let text = text_arg(args, "text");
    if text.is_empty() {
        return Some(String::new());
    }
    browser::click_label(paths, dot_id, &ClickTarget::Text(text))
}

/// How the gate sees a call. Blocking: a click reads the live page.
fn gate_for(paths: &GxserverPaths, dot: &Dot, name: &str, args: &Value) -> Gate {
    let mut gate = Gate {
        precheck: None,
        describe: None,
        default_decision: "allow",
        detail: None,
    };
    if BROWSER_TOOLS.contains(&name) && browser::is_taken_over(&dot.id) {
        gate.precheck = Some("The user is using your browser right now (they took it over from your Computer tab). Wait until they hand it back, or ask them with ask_user.".to_string());
        return gate;
    }
    match name {
        "open_url" => {
            gate.describe = Some(format!("open {} in its browser", text_arg(args, "url")))
        }
        "click" => {
            // CDXC:OpenDots 2026-10-01 WHY:
            // The risk is judged on the element the click will actually click (the ref wins over the text, and text matches by prefix and substring), so a ref to "Pay" sent with the text "Next", or the text "order" that lands on "Place order", cannot slip past the ask. A click whose target cannot be read asks.
            let target = click_target_label(paths, &dot.id, args);
            let typed = text_arg(args, "text");
            let label = match (&target, text_arg(args, "ref")) {
                (Some(label), _) if !label.is_empty() => label.clone(),
                (_, reference) if !reference.is_empty() => {
                    format!("element {}", normalize_ref(&reference))
                }
                _ => typed.clone(),
            };
            gate.describe = Some(format!("click \"{}\" in its browser", clip(&label, 120)));
            let risky = match &target {
                Some(found) => {
                    is_risky_click(found)
                        || (text_arg(args, "ref").is_empty() && is_risky_click(&typed))
                }
                None => true,
            };
            if risky {
                gate.default_decision = "ask";
            }
            if target.is_none() {
                gate.detail = Some(
                    "What it would click could not be read from the page before clicking."
                        .to_string(),
                );
            }
        }
        "type_text" => {
            let field = match (text_arg(args, "field"), text_arg(args, "ref")) {
                (field, _) if !field.is_empty() => field,
                (_, reference) if !reference.is_empty() => {
                    let reference = normalize_ref(&reference);
                    browser::ref_label(paths, &dot.id, &reference)
                        .filter(|label| !label.is_empty())
                        .or_else(|| snapshot_ref_label(&dot.id, &reference))
                        .unwrap_or_else(|| format!("field {reference}"))
                }
                _ => "the focused field".to_string(),
            };
            gate.describe = Some(format!(
                "type \"{}\" into \"{field}\" in its browser",
                clip(&text_arg(args, "text"), 60)
            ));
        }
        "press_key" => {
            gate.describe = Some(format!("press {} in its browser", text_arg(args, "key")))
        }
        "create_routine" => {
            gate.describe = Some(format!(
                "set up a recurring routine \"{}\" ({})",
                text_arg(args, "name"),
                text_arg(args, "schedule")
            ))
        }
        "delete_routine" => {
            gate.describe = Some(format!("delete routine {}", text_arg(args, "routine_id")))
        }
        "message_dot" => {
            gate.describe = Some(format!(
                "message the dot \"{}\"",
                text_arg(args, "dot_name")
            ))
        }
        "COMPOSIO_MULTI_EXECUTE_TOOL" => {
            gate.describe = Some(composio::describe_execute(args));
            gate.default_decision = match composio::execute_decision(args) {
                composio::ComposioDecision::Allow => "allow",
                composio::ComposioDecision::Ask => "ask",
            };
            gate.detail = Some(composio::execute_detail(args));
        }
        "COMPOSIO_MANAGE_CONNECTIONS" => {
            let toolkits = args
                .get("toolkits")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let action = |toolkit: &Value| {
                toolkit
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("add")
                    .to_string()
            };
            if toolkits.iter().any(|toolkit| action(toolkit) == "add") {
                gate.precheck = Some("To connect an app, call app_connect with the toolkit slug instead (it shows the user a Connect card).".to_string());
                return gate;
            }
            gate.describe = Some(format!(
                "change app connections ({})",
                clip(&Value::Array(toolkits.clone()).to_string(), 120)
            ));
            if !toolkits.iter().all(|toolkit| action(toolkit) == "list") {
                gate.default_decision = "ask";
            }
        }
        // CDXC:OpenDots 2026-10-01 WHY:
        // Any other tool Composio's server lists can reach the user's apps, so it is described for the rules and asks by default; only the search and schema tools run freely. An unknown tool used to get no description at all, which skipped even a "never" rule.
        composio_tool
            if !is_own_tool(composio_tool) && !FREE_COMPOSIO_TOOLS.contains(&composio_tool) =>
        {
            gate.describe = Some(format!(
                "use the app tool {composio_tool} ({})",
                clip(&args.to_string(), 120)
            ));
            gate.default_decision = "ask";
            gate.detail = Some(clip(
                &serde_json::to_string_pretty(args).unwrap_or_default(),
                700,
            ));
        }
        _ => {}
    }
    gate
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

fn from_result(result: Result<String, String>) -> ToolOutput {
    match result {
        Ok(text) => ToolOutput::text(text),
        Err(error) => ToolOutput::error(format!("Error: {error}")),
    }
}

fn routine_line(routine: &Routine) -> String {
    format!(
        "- [{}] {}: \"{}\"{}: {}{}",
        routine.id,
        routine.name,
        routine.schedule,
        if routine.enabled { "" } else { " (paused)" },
        routine.instruction,
        routine
            .next_run_at
            .map(|at| format!(" (next run {})", cron::describe_time(at)))
            .unwrap_or_default()
    )
}

// ------------------------------------------------------------------ execution

/// Runs one call through the gate. Never fails: problems come back as tool errors the model reads.
pub(crate) async fn call_tool(
    state: &Arc<AppState>,
    context: ToolContext,
    name: &str,
    args: Value,
) -> ToolOutput {
    let args = if args.is_object() { args } else { json!({}) };
    let dot = context.dot.clone();
    let known = is_own_tool(name) || {
        let paths = state.paths.clone();
        let wanted = name.to_string();
        blocking(move || {
            composio::signed_in(&paths)
                && composio::mcp_tools(&paths)
                    .iter()
                    .any(|tool| tool.get("name").and_then(Value::as_str) == Some(wanted.as_str()))
        })
        .await
        .unwrap_or(false)
    };
    if !known {
        return ToolOutput::error(format!("Unknown tool {name}"));
    }

    // The tools that exist to wait for the user.
    match name {
        "ask_user" => {
            let question = text_arg(&args, "question");
            if question.is_empty() {
                return ToolOutput::error("Ask a question.");
            }
            let options: Vec<String> = args
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|option| !option.is_empty())
                        .map(str::to_string)
                        .take(6)
                        .collect()
                })
                .unwrap_or_default();
            let resolution = wait_for_user(
                state,
                &context,
                NewApproval {
                    kind: "question",
                    title: question,
                    detail: None,
                    tool: Some(name.to_string()),
                    options,
                    rule_action: None,
                    toolkit: None,
                    url: None,
                },
            )
            .await;
            return ToolOutput::text(match resolution {
                Resolution::Answered(answer) => format!("The user answered: {answer}"),
                Resolution::Approved => "The user acknowledged the question without an answer.".to_string(),
                Resolution::Denied(_) => "The user dismissed the question without answering. Continue without it, or ask differently if you really need it.".to_string(),
                Resolution::Expired => "The user didn't answer in time. Continue without the answer, or ask again later.".to_string(),
                Resolution::Restarting => RESTARTING_MESSAGE.to_string(),
            });
        }
        "request_approval" => {
            let action = text_arg(&args, "action");
            if action.is_empty() {
                return ToolOutput::error("Say what you want to do.");
            }
            let details = text_arg(&args, "details");
            let resolution = wait_for_user(
                state,
                &context,
                NewApproval {
                    kind: "approval",
                    title: capitalize(&action),
                    detail: (!details.is_empty()).then_some(details),
                    tool: Some(name.to_string()),
                    options: Vec::new(),
                    rule_action: Some(action),
                    toolkit: None,
                    url: None,
                },
            )
            .await;
            return ToolOutput::text(match resolution {
                Resolution::Approved | Resolution::Answered(_) => "The user approved. Go ahead.".to_string(),
                Resolution::Denied(note) => format!(
                    "The user denied this. Do not do it; tell them briefly what you'll do instead, if anything.{}",
                    note.map(|note| format!(" They said: {note}")).unwrap_or_default()
                ),
                Resolution::Expired => "The user didn't answer in time, so don't do it. Ask again later if it still matters.".to_string(),
                Resolution::Restarting => format!("{RESTARTING_MESSAGE} Don't do it until they approve."),
            });
        }
        "app_connect" => return app_connect(state, &context, &args).await,
        "sign_in" => return sign_in(state, &context, &args).await,
        _ => {}
    }

    let gate = {
        let paths = state.paths.clone();
        let (gate_dot, gate_name, gate_args) = (dot.clone(), name.to_string(), args.clone());
        blocking(move || gate_for(&paths, &gate_dot, &gate_name, &gate_args)).await
    };
    let Some(gate) = gate else {
        return ToolOutput::error("Error: the tool could not be checked against your rules.");
    };
    if let Some(message) = gate.precheck {
        return ToolOutput::text(message);
    }
    if let Some(action) = gate.describe.as_deref() {
        if let Some(stop) = check_rules(
            state,
            &context,
            name,
            action,
            gate.default_decision,
            gate.detail.clone(),
        )
        .await
        {
            return stop;
        }
    }
    execute(state, &context, name, args).await
}

/// The answer for a tool call whose card was still waiting when gxserver began shutting down.
const RESTARTING_MESSAGE: &str =
    "Ghostex is restarting, so the user could not answer. Ask again in a moment.";

/// Open Dot's rule gate for one described action: a "never" rule refuses, "ask" (from a rule or
/// the tool's default) waits on an approval card. `None` means go ahead; `Some` is the tool's
/// answer when it must not run.
async fn check_rules(
    state: &Arc<AppState>,
    context: &ToolContext,
    tool_name: &str,
    action: &str,
    default_decision: &'static str,
    detail: Option<String>,
) -> Option<ToolOutput> {
    let verdict = review::review(&state.paths, &context.dot, action, default_decision).await;
    match verdict.decision.as_str() {
        "never" => Some(ToolOutput::text(format!(
            "Not allowed: the user's rule says never {}. Don't try to work around it.",
            verdict
                .rule
                .as_ref()
                .map(|rule| rule.action.as_str())
                .unwrap_or("do this")
        ))),
        "ask" => {
            let detail = [
                detail,
                verdict
                    .rule
                    .as_ref()
                    .map(|rule| format!("Your rule: ask first when it wants to {}.", rule.action)),
            ]
            .into_iter()
            .flatten()
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
            let resolution = wait_for_user(
                state,
                context,
                NewApproval {
                    kind: "approval",
                    title: capitalize(action),
                    detail: (!detail.is_empty()).then_some(detail),
                    tool: Some(tool_name.to_string()),
                    options: Vec::new(),
                    rule_action: Some(
                        verdict
                            .rule
                            .as_ref()
                            .map(|rule| rule.action.clone())
                            .unwrap_or_else(|| action.to_string()),
                    ),
                    toolkit: None,
                    url: None,
                },
            )
            .await;
            match resolution {
                Resolution::Approved | Resolution::Answered(_) => None,
                Resolution::Denied(note) => Some(ToolOutput::text(format!(
                    "The user denied this action. Don't retry it; continue without it or ask what they'd prefer.{}",
                    note.map(|note| format!(" They said: {note}")).unwrap_or_default()
                ))),
                Resolution::Expired => Some(ToolOutput::text("The user didn't answer in time, so this was not done. Ask them again later if it still matters.")),
                Resolution::Restarting => Some(ToolOutput::text(format!(
                    "{RESTARTING_MESSAGE} This was not done."
                ))),
            }
        }
        _ => None,
    }
}

async fn wait_for_user(
    state: &Arc<AppState>,
    context: &ToolContext,
    approval: NewApproval,
) -> Resolution {
    approvals::ask_and_wait(
        state,
        &context.dot,
        &context.project_id,
        context.session_id.as_deref(),
        context.call_id.as_deref(),
        approval,
    )
    .await
}

/// Where a sign-in would type: the page the dot's browser shows and the saved login chosen for
/// that page's own host.
struct SignInPlan {
    url: String,
    login: store::SavedLogin,
}

/// Reads the dot's page and picks the saved login for its host. `Err` is the tool's answer when
/// there is nothing to fill. Blocking.
fn plan_sign_in(paths: &GxserverPaths, dot_id: &str, claimed: &str) -> Result<SignInPlan, String> {
    let url = browser::current_url(paths, dot_id).map_err(|_| {
        "Your browser has no page open. Open the site's sign-in page first (open_url), then call sign_in.".to_string()
    })?;
    let parsed = url::Url::parse(&url).ok();
    let host = parsed
        .as_ref()
        .filter(|parsed| parsed.scheme() == "https")
        .and_then(|parsed| parsed.host_str())
        .map(str::to_ascii_lowercase);
    let Some(host) = host else {
        return Err(format!(
            "Your browser is on {}, which is not a secure (https) page. Saved passwords are only typed into https sign-in pages: open the site's https sign-in page first.",
            clip(&url, 120)
        ));
    };
    let db = store::open(paths).map_err(|error| error.message)?;
    let login = store::saved_login_for_host(&db, &host).map_err(|error| error.message)?;
    let Some(login) = login else {
        let claimed_host = store::host_of(claimed);
        let elsewhere = !claimed.trim().is_empty()
            && claimed_host != store::host_of(&host)
            && !host.ends_with(&format!(".{claimed_host}"));
        return Err(if elsewhere {
            format!(
                "Your browser is on {host}, not {claimed_host}. A saved login is only typed into its own site: open {claimed_host}'s sign-in page first."
            )
        } else {
            format!(
                "No saved login for {host} (the page open in your browser). Ask the user to add one under Saved logins in OpenDots Settings (never ask them to paste it in chat), or to take over your browser from the Computer tab and log in themselves."
            )
        });
    };
    Ok(SignInPlan { url, login })
}

/// `sign_in`: the saved login is chosen by the page's real host (not the site the model names),
/// the approval card shows the page's address, and the browser checks the host again inside the
/// page as it fills. The password never appears in what the model reads.
async fn sign_in(state: &Arc<AppState>, context: &ToolContext, args: &Value) -> ToolOutput {
    let dot_id = context.dot.id.clone();
    if browser::is_taken_over(&dot_id) {
        return ToolOutput::text("The user is using your browser right now (they took it over from your Computer tab). Wait until they hand it back, or ask them with ask_user.");
    }
    let planned = {
        let paths = state.paths.clone();
        let dot_id = dot_id.clone();
        let claimed = text_arg(args, "site");
        blocking(move || plan_sign_in(&paths, &dot_id, &claimed)).await
    };
    let plan = match planned {
        Some(Ok(plan)) => plan,
        Some(Err(message)) => return ToolOutput::text(message),
        None => return ToolOutput::error("Error: the sign-in could not be prepared."),
    };
    let host = store::host_of(&plan.url);
    let action = format!("sign in to {host} with the user's saved password");
    let detail = format!(
        "Page: {}\nLogin: {} (saved for {})",
        clip(&plan.url, 300),
        plan.login.username,
        plan.login.site
    );
    if let Some(stop) = check_rules(state, context, "sign_in", &action, "ask", Some(detail)).await {
        return stop;
    }
    let paths = state.paths.clone();
    let filled = blocking(move || -> Result<String, String> {
        let password = vault::unseal(&paths, &plan.login.sealed)
            .map_err(|error| format!("the saved password could not be read ({error})"))?;
        let result = browser::fill_login_for_host(
            &paths,
            &dot_id,
            &plan.login.username,
            &password,
            &plan.login.host,
        );
        // Belt and braces: nothing the model reads may carry the password.
        let scrub = |text: String| {
            if !password.is_empty() && text.contains(password.as_str()) {
                "Filled the login form. Submit it to continue.".to_string()
            } else {
                text
            }
        };
        result.map(scrub).map_err(|error| {
            if !password.is_empty() && error.contains(password.as_str()) {
                "Couldn't fill the login form on this page.".to_string()
            } else {
                error
            }
        })
    })
    .await;
    match filled {
        Some(result) => from_result(result),
        None => ToolOutput::error("Error: the sign-in stopped unexpectedly."),
    }
}

async fn app_connect(state: &Arc<AppState>, context: &ToolContext, args: &Value) -> ToolOutput {
    let toolkit = text_arg(args, "toolkit").to_ascii_lowercase();
    if toolkit.is_empty() {
        return ToolOutput::error("Give the toolkit slug, e.g. gmail.");
    }
    let started = {
        let paths = state.paths.clone();
        let toolkit = toolkit.clone();
        blocking(move || composio::start_connect(&paths, &toolkit)).await
    };
    let started = match started {
        Some(Ok(started)) => started,
        Some(Err(error)) => {
            return ToolOutput::text(format!("Couldn't start connecting {toolkit}: {error}"))
        }
        None => return ToolOutput::error("Couldn't start connecting the app."),
    };
    if started.already_connected {
        return ToolOutput::text("Already connected.");
    }
    let dot_name = context.dot.name.clone();
    let resolution = wait_for_user(
        state,
        context,
        NewApproval {
            kind: "connect",
            title: format!("Connect {}", started.name),
            detail: Some(format!(
                "{dot_name} needs access to your {name} to continue. You'll sign in with {name} directly; {dot_name} never sees your password.",
                name = started.name
            )),
            tool: Some("app_connect".to_string()),
            options: Vec::new(),
            rule_action: None,
            toolkit: Some(toolkit),
            url: started.url,
        },
    )
    .await;
    ToolOutput::text(match resolution {
        Resolution::Approved | Resolution::Answered(_) => "Connected. Continue with the task.",
        _ => "The user chose not to connect this app right now. Continue without it or tell them what you need.",
    })
}

async fn execute(
    state: &Arc<AppState>,
    context: &ToolContext,
    name: &str,
    args: Value,
) -> ToolOutput {
    let dot = context.dot.clone();
    let paths = state.paths.clone();
    match name {
        "message_dot" => {
            let target_name = text_arg(&args, "dot_name");
            let message = text_arg(&args, "message");
            if message.is_empty() {
                return ToolOutput::error("Write the message to send.");
            }
            let found = blocking(move || {
                let db = store::open(&paths).ok()?;
                let dots = store::list_dots(&db).ok()?;
                Some((
                    store::find_dot(&db, &target_name).ok().flatten(),
                    dots.into_iter().map(|dot| dot.name).collect::<Vec<_>>(),
                    target_name,
                ))
            })
            .await
            .flatten();
            let Some((target, names, target_name)) = found else {
                return ToolOutput::error("Your dots could not be read.");
            };
            let Some(target) = target else {
                return ToolOutput::text(format!(
                    "No dot named \"{target_name}\". Available: {}",
                    names.join(", ")
                ));
            };
            if target.id == dot.id {
                return ToolOutput::text("That's you.");
            }
            if target.paused {
                return ToolOutput::text(format!("{} is paused.", target.name));
            }
            from_result(
                handoffs::send_handoff(
                    state,
                    &dot,
                    Some(&context.project_id),
                    context.session_id.as_deref(),
                    &target,
                    &message,
                )
                .await,
            )
        }
        "screenshot" => {
            let dot_id = dot.id.clone();
            match blocking(move || browser::screenshot_png(&paths, &dot_id)).await {
                Some(Ok(png)) => ToolOutput {
                    content: vec![json!({
                        "type": "image",
                        "data": base64::engine::general_purpose::STANDARD.encode(png),
                        "mimeType": "image/png",
                    })],
                    is_error: false,
                },
                Some(Err(error)) => ToolOutput::error(format!("Error: {error}")),
                None => ToolOutput::error("Error: the screenshot failed."),
            }
        }
        _ => {
            let state = state.clone();
            let context = context.clone();
            let name = name.to_string();
            blocking(move || execute_blocking(&state, &context, &name, &args))
                .await
                .unwrap_or_else(|| ToolOutput::error("Error: the tool stopped unexpectedly."))
        }
    }
}

fn store_error(error: DomainStateError) -> ToolOutput {
    ToolOutput::error(format!("Error: {}", error.message))
}

fn execute_blocking(
    state: &AppState,
    context: &ToolContext,
    name: &str,
    args: &Value,
) -> ToolOutput {
    let paths = &state.paths;
    let dot = &context.dot;
    let db = match store::open(paths) {
        Ok(db) => db,
        Err(error) => return store_error(error),
    };
    let changed = |state: &AppState| {
        instructions::refresh_dot(&state.paths, &dot.id);
        publish::publish_changes(state);
    };
    match name {
        "remember" => {
            let fact = text_arg(args, "fact");
            if fact.is_empty() {
                return ToolOutput::error("Give the fact to remember.");
            }
            match store::add_memory(&db, &dot.id, &fact) {
                Ok(memory) => {
                    changed(state);
                    ToolOutput::text(format!("Saved to memory (id {}).", memory.id))
                }
                Err(error) => store_error(error),
            }
        }
        "forget" => {
            let memory_id = text_arg(args, "memory_id");
            match store::get_memory(&db, &memory_id) {
                Ok(Some(memory)) if memory.dot_id == dot.id => {
                    if let Err(error) = store::delete_memory(&db, &memory_id) {
                        return store_error(error);
                    }
                    changed(state);
                    ToolOutput::text("Forgotten.")
                }
                Ok(_) => ToolOutput::text(format!("You have no memory with id {memory_id}.")),
                Err(error) => store_error(error),
            }
        }
        "save_skill" => {
            let skill_name = text_arg(args, "name");
            if skill_name.is_empty() {
                return ToolOutput::error("Name the skill.");
            }
            match store::upsert_skill(
                &db,
                &dot.id,
                &skill_name,
                &text_arg(args, "description"),
                &text_arg(args, "instructions"),
            ) {
                Ok(skill) => {
                    changed(state);
                    ToolOutput::text(format!("Skill \"{}\" saved.", skill.name))
                }
                Err(error) => store_error(error),
            }
        }
        "use_skill" => {
            let skill_name = text_arg(args, "name");
            let skills = store::list_skills(&db, Some(&dot.id)).unwrap_or_default();
            match skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(&skill_name))
            {
                Some(skill) => ToolOutput::text(skill.body.clone()),
                None => ToolOutput::text(format!(
                    "No skill named \"{skill_name}\".{}",
                    if skills.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " Your skills: {}.",
                            skills
                                .iter()
                                .map(|skill| skill.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                )),
            }
        }
        "create_routine" => match routines::add_routine(
            paths,
            &dot.id,
            &text_arg(args, "name"),
            &text_arg(args, "instruction"),
            &text_arg(args, "schedule"),
        ) {
            Ok(routine) => {
                publish::publish_changes(state);
                ToolOutput::text(format!(
                    "Routine created (id {}). Next run: {}.",
                    routine.id,
                    routine
                        .next_run_at
                        .map(cron::describe_time)
                        .unwrap_or_else(|| "unknown".to_string())
                ))
            }
            Err(error) => ToolOutput::text(error.message),
        },
        "delete_routine" => {
            let routine_id = text_arg(args, "routine_id");
            match store::get_routine(&db, &routine_id) {
                Ok(Some(routine)) if routine.dot_id == dot.id => {
                    if let Err(error) = store::delete_routine(&db, &routine_id) {
                        return store_error(error);
                    }
                    changed(state);
                    ToolOutput::text("Routine deleted.")
                }
                Ok(_) => ToolOutput::text(format!("You have no routine with id {routine_id}.")),
                Err(error) => store_error(error),
            }
        }
        "list_routines" => {
            let routines = store::list_routines(&db, Some(&dot.id)).unwrap_or_default();
            if routines.is_empty() {
                ToolOutput::text("You have no routines.")
            } else {
                ToolOutput::text(
                    routines
                        .iter()
                        .map(routine_line)
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
        }
        "send_update" => {
            let text = text_arg(args, "text");
            if text.is_empty() {
                return ToolOutput::error("Write the message.");
            }
            let title = args.get("title").and_then(Value::as_str);
            match updates::post_update(
                state,
                dot,
                &context.project_id,
                context.session_id.as_deref(),
                title,
                &text,
                Vec::new(),
            ) {
                Ok(_) => ToolOutput::text("Delivered to the user."),
                Err(error) => store_error(error),
            }
        }
        "share_file" => {
            let file = match updates::shared_file(dot, &text_arg(args, "path")) {
                Ok(file) => file,
                Err(error) => return ToolOutput::text(error),
            };
            let note = text_arg(args, "note");
            let text = if note.is_empty() {
                format!("Here's {}.", file.name)
            } else {
                note
            };
            let (file_name, size) = (file.name.clone(), file.size);
            match updates::post_update(
                state,
                dot,
                &context.project_id,
                context.session_id.as_deref(),
                None,
                &text,
                vec![file],
            ) {
                Ok(_) => ToolOutput::text(format!(
                    "Shared {file_name} ({size} bytes) with the user. Don't repeat its contents unless asked."
                )),
                Err(error) => store_error(error),
            }
        }
        "open_url" => {
            let url = text_arg(args, "url");
            if url.is_empty() {
                return ToolOutput::error("Give the URL to open.");
            }
            from_result(browser::open_url(paths, &dot.id, &url))
        }
        "read_page" => from_result(browser::read_page(paths, &dot.id)),
        "page_snapshot" => {
            let result = browser::page_snapshot(paths, &dot.id);
            if let Ok(snapshot) = &result {
                remember_snapshot(&dot.id, snapshot);
            }
            from_result(result)
        }
        "click" => {
            let target = match (text_arg(args, "ref"), text_arg(args, "text")) {
                (reference, _) if !reference.is_empty() => {
                    ClickTarget::Ref(normalize_ref(&reference))
                }
                (_, text) if !text.is_empty() => ClickTarget::Text(text),
                _ => {
                    return ToolOutput::error("Give a ref from page_snapshot or the text to click.")
                }
            };
            from_result(browser::click(paths, &dot.id, &target))
        }
        "type_text" => {
            let target = match (text_arg(args, "ref"), text_arg(args, "field")) {
                (reference, _) if !reference.is_empty() => {
                    FieldTarget::Ref(normalize_ref(&reference))
                }
                (_, field) => FieldTarget::Label(field),
            };
            let submit = args.get("submit").and_then(Value::as_bool).unwrap_or(false);
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            from_result(browser::type_text(paths, &dot.id, &target, &text, submit))
        }
        "press_key" => {
            let key = text_arg(args, "key");
            if key.is_empty() {
                return ToolOutput::error("Give the key to press.");
            }
            from_result(browser::press_key(paths, &dot.id, &key))
        }
        "scroll" => {
            let amount = args
                .get("amount")
                .and_then(Value::as_f64)
                .filter(|amount| amount.is_finite() && *amount > 0.0)
                .unwrap_or(600.0)
                .min(20_000.0) as i64;
            let delta = if text_arg(args, "direction").eq_ignore_ascii_case("up") {
                -amount
            } else {
                amount
            };
            from_result(browser::scroll(paths, &dot.id, delta))
        }
        "go_back" => from_result(browser::go_back(paths, &dot.id)),
        // Everything else is one of Composio's tools (checked against its list above).
        composio_tool => match composio::call_tool(paths, composio_tool, args) {
            Ok(text) => ToolOutput::text(text),
            Err(error) => ToolOutput::error(format!("Error: {error}")),
        },
    }
}
