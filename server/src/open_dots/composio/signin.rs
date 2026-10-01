//! Signing in to and out of Composio For You (Open Dot's `signIn`, `finishSignIn`, `signOut`).

use crate::paths::GxserverPaths;

use super::account::{self, with_live, Connected};
use super::oauth_route::redirect_uri;
use super::tools;

/// Signed in to Composio For You (tokens saved).
pub(crate) fn signed_in(paths: &GxserverPaths) -> bool {
    account::signed_in(paths)
}

/// Starts the sign-in: the authorization URL to open in the user's browser, or `None` when already
/// signed in. `redirect_base` is gxserver's own `http://127.0.0.1:<port>` origin.
pub(crate) fn sign_in(
    paths: &GxserverPaths,
    redirect_base: &str,
) -> Result<Option<String>, String> {
    if with_live(paths, |live| live.session.is_some()) {
        return Ok(None);
    }
    match account::connect(paths, Some(&redirect_uri(redirect_base)), None)? {
        Connected::SignInAt(url) => Ok(Some(url)),
        Connected::Yes => {
            let listed = tools::refresh(paths);
            account::notify_changed();
            listed.map(|_| None)
        }
    }
}

fn finish(paths: &GxserverPaths, code: &str, state: Option<&str>) -> Result<(), String> {
    account::finish(paths, code, state)?;
    let listed = tools::refresh(paths);
    account::notify_changed();
    listed
}

/// The OAuth redirect landed with `code`, without the `state` check (for in-process callers; the
/// GET route `/open-dots/composio/oauth` uses `finish_sign_in_checked`).
#[allow(dead_code)]
pub(crate) fn finish_sign_in(paths: &GxserverPaths, code: &str) -> Result<(), String> {
    finish(paths, code, None)
}

/// `finish_sign_in` for the browser redirect: `state` must be the one this sign-in issued.
pub(crate) fn finish_sign_in_checked(
    paths: &GxserverPaths,
    code: &str,
    state: &str,
) -> Result<(), String> {
    finish(paths, code, Some(state))
}

pub(crate) fn sign_out(paths: &GxserverPaths) {
    account::sign_out(paths);
    account::notify_changed();
}
