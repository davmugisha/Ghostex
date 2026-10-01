//! A small blocking Chrome DevTools Protocol client for one page target, over tungstenite (the
//! same shape as `ghostex_cli/browser_mcp`'s client, without its console capture). Events are
//! dropped: the dot's browser only ever waits on call replies.

use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// How long one ordinary CDP call may take.
pub(super) const CALL_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct Cdp {
    socket: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    closed: bool,
}

impl Cdp {
    pub(super) fn connect(ws_url: &str) -> Result<Cdp, String> {
        let rest = ws_url
            .strip_prefix("ws://")
            .ok_or_else(|| "The browser gave an unexpected DevTools address.".to_string())?;
        let host_port = rest.split(['/', '?']).next().unwrap_or(rest);
        let address = std::net::ToSocketAddrs::to_socket_addrs(host_port)
            .map_err(|error| format!("Couldn't reach the dot's browser: {error}"))?
            .next()
            .ok_or_else(|| "Couldn't reach the dot's browser.".to_string())?;
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .map_err(|error| format!("Couldn't reach the dot's browser: {error}"))?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_nodelay(true);
        let (socket, _response) = tungstenite::client::client(ws_url, stream)
            .map_err(|error| format!("Couldn't connect to the dot's browser: {error}"))?;
        Ok(Cdp {
            socket,
            next_id: 1,
            closed: false,
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(super) fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call_with_timeout(method, params, CALL_TIMEOUT)
    }

    /// One request/reply. A reply that arrives after `timeout` is skipped by the next call, which
    /// only accepts its own id.
    pub(super) fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.closed {
            return Err("The connection to the dot's browser closed.".to_string());
        }
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({ "id": id, "method": method, "params": params }).to_string();
        if let Err(error) = self.socket.send(tungstenite::Message::Text(payload)) {
            self.closed = true;
            return Err(format!(
                "The connection to the dot's browser broke: {error}"
            ));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "The dot's browser did not answer in time ({method})."
                ));
            }
            let _ = self
                .socket
                .get_mut()
                .set_read_timeout(Some((deadline - now).max(Duration::from_millis(1))));
            match self.socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let Ok(message) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = message.get("error").filter(|error| !error.is_null()) {
                        let text = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error");
                        return Err(format!("The dot's browser refused {method}: {text}"));
                    }
                    return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
                }
                Ok(tungstenite::Message::Close(_)) => {
                    self.closed = true;
                    return Err("The connection to the dot's browser closed.".to_string());
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Err(format!(
                        "The dot's browser did not answer in time ({method})."
                    ));
                }
                Err(_) => {
                    self.closed = true;
                    return Err("The connection to the dot's browser closed.".to_string());
                }
            }
        }
    }

    pub(super) fn close(&mut self) {
        if !self.closed {
            let _ = self.socket.close(None);
            self.closed = true;
        }
    }
}

/// `GET http://127.0.0.1:<port><path>` as JSON (the DevTools HTTP endpoints).
pub(super) fn http_json(port: u16, path: &str, timeout: Duration) -> Result<Value, String> {
    http_request("GET", port, path, timeout)
}

/// `PUT` variant: Chrome only opens new tabs (`/json/new`) through PUT.
pub(super) fn http_put_json(port: u16, path: &str, timeout: Duration) -> Result<Value, String> {
    http_request("PUT", port, path, timeout)
}

fn http_request(method: &str, port: u16, path: &str, timeout: Duration) -> Result<Value, String> {
    let url = format!("http://127.0.0.1:{port}{path}");
    match ureq::request(method, &url).timeout(timeout).call() {
        Ok(response) => response
            .into_json::<Value>()
            .map_err(|error| format!("The dot's browser answered oddly: {error}")),
        Err(ureq::Error::Status(code, _)) => {
            Err(format!("The dot's browser answered HTTP {code}."))
        }
        Err(error) => Err(format!("Couldn't reach the dot's browser: {error}")),
    }
}
