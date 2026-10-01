//! Composio's REST API with a project API key (`x-api-key`), the calls `@composio/core` 0.18 makes
//! through `@composio/client` for triggers: trigger types, trigger instances, auth configs,
//! connected accounts and connect links (`/api/v3.1/…`), plus the realtime credentials and channel
//! auth its trigger subscription uses (`/api/v3/internal/sdk/realtime/…`).

use std::time::Duration;

use serde_json::{json, Value};

use super::http::{agent, read_json, send, HttpError};

const DEFAULT_BASE_URL: &str = "https://backend.composio.dev";
const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const BAD_KEY: &str =
    "Composio didn't accept the API key. Check it in OpenDots settings.";

/// A REST failure, already worded for the user.
#[derive(Debug)]
pub(crate) enum RestError {
    /// 401/403: the key is wrong or was revoked.
    BadKey(String),
    /// Composio answered with another error status.
    Status(u16, String),
    /// No answer.
    Unreached(String),
}

impl RestError {
    pub(crate) fn message(&self) -> String {
        match self {
            RestError::BadKey(_) => BAD_KEY.to_string(),
            RestError::Status(_, message) => format!("Composio: {message}"),
            RestError::Unreached(message) => format!("Couldn't reach Composio: {message}"),
        }
    }

    /// The raw text, for the key check's own wording.
    pub(crate) fn detail(&self) -> String {
        match self {
            RestError::BadKey(message)
            | RestError::Status(_, message)
            | RestError::Unreached(message) => message.clone(),
        }
    }

    fn from_http(error: HttpError) -> Self {
        match error.status() {
            Some(401 | 403) => RestError::BadKey(error.message()),
            Some(status) => RestError::Status(status, error.message()),
            None => RestError::Unreached(error.message()),
        }
    }
}

/// A client for one API key.
#[derive(Clone)]
pub(crate) struct ComposioRest {
    base_url: String,
    api_key: String,
}

/// `@composio/client` serializes array query values comma-separated.
fn comma(values: &[&str]) -> String {
    values.join(",")
}

impl ComposioRest {
    /// `COMPOSIO_BASE_URL` overrides the API host, as in the SDK.
    pub(crate) fn new(api_key: &str) -> Self {
        let base_url = std::env::var("COMPOSIO_BASE_URL")
            .ok()
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        ComposioRest {
            base_url,
            api_key: api_key.to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        agent()
            .request(method, &self.url(path))
            .set("x-api-key", &self.api_key)
            .set("Accept", "application/json")
            .timeout(TIMEOUT)
    }

    fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, RestError> {
        let mut request = self.request("GET", path);
        for (key, value) in query {
            request = request.query(key, value);
        }
        let response = send(request.call()).map_err(RestError::from_http)?;
        read_json(response).map_err(RestError::from_http)
    }

    fn send_json(&self, method: &str, path: &str, body: &Value) -> Result<Value, RestError> {
        let response = send(
            self.request(method, path)
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
        )
        .map_err(RestError::from_http)?;
        read_json(response).map_err(RestError::from_http)
    }

    /// `triggers.listTypes({ toolkits, limit })`.
    pub(crate) fn list_trigger_types(
        &self,
        toolkit: Option<&str>,
        limit: u32,
    ) -> Result<Value, RestError> {
        let mut query = vec![
            ("limit", limit.to_string()),
            ("toolkit_versions", "latest".to_string()),
        ];
        if let Some(toolkit) = toolkit {
            query.push(("toolkit_slugs", comma(&[toolkit])));
        }
        self.get("/api/v3.1/triggers_types", &query)
    }

    /// `triggers.getType(slug)`; the SDK checks the slug this way before creating a trigger.
    pub(crate) fn get_trigger_type(&self, slug: &str) -> Result<Value, RestError> {
        self.get(
            &format!("/api/v3.1/triggers_types/{}", encode_segment(slug)),
            &[("toolkit_versions", "latest".to_string())],
        )
    }

    /// `connectedAccounts.list({ userIds, authConfigIds?, statuses: ["ACTIVE"], limit })`.
    pub(crate) fn list_active_accounts(
        &self,
        user_id: &str,
        auth_config_id: Option<&str>,
    ) -> Result<Value, RestError> {
        let mut query = vec![
            ("user_ids", comma(&[user_id])),
            ("statuses", comma(&["ACTIVE"])),
            ("limit", "100".to_string()),
        ];
        if let Some(auth_config_id) = auth_config_id {
            query.push(("auth_config_ids", comma(&[auth_config_id])));
        }
        self.get("/api/v3.1/connected_accounts", &query)
    }

    /// `authConfigs.list({ toolkit })`.
    pub(crate) fn list_auth_configs(&self, toolkit: &str) -> Result<Value, RestError> {
        self.get(
            "/api/v3.1/auth_configs",
            &[("toolkit_slug", toolkit.to_string())],
        )
    }

    /// `authConfigs.create(toolkit, { type: "use_composio_managed_auth", name })`; returns the id.
    pub(crate) fn create_managed_auth_config(
        &self,
        toolkit: &str,
        name: &str,
    ) -> Result<String, RestError> {
        let value = self.send_json(
            "POST",
            "/api/v3.1/auth_configs",
            &json!({
                "toolkit": { "slug": toolkit },
                "auth_config": { "type": "use_composio_managed_auth", "name": name },
            }),
        )?;
        value
            .pointer("/auth_config/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| RestError::Status(200, "no auth config id in the answer".to_string()))
    }

    /// `connectedAccounts.link(userId, authConfigId)`: `{ redirect_url, connected_account_id }`.
    pub(crate) fn create_link(
        &self,
        auth_config_id: &str,
        user_id: &str,
    ) -> Result<Value, RestError> {
        self.send_json(
            "POST",
            "/api/v3.1/connected_accounts/link",
            &json!({ "auth_config_id": auth_config_id, "user_id": user_id }),
        )
    }

    /// `triggers.create(userId, slug, { triggerConfig })`; returns the trigger id.
    pub(crate) fn upsert_trigger(
        &self,
        slug: &str,
        user_id: &str,
        config: &Value,
    ) -> Result<String, RestError> {
        let value = self.send_json(
            "POST",
            &format!(
                "/api/v3.1/trigger_instances/{}/upsert",
                encode_segment(slug)
            ),
            &json!({
                "trigger_config": config,
                "toolkit_versions": "latest",
                "user_id": user_id,
            }),
        )?;
        value
            .get("trigger_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| RestError::Status(200, "no trigger id in the answer".to_string()))
    }

    /// `triggers.enable` / `triggers.disable`.
    pub(crate) fn set_trigger_status(
        &self,
        trigger_id: &str,
        enabled: bool,
    ) -> Result<(), RestError> {
        self.send_json(
            "PATCH",
            &format!(
                "/api/v3.1/trigger_instances/manage/{}",
                encode_segment(trigger_id)
            ),
            &json!({ "status": if enabled { "enable" } else { "disable" } }),
        )
        .map(drop)
    }

    /// `triggers.delete`.
    pub(crate) fn delete_trigger(&self, trigger_id: &str) -> Result<(), RestError> {
        let response = send(
            self.request(
                "DELETE",
                &format!(
                    "/api/v3.1/trigger_instances/manage/{}",
                    encode_segment(trigger_id)
                ),
            )
            .call(),
        )
        .map_err(RestError::from_http)?;
        drop(response);
        Ok(())
    }

    /// `InternalService.getSDKRealtimeCredentials`: `{ pusher_key, project_id, pusher_cluster }`.
    pub(crate) fn realtime_credentials(&self) -> Result<Value, RestError> {
        self.get("/api/v3/internal/sdk/realtime/credentials", &[])
    }

    /// The private-channel signature pusher-js's `ajax` channel authorization asks for:
    /// a form POST of `socket_id` and `channel_name`, answered with `{ auth }`.
    pub(crate) fn realtime_channel_auth(
        &self,
        socket_id: &str,
        channel_name: &str,
    ) -> Result<Value, RestError> {
        let response = send(
            self.request("POST", "/api/v3/internal/sdk/realtime/auth")
                .send_form(&[("socket_id", socket_id), ("channel_name", channel_name)]),
        )
        .map_err(RestError::from_http)?;
        read_json(response).map_err(RestError::from_http)
    }
}

/// Percent-encodes one path segment (slugs and ids are plain, but never trust them in a URL).
fn encode_segment(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
