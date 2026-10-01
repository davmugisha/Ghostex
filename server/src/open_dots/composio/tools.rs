//! Composio's hosted MCP tools for the dots: the tool list, the connected apps it reveals, and
//! calling a tool (Open Dot's `refresh`, `apps`, `mcpTools`, `callTool`).

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::paths::GxserverPaths;

use super::account::{self, with_live, RpcFailure};
use super::http::clip;
use super::names::{logo_url, pretty_name};
use super::SUGGESTED_APPS;

/// Tools the dots never see (Open Dot's `HIDDEN_TOOLS`): Composio's sandbox shell and workbench,
/// its skill and feedback tools, and the blocking wait the connect card replaces.
const HIDDEN_TOOLS: &[&str] = &[
    "REMOTE_BASH",
    "REMOTE_WORKBENCH",
    "SKILL",
    "SUBMIT_FEEDBACK",
    "WAIT_FOR_CONNECTIONS",
];
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
const CALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const OUTPUT_LIMIT: usize = 30_000;
/// How long a failed `tools/list` waits before a read tries again on its own.
const LIST_RETRY_AFTER: Duration = Duration::from_secs(60);

fn hidden(name: &str) -> bool {
    HIDDEN_TOOLS.iter().any(|hidden| name.contains(hidden))
}

/// "…connected the apps: gmail, slack. …" in `COMPOSIO_SEARCH_TOOLS`' description (Open Dot's
/// `/connected the apps:\s*([^.\n]+)/i`).
fn connected_apps_from_description(description: &str) -> Option<Vec<String>> {
    let lower = description.to_ascii_lowercase();
    let marker = "connected the apps:";
    let start = lower.find(marker)? + marker.len();
    let rest = description[start..].trim_start();
    let end = rest.find(['.', '\n']).unwrap_or(rest.len());
    let listed = &rest[..end];
    if listed.is_empty() {
        return None;
    }
    Some(
        listed
            .split(',')
            .map(|slug| slug.trim().to_lowercase())
            .filter(|slug| !slug.is_empty())
            .collect(),
    )
}

/// Re-lists Composio's tools; their descriptions also say which apps the user connected.
pub(crate) fn refresh(paths: &GxserverPaths) -> Result<(), String> {
    with_live(paths, |live| live.list_attempted_at = Some(Instant::now()));
    let mut tools: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let params = match &cursor {
            Some(cursor) => json!({ "cursor": cursor }),
            None => json!({}),
        };
        let result = account::rpc(paths, "tools/list", params, LIST_TIMEOUT)
            .map_err(|failure| failure.message().to_string())?;
        if let Some(page) = result.get("tools").and_then(Value::as_array) {
            tools.extend(page.iter().cloned());
        }
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    let listed = tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some("COMPOSIO_SEARCH_TOOLS"))
        .and_then(|tool| tool.get("description").and_then(Value::as_str))
        .and_then(connected_apps_from_description);
    let visible = tools
        .into_iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !hidden(name))
        })
        .collect::<Vec<_>>();
    with_live(paths, |live| {
        live.tools = visible;
        live.listed = true;
        if let Some(listed) = listed {
            live.connected = listed;
        }
    });
    account::notify_changed();
    Ok(())
}

/// True when a read may list the tools on its own: signed in, never listed, and no failed try in
/// the last minute.
fn list_due(paths: &GxserverPaths) -> bool {
    account::signed_in(paths)
        && with_live(paths, |live| {
            !live.listed
                && live
                    .list_attempted_at
                    .is_none_or(|at| at.elapsed() >= LIST_RETRY_AFTER)
        })
}

/// Composio's hosted MCP tools the dots get while signed in: `{ name, description, inputSchema }`.
/// Lists them first when this process hasn't yet.
pub(crate) fn mcp_tools(paths: &GxserverPaths) -> Vec<Value> {
    if !account::signed_in(paths) {
        return Vec::new();
    }
    if list_due(paths) {
        let _ = refresh(paths);
    }
    with_live(paths, |live| {
        live.tools
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(Value::as_str)?;
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(name);
                let input_schema = tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object" }));
                Some(json!({
                    "name": name,
                    "description": description,
                    "inputSchema": input_schema,
                }))
            })
            .collect()
    })
}

/// The app list for Settings and the Home strip: connected apps first, then the suggestions.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The snapshot reads this every few seconds, so it never waits on Composio: the first read after a restart lists the tools on a background thread and the change event refetches.
pub(crate) fn apps(paths: &GxserverPaths) -> Vec<Value> {
    if list_due(paths) {
        let start = with_live(paths, |live| {
            !std::mem::replace(&mut live.background_refresh_running, true)
        });
        if start {
            let paths = paths.clone();
            std::thread::spawn(move || {
                let _ = refresh(&paths);
                with_live(&paths, |live| live.background_refresh_running = false);
            });
        }
    }
    let connected = with_live(paths, |live| live.connected.clone());
    connected
        .iter()
        .map(String::as_str)
        .chain(
            SUGGESTED_APPS
                .iter()
                .copied()
                .filter(|slug| !connected.iter().any(|connected| connected == slug)),
        )
        .map(|slug| {
            json!({
                "slug": slug,
                "name": pretty_name(slug),
                "logo": logo_url(slug),
                "connected": connected.iter().any(|connected| connected == slug),
            })
        })
        .collect()
}

/// `text` parts as text, anything else as `[type]`; "(no output)" when empty; clipped; "Error: "
/// first when the tool failed.
fn tool_result_text(result: &Value) -> String {
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|content| {
            content
                .iter()
                .map(|part| {
                    let kind = part.get("type").and_then(Value::as_str).unwrap_or_default();
                    if kind == "text" {
                        part.get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    } else {
                        format!("[{kind}]")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let text = if text.is_empty() {
        "(no output)".to_string()
    } else {
        text
    };
    let is_error = result.get("isError").and_then(Value::as_bool) == Some(true);
    format!(
        "{}{}",
        if is_error { "Error: " } else { "" },
        clip(&text, OUTPUT_LIMIT)
    )
}

/// Calls one of Composio's tools (5 minute limit).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Open Dot retried any failed call once after reconnecting; here the one retry happens only when the call itself could not connect (DNS, connect or TLS handshake failed), so a send, post or delete whose connection broke after the request went out is never run twice.
pub(crate) fn call_tool(
    paths: &GxserverPaths,
    name: &str,
    arguments: &Value,
) -> Result<String, String> {
    let run = || {
        account::rpc(
            paths,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            CALL_TIMEOUT,
        )
        .map(|result| tool_result_text(&result))
    };
    match run() {
        Ok(text) => Ok(text),
        Err(RpcFailure::Unreached(_)) => {
            account::drop_session(paths);
            run().map_err(|failure| failure.message().to_string())
        }
        Err(failure) => Err(failure.message().to_string()),
    }
}
