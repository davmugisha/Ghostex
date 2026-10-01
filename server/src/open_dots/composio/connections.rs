//! Connecting one of the user's apps to Composio For You (Open Dot's `isConnected` and
//! `startConnect`): `COMPOSIO_MANAGE_CONNECTIONS` lists the app's status or returns its sign-in link.

use serde_json::json;

use crate::paths::GxserverPaths;

use super::account::{self, with_live};
use super::http::clip;
use super::names::pretty_name;
use super::tools::call_tool;
use super::ComposioConnectStart;

const MANAGE_CONNECTIONS: &str = "COMPOSIO_MANAGE_CONNECTIONS";

/// `"status"\s*:\s*"ACTIVE"`, case-insensitive.
fn has_active_status(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(found) = rest.find("\"status\"") {
        let after = rest[found + "\"status\"".len()..].trim_start();
        if let Some(value) = after.strip_prefix(':') {
            if value.trim_start().starts_with("\"active\"") {
                return true;
            }
        }
        rest = &rest[found + 1..];
    }
    false
}

/// The sign-in link in the tool output: `"redirect_url": "…"` (JSON-unescaped), else the first
/// `https://` URL.
fn sign_in_link(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(found) = rest.find("\"redirect_url\"") {
        let after = rest[found + "\"redirect_url\"".len()..].trim_start();
        if let Some(value) = after.strip_prefix(':').map(str::trim_start) {
            if let Some(quoted) = value.strip_prefix('"') {
                let mut end = None;
                let mut escaped = false;
                for (index, c) in quoted.char_indices() {
                    match c {
                        '\\' if !escaped => escaped = true,
                        '"' if !escaped => {
                            end = Some(index);
                            break;
                        }
                        _ => escaped = false,
                    }
                }
                if let Some(end) = end.filter(|end| *end > 0) {
                    let raw = &quoted[..end];
                    let url = serde_json::from_str::<String>(&format!("\"{raw}\""))
                        .unwrap_or_else(|_| raw.to_string());
                    return Some(url);
                }
            }
        }
        rest = &rest[found + 1..];
    }
    let start = text.find("https://")?;
    let url = text[start..]
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']'))
        .next()?;
    Some(url.to_string())
}

/// True when the app is connected in the user's Composio account. A failed check reads as not
/// connected.
pub(crate) fn is_connected(paths: &GxserverPaths, toolkit: &str) -> bool {
    let toolkit = toolkit.trim().to_lowercase();
    if with_live(paths, |live| live.connected.contains(&toolkit)) {
        return true;
    }
    let Ok(output) = call_tool(
        paths,
        MANAGE_CONNECTIONS,
        &json!({ "toolkits": [{ "name": toolkit, "action": "list" }] }),
    ) else {
        return false;
    };
    let active = has_active_status(&output);
    if active {
        let added = with_live(paths, |live| {
            if live.connected.contains(&toolkit) {
                false
            } else {
                live.connected.push(toolkit.clone());
                true
            }
        });
        if added {
            account::notify_changed();
        }
    }
    active
}

/// Creates a connect link for an app; `already_connected` when it is live.
pub(crate) fn start_connect(
    paths: &GxserverPaths,
    toolkit: &str,
) -> Result<ComposioConnectStart, String> {
    let toolkit = toolkit.trim().to_lowercase();
    if toolkit.is_empty() {
        return Err("Name the app to connect, like gmail or slack.".to_string());
    }
    let name = pretty_name(&toolkit);
    if is_connected(paths, &toolkit) {
        return Ok(ComposioConnectStart {
            already_connected: true,
            name,
            url: None,
        });
    }
    let output = call_tool(
        paths,
        MANAGE_CONNECTIONS,
        &json!({ "toolkits": [{ "name": toolkit, "action": "add" }] }),
    )?;
    let url = sign_in_link(&output).ok_or_else(|| {
        format!(
            "Composio didn't return a sign-in link for {name}: {}",
            clip(&output, 300)
        )
    })?;
    Ok(ComposioConnectStart {
        already_connected: false,
        name,
        url: Some(url),
    })
}
