//! The user's apps through Composio, ported from Open Dot's `composio.ts` (Composio For You: an
//! OAuth sign-in held by gxserver and Composio's hosted MCP server, whose tools are proxied to the
//! dots with Open Dot's read/write approval rule) and `triggers.ts` (Composio project API key,
//! trigger types, trigger creation and the live event subscription).
//!
//! The tool executor and the RPC methods call the functions re-exported here. All of them are
//! blocking (call from `spawn_blocking`); the OAuth redirect route is the one async entry.

use std::sync::Arc;

use serde_json::Value;

mod account;
mod approvals;
mod connections;
mod http;
mod mcp_client;
mod names;
mod oauth;
mod oauth_route;
mod realtime;
mod rest;
mod signin;
mod tools;
mod triggers;

pub(crate) use account::set_changed_listener;
pub(crate) use approvals::{describe_execute, execute_decision, execute_detail};
pub(crate) use connections::{is_connected, start_connect};
pub(crate) use oauth_route::{serve_oauth_redirect, OAUTH_REDIRECT_PATH};
// `finish_sign_in` stays for in-process callers; the browser redirect uses the state-checking
// `finish_sign_in_checked` inside `oauth_route`.
#[allow(unused_imports)]
pub(crate) use signin::finish_sign_in;
pub(crate) use signin::{sign_in, sign_out, signed_in};
pub(crate) use tools::{apps, call_tool, mcp_tools, refresh};
pub(crate) use triggers::{
    connect_trigger_app, create_trigger, delete_trigger, save_triggers_key, set_trigger_enabled,
    start_trigger_events, stop_trigger_events, trigger_apps, trigger_types, triggers_key_source,
};

/// Apps suggested before the user connected any (Open Dot's SUGGESTED).
pub(crate) const SUGGESTED_APPS: &[&str] = &[
    "gmail",
    "googlecalendar",
    "slack",
    "notion",
    "github",
    "googledrive",
    "linear",
    "outlook",
];

/// Open Dot's rule decision for a proxied call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposioDecision {
    Allow,
    Ask,
}

/// A started app connection: the sign-in link to show on the connect card.
#[derive(Clone, Debug)]
pub(crate) struct ComposioConnectStart {
    pub(crate) already_connected: bool,
    pub(crate) name: String,
    pub(crate) url: Option<String>,
}

/// One trigger event delivered by the live subscription.
#[derive(Clone, Debug)]
pub(crate) struct ComposioTriggerEvent {
    /// The Composio trigger id (`metadata.id` / `metadata.uuid` / `id`).
    pub(crate) composio_ids: Vec<String>,
    pub(crate) payload: Value,
}

pub(crate) type TriggerEventSink = Arc<dyn Fn(ComposioTriggerEvent) + Send + Sync>;
