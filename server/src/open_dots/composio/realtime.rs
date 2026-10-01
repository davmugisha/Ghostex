//! Composio's live trigger events, the subscription `@composio/core`'s `triggers.subscribe` opens
//! with pusher-js: realtime credentials from Composio, a Pusher protocol-7 websocket to
//! `ws-<cluster>.pusher.com`, the private channel `private-<projectId>_triggers` signed by
//! Composio's channel-auth endpoint, and `trigger_to_client` events (plus their `chunked-` form).
//! Pings, pongs and Pusher's close codes follow pusher-js; reconnects always wait (at least a
//! second, doubling, with jitter), and a refused API key is retried on a slow backoff.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::Rng;
use serde_json::{json, Map, Value};
use tungstenite::{Message, WebSocket};

use super::rest::{ComposioRest, RestError};

/// What pusher-js 8 sends when it connects.
const PUSHER_CLIENT_QUERY: &str = "protocol=7&client=js&version=8.4.0&flash=false";
const TRIGGER_EVENT: &str = "trigger_to_client";
const CHUNKED_TRIGGER_EVENT: &str = "chunked-trigger_to_client";
/// pusher-js: wait this long for a pong before reconnecting.
const PONG_TIMEOUT: Duration = Duration::from_secs(30);
/// pusher-js's own `activityTimeout` cap.
const MAX_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(120);
/// How often a blocked read wakes to check the stop flag and the ping timer.
const POLL: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The shortest wait before any reconnect, even when Pusher says "reconnect now".
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// A connection that stayed up this long resets the backoff to `MIN_BACKOFF`.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// Pusher 4000-4099 ("don't reconnect"): the app or key is refused; try again much later.
const REFUSED_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// Composio refused the API key: try again after this, doubling up to `BAD_KEY_MAX_BACKOFF`.
const BAD_KEY_BACKOFF: Duration = Duration::from_secs(30);
const BAD_KEY_MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);

type Socket = WebSocket<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

/// What a connection reported to the caller's log.
#[derive(Debug)]
pub(crate) enum Status {
    Subscribed,
    Lost(String),
}

/// One trigger message, normalized the way the SDK's `parsePusherPayload` does.
#[derive(Clone, Debug)]
pub(crate) struct TriggerMessage {
    /// The trigger's ids as Composio may know them: `metadata.id`, `metadata.uuid`, `id`.
    pub(crate) ids: Vec<String>,
    /// The Composio user the trigger belongs to, when the payload says.
    pub(crate) user_id: String,
    pub(crate) payload: Value,
}

struct Credentials {
    key: String,
    cluster: String,
    channel: String,
}

enum Ended {
    Asked,
    BadKey,
    Retry {
        after: Option<Duration>,
        subscribed: bool,
        reason: String,
    },
}

/// Subscribes and keeps the subscription alive until `stop` is set.
pub(crate) fn run(
    rest: &ComposioRest,
    stop: &AtomicBool,
    on_message: &dyn Fn(TriggerMessage),
    on_status: &dyn Fn(Status),
) {
    run_with(rest, stop, &open_socket, on_message, on_status)
}

/// `run` over any websocket `open(cluster, key)` returns (its reads must time out within a second
/// or so, so the stop flag and the ping timer get checked).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Every reconnect waits at least a second (doubling to a minute, plus up to a quarter more at random), and only a connection that stayed up for a minute resets that: a socket that Pusher closes right after subscribing, or with "reconnect now" codes, would otherwise hit Composio's credential and channel-auth endpoints in a tight loop. A refused API key is retried every 30 seconds doubling to 30 minutes instead of stopping, so a brief 401 from Composio does not end trigger events until gxserver restarts.
pub(crate) fn run_with<S: Read + Write>(
    rest: &ComposioRest,
    stop: &AtomicBool,
    open: &dyn Fn(&str, &str) -> Result<WebSocket<S>, String>,
    on_message: &dyn Fn(TriggerMessage),
    on_status: &dyn Fn(Status),
) {
    let mut backoff = MIN_BACKOFF;
    let mut bad_key_backoff = BAD_KEY_BACKOFF;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let started = Instant::now();
        let (wait, reason) = match connection(rest, stop, open, on_message, on_status) {
            Ended::Asked => return,
            Ended::BadKey => {
                let wait = bad_key_backoff;
                bad_key_backoff = (bad_key_backoff * 2).min(BAD_KEY_MAX_BACKOFF);
                (wait, "Composio refused the API key".to_string())
            }
            Ended::Retry {
                after,
                subscribed,
                reason,
            } => {
                if subscribed {
                    bad_key_backoff = BAD_KEY_BACKOFF;
                    if started.elapsed() >= STABLE_AFTER {
                        backoff = MIN_BACKOFF;
                    }
                }
                let wait = match after {
                    Some(after) if !after.is_zero() => after,
                    _ => backoff,
                };
                backoff = (backoff * 2).min(MAX_BACKOFF);
                (wait, reason)
            }
        };
        let wait = with_jitter(wait.max(MIN_BACKOFF));
        on_status(Status::Lost(format!(
            "{reason} (trying again in {}s)",
            wait.as_secs().max(1)
        )));
        if !sleep_unless_stopped(wait, stop) {
            return;
        }
    }
}

/// `wait` plus up to a quarter more at random, so many installs don't reconnect in step.
fn with_jitter(wait: Duration) -> Duration {
    let quarter = (wait.as_millis() / 4) as u64;
    wait + Duration::from_millis(rand::thread_rng().gen_range(0..=quarter))
}

fn sleep_unless_stopped(duration: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250).min(deadline - Instant::now()));
    }
    !stop.load(Ordering::SeqCst)
}

fn credentials(rest: &ComposioRest) -> Result<Credentials, RestError> {
    let value = rest.realtime_credentials()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|text| !text.is_empty())
    };
    match (
        field("pusher_key"),
        field("pusher_cluster"),
        field("project_id"),
    ) {
        (Some(key), Some(cluster), Some(project_id)) => Ok(Credentials {
            key,
            cluster,
            channel: format!("private-{project_id}_triggers"),
        }),
        _ => Err(RestError::Status(
            200,
            "Composio's realtime settings are incomplete".to_string(),
        )),
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|error| error.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Opens `wss://ws-<cluster>.pusher.com/app/<key>?protocol=7…` over rustls.
pub(crate) fn open_socket(cluster: &str, key: &str) -> Result<Socket, String> {
    let host = format!("ws-{cluster}.pusher.com");
    let address = (host.as_str(), 443)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| format!("{host} has no address"))?;
    let tcp =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|error| error.to_string())?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
        .and_then(|_| tcp.set_write_timeout(Some(CONNECT_TIMEOUT)))
        .map_err(|error| error.to_string())?;
    let server_name =
        rustls::pki_types::ServerName::try_from(host.clone()).map_err(|error| error.to_string())?;
    let tls = rustls::ClientConnection::new(tls_config()?, server_name)
        .map_err(|error| error.to_string())?;
    let stream = rustls::StreamOwned::new(tls, tcp);
    let url = format!("wss://{host}/app/{key}?{PUSHER_CLIENT_QUERY}");
    let (socket, _response) =
        tungstenite::client::client(url.as_str(), stream).map_err(|error| error.to_string())?;
    socket
        .get_ref()
        .sock
        .set_read_timeout(Some(POLL))
        .map_err(|error| error.to_string())?;
    Ok(socket)
}

/// A Pusher frame's `data`: a JSON string pusher-js parses, or an object.
pub(crate) fn event_data(message: &Value) -> Value {
    match message.get("data") {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

pub(crate) enum Frame {
    Event(Value),
    Idle,
    Closed(Option<u16>),
}

pub(crate) fn read_frame<S: Read + Write>(socket: &mut WebSocket<S>) -> Result<Frame, String> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(serde_json::from_str::<Value>(&text)
            .map(Frame::Event)
            .unwrap_or(Frame::Idle)),
        Ok(Message::Close(frame)) => Ok(Frame::Closed(frame.map(|frame| u16::from(frame.code)))),
        Ok(_) => Ok(Frame::Idle),
        Err(tungstenite::Error::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(Frame::Idle)
        }
        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
            Ok(Frame::Closed(None))
        }
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn send_event<S: Read + Write>(
    socket: &mut WebSocket<S>,
    event: &str,
    data: Value,
) -> Result<(), String> {
    socket
        .send(Message::Text(
            json!({ "event": event, "data": data }).to_string(),
        ))
        .map_err(|error| error.to_string())
}

/// pusher-js's reading of a Pusher error code: 4000-4099 don't reconnect, 4100-4199 back off,
/// 4200-4299 reconnect now.
fn retry_for_code(code: Option<u16>, subscribed: bool, reason: String) -> Ended {
    let after = match code {
        Some(4000..=4099) => Some(REFUSED_BACKOFF),
        Some(4200..=4299) => Some(Duration::ZERO),
        _ => None,
    };
    Ended::Retry {
        after,
        subscribed,
        reason,
    }
}

/// Reassembles `chunked-trigger_to_client` parts (`{ id, index, chunk, final }`).
#[derive(Default)]
struct Chunks {
    pending: HashMap<String, (Vec<Option<String>>, bool)>,
}

impl Chunks {
    fn add(&mut self, data: &Value) -> Option<Value> {
        let id = data.get("id")?.as_str()?.to_string();
        let index = usize::try_from(data.get("index")?.as_u64()?).ok()?;
        if index > 10_000 {
            return None;
        }
        let chunk = data
            .get("chunk")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if self.pending.len() > 64 && !self.pending.contains_key(&id) {
            self.pending.clear();
        }
        let (parts, received_final) = self.pending.entry(id.clone()).or_default();
        if parts.len() <= index {
            parts.resize(index + 1, None);
        }
        parts[index] = Some(chunk);
        if data.get("final").and_then(Value::as_bool) == Some(true) {
            *received_final = true;
        }
        if !*received_final || parts.iter().any(Option::is_none) {
            return None;
        }
        let (parts, _) = self.pending.remove(&id)?;
        let joined = parts.into_iter().flatten().collect::<String>();
        serde_json::from_str(&joined).ok()
    }
}

fn connection<S: Read + Write>(
    rest: &ComposioRest,
    stop: &AtomicBool,
    open: &dyn Fn(&str, &str) -> Result<WebSocket<S>, String>,
    on_message: &dyn Fn(TriggerMessage),
    on_status: &dyn Fn(Status),
) -> Ended {
    let retry = |reason: String| Ended::Retry {
        after: None,
        subscribed: false,
        reason,
    };
    let credentials = match credentials(rest) {
        Ok(credentials) => credentials,
        Err(RestError::BadKey(_)) => return Ended::BadKey,
        Err(error) => return retry(error.message()),
    };
    let mut socket = match open(&credentials.cluster, &credentials.key) {
        Ok(socket) => socket,
        Err(error) => return retry(format!("Couldn't open the live connection: {error}")),
    };

    // pusher:connection_established carries the socket id the channel signature is for.
    let started = Instant::now();
    let (socket_id, activity_timeout) = loop {
        if stop.load(Ordering::SeqCst) {
            let _ = socket.close(None);
            return Ended::Asked;
        }
        if started.elapsed() > CONNECT_TIMEOUT {
            return retry("Pusher didn't confirm the connection".to_string());
        }
        match read_frame(&mut socket) {
            Ok(Frame::Event(message)) => {
                let event = message
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let data = event_data(&message);
                if event == "pusher:connection_established" {
                    let Some(socket_id) = data.get("socket_id").and_then(Value::as_str) else {
                        return retry("Pusher sent no socket id".to_string());
                    };
                    let activity = data
                        .get("activity_timeout")
                        .and_then(Value::as_u64)
                        .map(Duration::from_secs)
                        .unwrap_or(MAX_ACTIVITY_TIMEOUT)
                        .min(MAX_ACTIVITY_TIMEOUT);
                    break (socket_id.to_string(), activity);
                }
                if event == "pusher:error" {
                    let code = data
                        .get("code")
                        .and_then(Value::as_u64)
                        .map(|code| code as u16);
                    let text = data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("error");
                    return retry_for_code(
                        code,
                        false,
                        format!("Pusher refused the connection: {text}"),
                    );
                }
            }
            Ok(Frame::Idle) => {}
            Ok(Frame::Closed(code)) => {
                return retry_for_code(code, false, "Pusher closed the connection".to_string())
            }
            Err(error) => return retry(error),
        }
    };

    let auth = match rest.realtime_channel_auth(&socket_id, &credentials.channel) {
        Ok(value) => value,
        Err(RestError::BadKey(_)) => {
            let _ = socket.close(None);
            return Ended::BadKey;
        }
        Err(error) => {
            let _ = socket.close(None);
            return retry(error.message());
        }
    };
    let Some(signature) = auth.get("auth").and_then(Value::as_str) else {
        let _ = socket.close(None);
        return retry("Composio didn't sign the trigger channel".to_string());
    };
    let mut subscribe = json!({ "auth": signature, "channel": credentials.channel });
    if let Some(channel_data) = auth.get("channel_data").and_then(Value::as_str) {
        subscribe["channel_data"] = Value::String(channel_data.to_string());
    }
    if let Err(error) = send_event(&mut socket, "pusher:subscribe", subscribe) {
        return retry(error);
    }

    let mut subscribed = false;
    let mut last_activity = Instant::now();
    let mut ping_sent_at: Option<Instant> = None;
    let mut chunks = Chunks::default();
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = send_event(
                &mut socket,
                "pusher:unsubscribe",
                json!({ "channel": credentials.channel }),
            );
            let _ = socket.close(None);
            return Ended::Asked;
        }
        let frame = match read_frame(&mut socket) {
            Ok(frame) => frame,
            Err(error) => {
                return Ended::Retry {
                    after: None,
                    subscribed,
                    reason: format!("The live connection broke: {error}"),
                }
            }
        };
        match frame {
            Frame::Idle => {
                if let Some(sent_at) = ping_sent_at {
                    if sent_at.elapsed() > PONG_TIMEOUT {
                        return Ended::Retry {
                            after: Some(Duration::ZERO),
                            subscribed,
                            reason: "Pusher stopped answering".to_string(),
                        };
                    }
                } else if last_activity.elapsed() >= activity_timeout {
                    if let Err(error) = send_event(&mut socket, "pusher:ping", json!({})) {
                        return Ended::Retry {
                            after: None,
                            subscribed,
                            reason: error,
                        };
                    }
                    ping_sent_at = Some(Instant::now());
                }
            }
            Frame::Closed(code) => {
                return retry_for_code(code, subscribed, "Pusher closed the connection".to_string())
            }
            Frame::Event(message) => {
                last_activity = Instant::now();
                ping_sent_at = None;
                let event = message
                    .get("event")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let channel = message.get("channel").and_then(Value::as_str);
                match event {
                    "pusher:ping" => {
                        if let Err(error) = send_event(&mut socket, "pusher:pong", json!({})) {
                            return Ended::Retry {
                                after: None,
                                subscribed,
                                reason: error,
                            };
                        }
                    }
                    "pusher_internal:subscription_succeeded" => {
                        subscribed = true;
                        on_status(Status::Subscribed);
                    }
                    "pusher:subscription_error" => {
                        let data = event_data(&message);
                        let status = data.get("status").and_then(Value::as_u64);
                        if matches!(status, Some(401 | 403)) {
                            let _ = socket.close(None);
                            return Ended::BadKey;
                        }
                        let _ = socket.close(None);
                        return Ended::Retry {
                            after: None,
                            subscribed,
                            reason: format!("Pusher refused the trigger channel: {data}"),
                        };
                    }
                    "pusher:error" => {
                        let data = event_data(&message);
                        let code = data
                            .get("code")
                            .and_then(Value::as_u64)
                            .map(|code| code as u16);
                        if code.is_some() {
                            let _ = socket.close(None);
                            let text = data
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("error");
                            return retry_for_code(
                                code,
                                subscribed,
                                format!("Pusher error: {text}"),
                            );
                        }
                    }
                    TRIGGER_EVENT if channel == Some(credentials.channel.as_str()) => {
                        let data = event_data(&message);
                        if data.is_object() {
                            on_message(normalize(&data));
                        }
                    }
                    CHUNKED_TRIGGER_EVENT if channel == Some(credentials.channel.as_str()) => {
                        if let Some(Value::Object(object)) = chunks.add(&event_data(&message)) {
                            on_message(normalize(&Value::Object(object)));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn push_id(ids: &mut Vec<String>, id: Option<&str>) {
    if let Some(id) = id.filter(|id| !id.is_empty() && *id != "unknown") {
        if !ids.iter().any(|known| known == id) {
            ids.push(id.to_string());
        }
    }
}

/// The SDK's `parsePusherPayload`: webhook V3, V2, V1, then the legacy `TriggerData` shape, then a
/// best-effort read of whatever came.
pub(crate) fn normalize(data: &Value) -> TriggerMessage {
    let object = |key: &str| data.get(key).filter(|value| value.is_object());
    // V3: { id, timestamp, type: "composio.…", metadata, data }.
    if text(data, "id").is_some()
        && text(data, "timestamp").is_some()
        && text(data, "type").is_some_and(|kind| kind.starts_with("composio."))
        && object("metadata").is_some()
        && object("data").is_some()
    {
        let metadata = &data["metadata"];
        let trigger_fields = [
            "log_id",
            "trigger_slug",
            "trigger_id",
            "connected_account_id",
            "auth_config_id",
            "user_id",
        ];
        let mut ids = Vec::new();
        if trigger_fields
            .iter()
            .all(|field| text(metadata, field).is_some())
        {
            push_id(&mut ids, text(metadata, "trigger_id"));
            return TriggerMessage {
                ids,
                user_id: text(metadata, "user_id").unwrap_or_default().to_string(),
                payload: data["data"].clone(),
            };
        }
        push_id(&mut ids, text(data, "id"));
        return TriggerMessage {
            ids,
            user_id: String::new(),
            payload: data["data"].clone(),
        };
    }
    // V2: { type, timestamp, log_id, data: { connection_id, connection_nano_id, trigger_nano_id, trigger_id, user_id, … } }.
    if let Some(inner) = object("data") {
        let v2_fields = [
            "connection_id",
            "connection_nano_id",
            "trigger_nano_id",
            "trigger_id",
            "user_id",
        ];
        if text(data, "type").is_some()
            && text(data, "timestamp").is_some()
            && text(data, "log_id").is_some()
            && v2_fields.iter().all(|field| text(inner, field).is_some())
        {
            let mut ids = Vec::new();
            push_id(&mut ids, text(inner, "trigger_nano_id"));
            push_id(&mut ids, text(inner, "trigger_id"));
            let mut payload = inner.as_object().cloned().unwrap_or_default();
            for field in v2_fields {
                payload.remove(field);
            }
            return TriggerMessage {
                ids,
                user_id: text(inner, "user_id").unwrap_or_default().to_string(),
                payload: Value::Object(payload),
            };
        }
    }
    // V1: { trigger_name, connection_id, trigger_id, payload, log_id }.
    if ["trigger_name", "connection_id", "trigger_id", "log_id"]
        .iter()
        .all(|field| text(data, field).is_some())
        && object("payload").is_some()
    {
        let mut ids = Vec::new();
        push_id(&mut ids, text(data, "trigger_id"));
        return TriggerMessage {
            ids,
            user_id: String::new(),
            payload: data["payload"].clone(),
        };
    }
    // Legacy TriggerData: { appName, payload, originalPayload, metadata: { id, nanoId, connection: { clientUniqueUserId } } }.
    if let Some(metadata) = object("metadata") {
        if text(metadata, "nanoId").is_some() && text(data, "appName").is_some() {
            let mut ids = Vec::new();
            push_id(&mut ids, text(metadata, "nanoId"));
            push_id(&mut ids, text(metadata, "id"));
            let payload = data
                .get("payload")
                .filter(|payload| !payload.is_null())
                .or_else(|| data.get("originalPayload"))
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));
            return TriggerMessage {
                ids,
                user_id: metadata
                    .pointer("/connection/clientUniqueUserId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                payload,
            };
        }
    }
    let mut ids = Vec::new();
    push_id(
        &mut ids,
        text(data, "id").or_else(|| text(data, "trigger_id")),
    );
    push_id(&mut ids, text(data, "uuid").or_else(|| text(data, "id")));
    TriggerMessage {
        ids,
        user_id: text(data, "userId").unwrap_or_default().to_string(),
        payload: data
            .get("payload")
            .filter(|payload| payload.is_object())
            .cloned()
            .unwrap_or_else(|| data.clone()),
    }
}
