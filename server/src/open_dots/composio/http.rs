//! The blocking HTTP plumbing every Composio call shares: one `ureq` agent, a status-or-transport
//! error, and the readable message Composio puts in an error body.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

const USER_AGENT: &str = "Ghostex-OpenDots/1.0";

/// One agent for every Composio host: keeps TLS connections alive between calls.
pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
    })
}

/// A failed request: the server answered with an error status, or the request never got an answer.
#[derive(Debug)]
pub(crate) enum HttpError {
    Status {
        status: u16,
        /// `WWW-Authenticate`, kept for the MCP OAuth discovery.
        www_authenticate: Option<String>,
        body: String,
    },
    /// No connection was made (DNS, connect, TLS handshake), so the server never saw the request.
    Unreached(String),
    /// The connection broke after the request may have been sent, or the answer was unreadable.
    Transport(String),
}

impl HttpError {
    pub(crate) fn status(&self) -> Option<u16> {
        match self {
            HttpError::Status { status, .. } => Some(*status),
            HttpError::Unreached(_) | HttpError::Transport(_) => None,
        }
    }

    /// The readable part of the error: Composio's `error.message` / OAuth's `error_description`
    /// when the body carries one, else a short status line.
    pub(crate) fn message(&self) -> String {
        match self {
            HttpError::Status { status, body, .. } => {
                error_message_from_body(body).unwrap_or_else(|| format!("HTTP {status}"))
            }
            HttpError::Unreached(message) | HttpError::Transport(message) => message.clone(),
        }
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// Sends a request and turns ureq's error statuses into `HttpError::Status` with the body read.
pub(crate) fn send(
    result: Result<ureq::Response, ureq::Error>,
) -> Result<ureq::Response, HttpError> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            let www_authenticate = response.header("www-authenticate").map(str::to_string);
            let body = response.into_string().unwrap_or_default();
            Err(HttpError::Status {
                status,
                www_authenticate,
                body,
            })
        }
        Err(ureq::Error::Transport(transport)) => {
            let unreached = matches!(
                transport.kind(),
                ureq::ErrorKind::Dns
                    | ureq::ErrorKind::ConnectionFailed
                    | ureq::ErrorKind::ProxyConnect
            );
            let message = transport.to_string();
            Err(if unreached {
                HttpError::Unreached(message)
            } else {
                HttpError::Transport(message)
            })
        }
    }
}

/// Reads a JSON body; an empty body is `null`.
pub(crate) fn read_json(response: ureq::Response) -> Result<Value, HttpError> {
    let text = response
        .into_string()
        .map_err(|error| HttpError::Transport(error.to_string()))?;
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| {
        HttpError::Transport(format!(
            "Composio sent an unreadable answer: {}",
            clip(&text, 200)
        ))
    })
}

/// `{"error":{"message":…}}` (Composio REST), `{"error":"…","error_description":"…"}` (OAuth),
/// `{"message":…}` or `{"error":"…"}`.
pub(crate) fn error_message_from_body(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    text(value.pointer("/error/message"))
        .or_else(|| text(value.get("error_description")))
        .or_else(|| text(value.get("message")))
        .or_else(|| text(value.get("error")))
        .or_else(|| text(value.get("reason")))
}

/// Clips to `max` chars (Open Dot's `clip`).
pub(crate) fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((index, _)) => format!("{}…[truncated]", &text[..index]),
        None => text.to_string(),
    }
}
