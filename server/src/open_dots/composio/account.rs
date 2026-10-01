//! The Composio For You sign-in held by gxserver (Open Dot's `Provider` + `connect`/`ensureClient`):
//! the sealed `composio_oauth` setting, the live MCP session, and the order of the OAuth steps
//! (renew the token, else hand back an authorization URL).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use subtle::ConstantTimeEq;

use crate::open_dots::{get_setting, open_open_dots_database, set_setting, vault};
use crate::paths::GxserverPaths;

use super::mcp_client::{self, McpError, McpSession};
use super::oauth::{self, BearerChallenge, ClientInfo, Discovery, OAuthError, Tokens};

/// Composio For You's hosted MCP server.
pub(crate) const MCP_URL: &str = "https://connect.composio.dev/mcp";
/// The sealed JSON of the client registration, tokens, PKCE verifier, discovery and state.
const OAUTH_SETTING: &str = "composio_oauth";

pub(crate) const NOT_CONNECTED: &str = "Composio isn't connected. Sign in from OpenDots settings.";
pub(crate) const EXPIRED: &str =
    "Your Composio sign-in expired. Sign in again from OpenDots settings.";
const NO_SIGN_IN: &str =
    "No Composio sign-in is in progress. Start it again from OpenDots settings.";
pub(super) const STALE_LINK: &str =
    "This sign-in link is out of date. Start the Composio sign-in again from OpenDots settings.";
/// How long a started sign-in waits for Composio's redirect.
const SIGN_IN_TTL_MS: i64 = 10 * 60 * 1000;
const DIDNT_COMPLETE: &str = "Composio sign-in didn't complete. Try again.";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client: Option<ClientInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    discovery: Option<Discovery>,
    /// The OAuth `state` this sign-in sent; the redirect must bring it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    /// When `state` was issued (ms); the redirect is accepted for 10 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state_issued_at: Option<i64>,
    /// The redirect URI the authorization request used; the code exchange must repeat it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redirect_uri: Option<String>,
}

/// What this process knows right now (Open Dot's `globalThis.__dotsComposio`).
pub(super) struct Live {
    root: Option<PathBuf>,
    /// Cache of the sealed setting; `None` until read. Only this module writes the setting.
    stored: Option<Stored>,
    pub(super) session: Option<McpSession>,
    pub(super) tools: Vec<Value>,
    /// App slugs connected in the user's Composio account.
    pub(super) connected: Vec<String>,
    /// `tools/list` succeeded in this process.
    pub(super) listed: bool,
    /// When `tools/list` was last tried, so a Composio outage isn't retried on every read.
    pub(super) list_attempted_at: Option<std::time::Instant>,
    pub(super) background_refresh_running: bool,
}

impl Live {
    const fn empty() -> Self {
        Live {
            root: None,
            stored: None,
            session: None,
            tools: Vec::new(),
            connected: Vec::new(),
            listed: false,
            list_attempted_at: None,
            background_refresh_running: false,
        }
    }
}

static LIVE: Mutex<Live> = Mutex::new(Live::empty());
/// Serializes connecting, renewing and finishing a sign-in.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Composio's sign-in server (WorkOS AuthKit) rotates refresh tokens, so two threads renewing at once would spend the same refresh token twice and the loser would sign the user out.
static AUTH_LOCK: Mutex<()> = Mutex::new(());

type ChangeListener = Arc<dyn Fn() + Send + Sync>;
static CHANGE_LISTENER: Mutex<Option<ChangeListener>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `f` on this gxserver's live state (a different state root starts clean).
pub(super) fn with_live<R>(paths: &GxserverPaths, f: impl FnOnce(&mut Live) -> R) -> R {
    let mut live = lock(&LIVE);
    if live.root.as_deref() != Some(paths.root_dir.as_path()) {
        *live = Live::empty();
        live.root = Some(paths.root_dir.clone());
    }
    f(&mut live)
}

/// Called after the sign-in, the tool list or the connected apps change, so clients refetch.
pub(crate) fn set_changed_listener(listener: Arc<dyn Fn() + Send + Sync>) {
    *lock(&CHANGE_LISTENER) = Some(listener);
}

pub(super) fn notify_changed() {
    let listener = lock(&CHANGE_LISTENER).clone();
    if let Some(listener) = listener {
        listener();
    }
}

fn read_stored(paths: &GxserverPaths) -> Result<Stored, String> {
    let db = open_open_dots_database(paths).map_err(|error| error.to_string())?;
    let Some(raw) = get_setting(&db, OAUTH_SETTING).map_err(|error| error.to_string())? else {
        return Ok(Stored::default());
    };
    let plain = vault::unseal(paths, &raw)?;
    Ok(serde_json::from_str(&plain).unwrap_or_default())
}

/// The saved sign-in. An unreadable one reads as signed out (Open Dot's `load`) and is not cached,
/// so a locked keychain is retried next time.
fn load(paths: &GxserverPaths) -> Stored {
    if let Some(stored) = with_live(paths, |live| live.stored.clone()) {
        return stored;
    }
    match read_stored(paths) {
        Ok(stored) => {
            with_live(paths, |live| live.stored = Some(stored.clone()));
            stored
        }
        Err(_) => Stored::default(),
    }
}

fn save(paths: &GxserverPaths, stored: &Stored) -> Result<(), String> {
    let plain = serde_json::to_string(stored).map_err(|error| error.to_string())?;
    let sealed = vault::seal(paths, &plain)
        .map_err(|error| format!("Couldn't save the Composio sign-in: {error}"))?;
    let db = open_open_dots_database(paths).map_err(|error| error.to_string())?;
    set_setting(&db, OAUTH_SETTING, Some(&sealed)).map_err(|error| error.to_string())?;
    with_live(paths, |live| live.stored = Some(stored.clone()));
    Ok(())
}

/// Signed in: a live session, or saved tokens (Open Dot's `signedIn`).
pub(super) fn signed_in(paths: &GxserverPaths) -> bool {
    with_live(paths, |live| live.session.is_some()) || load(paths).tokens.is_some()
}

pub(super) fn access_token(paths: &GxserverPaths) -> Option<String> {
    load(paths).tokens.map(|tokens| tokens.access_token)
}

pub(super) fn drop_session(paths: &GxserverPaths) {
    with_live(paths, |live| live.session = None);
}

pub(super) enum Connected {
    Yes,
    /// Open this URL in the user's browser to sign in.
    SignInAt(String),
}

enum Renewal {
    Renewed,
    /// Composio refused the refresh token (or there is none): a new sign-in is needed.
    Refused,
    /// The renewal didn't get an answer.
    Unavailable(String),
}

fn ensure_discovery(
    paths: &GxserverPaths,
    stored: &mut Stored,
    challenge: Option<&BearerChallenge>,
) -> Result<Discovery, String> {
    if let Some(discovery) = &stored.discovery {
        return Ok(discovery.clone());
    }
    let discovery = oauth::discover(
        MCP_URL,
        challenge.and_then(|challenge| challenge.resource_metadata_url.as_deref()),
    )?;
    stored.discovery = Some(discovery.clone());
    save(paths, stored)?;
    Ok(discovery)
}

/// The SDK's refresh branch of `auth()`, with its recovery: `invalid_grant` forgets the tokens,
/// `invalid_client` forgets everything.
fn renew(
    paths: &GxserverPaths,
    stored: &mut Stored,
    challenge: Option<&BearerChallenge>,
) -> Result<Renewal, String> {
    let (Some(refresh_token), Some(client)) = (
        stored
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.refresh_token.clone()),
        stored.client.clone(),
    ) else {
        return Ok(Renewal::Refused);
    };
    let discovery = match ensure_discovery(paths, stored, challenge) {
        Ok(discovery) => discovery,
        Err(error) => return Ok(Renewal::Unavailable(error)),
    };
    let resource = oauth::select_resource(MCP_URL, &discovery)?;
    match oauth::refresh_tokens(&discovery, &client, &refresh_token, resource.as_deref()) {
        Ok(tokens) => {
            stored.tokens = Some(tokens);
            save(paths, stored)?;
            Ok(Renewal::Renewed)
        }
        Err(OAuthError::InvalidGrant(_)) => {
            stored.tokens = None;
            save(paths, stored)?;
            Ok(Renewal::Refused)
        }
        Err(OAuthError::InvalidClient(_)) => {
            *stored = Stored::default();
            save(paths, stored)?;
            Ok(Renewal::Refused)
        }
        Err(OAuthError::Other(message)) => Ok(Renewal::Unavailable(message)),
    }
}

/// Registers (again when the redirect changed), then builds the authorization URL and saves the
/// PKCE verifier and state for the redirect.
fn start_authorization(
    paths: &GxserverPaths,
    stored: &mut Stored,
    challenge: &BearerChallenge,
    redirect_uri: &str,
) -> Result<String, String> {
    let discovery = ensure_discovery(paths, stored, Some(challenge))?;
    let scope = oauth::resolve_scope(challenge.scope.as_deref(), &discovery);
    let resource = oauth::select_resource(MCP_URL, &discovery)?;
    let client = match &stored.client {
        Some(client)
            if client.redirect_uris.is_empty()
                || client.redirect_uris.iter().any(|uri| uri == redirect_uri) =>
        {
            client.clone()
        }
        _ => {
            let client = oauth::register_client(&discovery, redirect_uri, scope.as_deref())?;
            stored.client = Some(client.clone());
            client
        }
    };
    let state = oauth::random_token();
    let (url, verifier) = oauth::authorization_url(
        &discovery,
        &client,
        redirect_uri,
        scope.as_deref(),
        &state,
        resource.as_deref(),
    )?;
    stored.verifier = Some(verifier);
    stored.state = Some(state);
    stored.state_issued_at = Some(oauth::now_ms());
    stored.redirect_uri = Some(redirect_uri.to_string());
    save(paths, stored)?;
    Ok(url)
}

/// Connects with the saved sign-in (Open Dot's `connect`). A token that ran out is renewed first;
/// `failed_token` is an access token a request was just refused with, renewed unless another
/// thread already did. With a `redirect_uri` a missing or refused sign-in returns the URL to sign
/// in at; without one it is an error.
pub(super) fn connect(
    paths: &GxserverPaths,
    redirect_uri: Option<&str>,
    failed_token: Option<&str>,
) -> Result<Connected, String> {
    let _guard = lock(&AUTH_LOCK);
    let mut stored = load(paths);
    let had_tokens = stored.tokens.is_some();
    if !had_tokens && redirect_uri.is_none() {
        return Err(NOT_CONNECTED.to_string());
    }
    let renewed_elsewhere = failed_token.is_some_and(|failed| {
        stored
            .tokens
            .as_ref()
            .map(|tokens| tokens.access_token.as_str())
            != Some(failed)
    });
    if (failed_token.is_none() || renewed_elsewhere)
        && with_live(paths, |live| live.session.is_some())
    {
        return Ok(Connected::Yes);
    }
    let mut renewed_once = false;
    let mut unavailable: Option<String> = None;
    let token_is_stale = stored.tokens.as_ref().is_some_and(|tokens| {
        tokens.expired(oauth::now_ms()) || Some(tokens.access_token.as_str()) == failed_token
    });
    if token_is_stale {
        renewed_once = true;
        if let Renewal::Unavailable(message) = renew(paths, &mut stored, None)? {
            unavailable = Some(message);
        }
    }
    loop {
        let token = stored
            .tokens
            .as_ref()
            .map(|tokens| tokens.access_token.clone());
        match mcp_client::initialize(MCP_URL, token.as_deref(), oauth::CLIENT_NAME) {
            Ok(session) => {
                with_live(paths, |live| live.session = Some(session));
                return Ok(Connected::Yes);
            }
            Err(McpError::Unauthorized(challenge)) => {
                if !renewed_once {
                    renewed_once = true;
                    match renew(paths, &mut stored, Some(&challenge))? {
                        Renewal::Renewed => continue,
                        Renewal::Refused => {}
                        Renewal::Unavailable(message) => unavailable = Some(message),
                    }
                }
                let Some(redirect_uri) = redirect_uri else {
                    if let Some(message) = unavailable {
                        return Err(format!("Couldn't renew the Composio sign-in: {message}"));
                    }
                    return Err(if had_tokens { EXPIRED } else { NOT_CONNECTED }.to_string());
                };
                return start_authorization(paths, &mut stored, &challenge, redirect_uri)
                    .map(Connected::SignInAt);
            }
            Err(error) => return Err(error.message()),
        }
    }
}

/// The OAuth redirect landed with `code` (Open Dot's `finishSignIn`). `state` is checked against
/// the one this sign-in sent; `None` skips the check for trusted in-process callers.
pub(super) fn finish(paths: &GxserverPaths, code: &str, state: Option<&str>) -> Result<(), String> {
    {
        let _guard = lock(&AUTH_LOCK);
        let mut stored = load(paths);
        let (Some(verifier), Some(client), Some(redirect_uri)) = (
            stored.verifier.clone(),
            stored.client.clone(),
            stored.redirect_uri.clone(),
        ) else {
            return Err(NO_SIGN_IN.to_string());
        };
        if state.is_some_and(|state| !state_matches(&stored, state)) {
            return Err(STALE_LINK.to_string());
        }
        let discovery = ensure_discovery(paths, &mut stored, None)?;
        let resource = oauth::select_resource(MCP_URL, &discovery)?;
        let tokens = oauth::exchange_code(
            &discovery,
            &client,
            code,
            &verifier,
            &redirect_uri,
            resource.as_deref(),
        )
        .map_err(|error| format!("Composio didn't accept the sign-in: {}", error.message()))?;
        stored.tokens = Some(tokens);
        stored.verifier = None;
        stored.state = None;
        stored.state_issued_at = None;
        save(paths, &stored)?;
        drop_session(paths);
    }
    match connect(paths, None, None) {
        Ok(_) => Ok(()),
        Err(error) if error == EXPIRED || error == NOT_CONNECTED => Err(DIDNT_COMPLETE.to_string()),
        Err(error) => Err(error),
    }
}

/// The redirect brought back the `state` of the sign-in in progress, issued less than 10 minutes
/// ago (constant-time compare).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The redirect route needs no gxserver token, so any website can send the user's browser to it. Only a request carrying the live, unexpired state may change anything or tell clients to refetch; a bogus one must not cancel the real sign-in either.
fn state_matches(stored: &Stored, state: &str) -> bool {
    let expected = stored.state.as_deref().unwrap_or_default();
    let fresh = stored
        .state_issued_at
        .is_some_and(|issued| (oauth::now_ms() - issued).abs() <= SIGN_IN_TTL_MS);
    fresh && !expected.is_empty() && bool::from(expected.as_bytes().ct_eq(state.as_bytes()))
}

/// `state` belongs to the sign-in in progress and has not expired.
pub(super) fn pending_state_matches(paths: &GxserverPaths, state: &str) -> bool {
    state_matches(&load(paths), state)
}

/// The user declined at Composio: forget the sign-in in progress when `state` is its own.
pub(super) fn cancel_pending(paths: &GxserverPaths, state: &str) {
    let _guard = lock(&AUTH_LOCK);
    let mut stored = load(paths);
    if state_matches(&stored, state) {
        stored.verifier = None;
        stored.state = None;
        stored.state_issued_at = None;
        let _ = save(paths, &stored);
    }
}

/// Forgets the sign-in and everything learned with it (Open Dot's `signOut`).
pub(super) fn sign_out(paths: &GxserverPaths) {
    let _guard = lock(&AUTH_LOCK);
    with_live(paths, |live| {
        live.session = None;
        live.tools.clear();
        live.connected.clear();
        live.listed = false;
        live.stored = Some(Stored::default());
    });
    if let Ok(db) = open_open_dots_database(paths) {
        let _ = set_setting(&db, OAUTH_SETTING, None);
    }
}

/// A Composio request that failed. `NotRun` stopped before the request (no sign-in, a refused
/// renewal, a failed connect); `Unreached` is the request itself failing to connect (DNS, connect,
/// TLS handshake), so a retry can't repeat a side effect; `Failed` may have run.
pub(super) enum RpcFailure {
    NotRun(String),
    Unreached(String),
    Failed(String),
}

impl RpcFailure {
    pub(super) fn message(&self) -> &str {
        match self {
            RpcFailure::NotRun(message)
            | RpcFailure::Unreached(message)
            | RpcFailure::Failed(message) => message,
        }
    }
}

/// One JSON-RPC request in the live session, connecting first, renewing the token on a 401 and
/// starting a new session when Composio dropped the old one.
pub(super) fn rpc(
    paths: &GxserverPaths,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, RpcFailure> {
    let mut failed_token: Option<String> = None;
    for _ in 0..3 {
        connect(paths, None, failed_token.as_deref()).map_err(RpcFailure::NotRun)?;
        let Some(session) = with_live(paths, |live| live.session.clone()) else {
            continue;
        };
        let token = access_token(paths);
        match mcp_client::request(
            MCP_URL,
            token.as_deref(),
            &session,
            method,
            params.clone(),
            timeout,
        ) {
            Ok(result) => return Ok(result),
            Err(McpError::Unauthorized(_)) => {
                drop_session(paths);
                failed_token = Some(token.unwrap_or_default());
            }
            Err(McpError::SessionGone) => {
                drop_session(paths);
                failed_token = None;
            }
            Err(error @ McpError::Unreached(_)) => {
                drop_session(paths);
                return Err(RpcFailure::Unreached(error.message()));
            }
            Err(error) => return Err(RpcFailure::Failed(error.message())),
        }
    }
    Err(RpcFailure::NotRun(EXPIRED.to_string()))
}
