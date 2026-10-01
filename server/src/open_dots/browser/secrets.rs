//! The last saved password each dot's browser filled, kept in memory only, so every browser tool's
//! text (page text, snapshots, titles, URLs, errors) can be scrubbed of it before the model sees it.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::session::lock;

/// What replaces the password wherever it shows up.
const HIDDEN: &str = "(hidden)";
/// Shorter passwords are only hidden field by field: replacing every "abc" in a page would garble it.
const MIN_SCRUB_LEN: usize = 4;

fn filled() -> &'static Mutex<HashMap<String, String>> {
    static FILLED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    FILLED.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn remember(dot_id: &str, password: &str) {
    if password.is_empty() {
        return;
    }
    lock(filled()).insert(dot_id.to_string(), password.to_string());
}

pub(super) fn forget(dot_id: &str) {
    lock(filled()).remove(dot_id);
}

fn secret(dot_id: &str) -> Option<String> {
    lock(filled()).get(dot_id).cloned()
}

/// The forms the password can take in page text or a URL: as typed, with whitespace collapsed (the
/// snapshot cleans labels that way), and URL-encoded (a login form sent with GET, or a page's
/// `encodeURIComponent`).
fn variants(password: &str) -> Vec<String> {
    let mut list = vec![password.to_string()];
    let collapsed = password.split_whitespace().collect::<Vec<_>>().join(" ");
    let form: String = url::form_urlencoded::byte_serialize(password.as_bytes()).collect();
    let percent = form.replace('+', "%20");
    for variant in [collapsed, form, percent, uri_component(password)] {
        if !variant.is_empty() && !list.contains(&variant) {
            list.push(variant);
        }
    }
    // Longest first, so a longer encoded form is replaced before a shorter one inside it.
    list.sort_by_key(|variant| std::cmp::Reverse(variant.len()));
    list
}

/// JavaScript's `encodeURIComponent`: everything but letters, digits and `-_.!~*'()` as `%XX`.
fn uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `text` with the dot's last filled password (in any of its forms) replaced by "(hidden)".
pub(super) fn scrub(dot_id: &str, text: String) -> String {
    let Some(password) = secret(dot_id) else {
        return text;
    };
    if password.chars().count() < MIN_SCRUB_LEN {
        return text;
    }
    let mut text = text;
    for variant in variants(&password) {
        if text.contains(&variant) {
            text = text.replace(&variant, HIDDEN);
        }
    }
    text
}

/// `text` holds the dot's last filled password in one of its forms.
pub(super) fn contains_secret(dot_id: &str, text: &str) -> bool {
    scrub(dot_id, text.to_string()) != text
}

pub(super) fn scrub_result(dot_id: &str, result: Result<String, String>) -> Result<String, String> {
    match result {
        Ok(text) => Ok(scrub(dot_id, text)),
        Err(error) => Err(scrub(dot_id, error)),
    }
}

/// A field value that holds the dot's last filled password, whole or cut short by the snapshot's
/// 120-character clip: the field is reported as "(filled)" instead of showing it.
pub(super) fn is_secret_value(dot_id: &str, value: &str) -> bool {
    let Some(password) = secret(dot_id) else {
        return false;
    };
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let collapsed = password.split_whitespace().collect::<Vec<_>>().join(" ");
    if password.chars().count() < MIN_SCRUB_LEN {
        return value == password || value == collapsed;
    }
    value.contains(password.as_str())
        || value.contains(collapsed.as_str())
        || (value.chars().count() >= CLIPPED_VALUE_LEN
            && (password.starts_with(value) || collapsed.starts_with(value)))
}

/// The snapshot clips field values to 120 characters; a value this long may be a clipped password.
const CLIPPED_VALUE_LEN: usize = 100;
