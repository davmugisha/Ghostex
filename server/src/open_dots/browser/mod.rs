//! Each dot's own browser: a headless Google Chrome with a persistent profile per dot, driven by
//! gxserver over the DevTools protocol, ported from Open Dot's `computer/browser.ts` and
//! `computer/dom-actions.ts`. The Computer tab polls `live_screenshot`, and "Take over" forwards the
//! user's input with `user_input`; the dot's own tools refuse while the user drives.
//!
//! Every signature below is what the tool executor and the RPC methods call. All functions are
//! blocking (call from `spawn_blocking`); different dots run in parallel, and one dot's Computer
//! tab keeps updating while its own tool call waits on a slow page.
//!
//! Every text a browser tool returns is scrubbed of the last password `sign_in` filled for that
//! dot (`secrets`), so a page that echoes it, or a "Show password" button, cannot hand it to the
//! model.

mod cdp;
mod chrome;
mod input;
mod scripts;
mod secrets;
mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::paths::GxserverPaths;

use session::{
    browser_for, drain_browsers, existing_browser, read_last_url, remove_browser, string_field,
    DotBrowser, Lane, Page, RefEntry, Start,
};

/// The page size the dot's browser renders at (Open Dot's SCREEN).
pub(crate) const SCREEN_WIDTH: u32 = 1280;
pub(crate) const SCREEN_HEIGHT: u32 = 800;

/// How many elements `page_snapshot` lists (the ones in view first).
const SNAPSHOT_LIMIT: u64 = 200;
/// Open Dot's `read_page` clip.
const READ_PAGE_LIMIT: usize = 15_000;
/// Open Dot's `goto` timeout.
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(45);

/// What `browserScreenshot` returns.
#[derive(Clone, Debug, Default)]
pub(crate) struct BrowserShot {
    /// JPEG bytes, `None` when no browser runs and `wake` was false.
    pub(crate) jpeg: Option<Vec<u8>>,
    pub(crate) url: String,
    pub(crate) title: String,
    /// The browser is running.
    pub(crate) live: bool,
    pub(crate) taken_over: bool,
}

/// A click target: an element ref from `page_snapshot`, or visible text.
#[derive(Clone, Debug)]
pub(crate) enum ClickTarget {
    Ref(String),
    Text(String),
}

/// A field target: an element ref from `page_snapshot`, or a label/placeholder/name.
#[derive(Clone, Debug)]
pub(crate) enum FieldTarget {
    Ref(String),
    Label(String),
}

/// The installed Chrome the dots use, if any.
pub(crate) fn chrome_path() -> Option<std::path::PathBuf> {
    chrome::find_chrome()
}

/// Opens a URL in the dot's browser (starting it if needed). Returns
/// `Opened <url> ("<title>").` A URL without a scheme gets `https://`.
pub(crate) fn open_url(paths: &GxserverPaths, dot_id: &str, url: &str) -> Result<String, String> {
    secrets::scrub_result(dot_id, open_url_inner(paths, dot_id, url))
}

fn open_url_inner(paths: &GxserverPaths, dot_id: &str, url: &str) -> Result<String, String> {
    let target = normalize_tool_url(url)?;
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Launch)?;
    session::navigate(page.cdp(), &target, NAVIGATION_TIMEOUT)?;
    std::thread::sleep(Duration::from_millis(800));
    let (url, title) = page_place(&mut page);
    Ok(format!("Opened {}.", place(&url, &title)))
}

/// The visible text of the current page, prefixed with its URL and title (≤ 15k chars).
pub(crate) fn read_page(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    secrets::scrub_result(dot_id, read_page_inner(paths, dot_id))
}

fn read_page_inner(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let text = page.run(scripts::PAGE_TEXT, json!([]))?;
    let text = collapse_blank_lines(text.as_str().unwrap_or_default());
    let clipped: String = text.chars().take(READ_PAGE_LIMIT).collect();
    let (url, title) = page_place(&mut page);
    Ok(format!(
        "URL: {}\nTitle: {title}\n\n{clipped}",
        display_url(&url)
    ))
}

/// An outline of the page's interactive elements with refs (`[e12] button "Continue"`), the ones
/// in view first. The refs work with `click` and `type_text` until the next snapshot.
pub(crate) fn page_snapshot(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    secrets::scrub_result(dot_id, page_snapshot_inner(paths, dot_id))
}

fn page_snapshot_inner(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let snapshot = page.run(scripts::SNAPSHOT, json!([SNAPSHOT_LIMIT]))?;
    let url = string_field(&snapshot, "url");
    let title = string_field(&snapshot, "title");
    page.remember_url(&url);
    let elements = snapshot
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut refs = HashMap::new();
    let mut lines = Vec::with_capacity(elements.len());
    for element in &elements {
        let reference = string_field(element, "ref");
        let label = string_field(element, "label");
        refs.insert(
            reference.clone(),
            RefEntry {
                selector: string_field(element, "selector"),
                url: url.clone(),
            },
        );
        lines.push(snapshot_line(dot_id, &reference, element, &label));
    }
    page.set_refs(refs);
    let mut out = format!("URL: {}\nTitle: {title}\n\n", display_url(&url));
    if lines.is_empty() {
        out.push_str("No buttons, links or fields on this page. Read the page to see its text.");
        return Ok(out);
    }
    out.push_str("Interactive elements (pass a ref to click or type_text):\n");
    out.push_str(&lines.join("\n"));
    let total = snapshot.get("total").and_then(Value::as_u64).unwrap_or(0);
    if total > elements.len() as u64 {
        out.push_str(&format!(
            "\n\n(Showing {} of {total}. Scroll and take another snapshot for the rest.)",
            elements.len()
        ));
    }
    Ok(out)
}

fn snapshot_line(dot_id: &str, reference: &str, element: &Value, label: &str) -> String {
    let value = string_field(element, "value");
    // A field holding the filled password (a "Show password" button made it a text field and
    // dropped the page's own marker) reads as a filled password field, never its value.
    let holds_secret = secrets::is_secret_value(dot_id, &value);
    let role = if holds_secret {
        "password".to_string()
    } else {
        string_field(element, "role")
    };
    let mut line = format!("[{reference}] {role} \"{label}\"");
    if !value.is_empty() && !holds_secret {
        line.push_str(&format!(" value=\"{value}\""));
    }
    let filled = if holds_secret {
        Some(true)
    } else {
        element.get("filled").and_then(Value::as_bool)
    };
    match filled {
        Some(true) => line.push_str(" (filled)"),
        Some(false) => line.push_str(" (empty)"),
        None => {}
    }
    match element.get("checked").and_then(Value::as_bool) {
        Some(true) => line.push_str(" checked"),
        Some(false) => line.push_str(" unchecked"),
        None => {}
    }
    if element.get("disabled").and_then(Value::as_bool) == Some(true) {
        line.push_str(" disabled");
    }
    let href = string_field(element, "href");
    if !href.is_empty() {
        line.push_str(&format!(" -> {href}"));
    }
    if element.get("inView").and_then(Value::as_bool) == Some(false) {
        line.push_str(" (off screen)");
    }
    line
}

pub(crate) fn click(
    paths: &GxserverPaths,
    dot_id: &str,
    target: &ClickTarget,
) -> Result<String, String> {
    secrets::scrub_result(dot_id, click_inner(paths, dot_id, target))
}

/// What `CLICK_TARGET` is given for a target: mode, query and the snapshot's selector fallback.
fn click_query(page: &Page<'_>, target: &ClickTarget) -> (&'static str, String, Option<String>) {
    match target {
        ClickTarget::Ref(reference) => {
            let reference = normalize_ref(reference);
            let fallback = page.ref_entry(&reference).map(|entry| entry.selector);
            ("ref", reference, fallback)
        }
        ClickTarget::Text(text) => ("text", text.trim().to_string(), None),
    }
}

fn click_inner(
    paths: &GxserverPaths,
    dot_id: &str,
    target: &ClickTarget,
) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let (mode, query, fallback) = click_query(&page, target);
    let found = page.run(scripts::CLICK_TARGET, json!([mode, query, fallback, false]))?;
    if found.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(script_error(&found, "Couldn't click that."));
    }
    if found.get("hit").and_then(Value::as_bool) == Some(true) {
        let x = found.get("x").and_then(Value::as_f64).unwrap_or(0.0);
        let y = found.get("y").and_then(Value::as_f64).unwrap_or(0.0);
        input::mouse_click(page.cdp(), x, y, "left", 1)?;
    } else {
        // Something covers it (or it is off screen): click it directly in the page.
        page.run(scripts::CLICK_MARKED, json!([]))?;
    }
    let label = Some(string_field(&found, "label"))
        .filter(|label| !label.is_empty())
        .unwrap_or(query);
    let (url, title) = settle(&mut page, Duration::from_millis(700));
    Ok(format!(
        "Clicked \"{label}\". Now on {}. Read the page to see what changed.",
        place(&url, &title)
    ))
}

pub(crate) fn type_text(
    paths: &GxserverPaths,
    dot_id: &str,
    target: &FieldTarget,
    text: &str,
    submit: bool,
) -> Result<String, String> {
    secrets::scrub_result(dot_id, type_text_inner(paths, dot_id, target, text, submit))
}

fn type_text_inner(
    paths: &GxserverPaths,
    dot_id: &str,
    target: &FieldTarget,
    text: &str,
    submit: bool,
) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let (mode, query, fallback) = match target {
        FieldTarget::Ref(reference) => {
            let reference = normalize_ref(reference);
            let fallback = page.ref_entry(&reference).map(|entry| entry.selector);
            ("ref", reference, fallback)
        }
        FieldTarget::Label(label) => ("label", label.trim().to_string(), None),
    };
    let found = page.run(scripts::FOCUS_FIELD, json!([mode, query, fallback, text]))?;
    if found.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(script_error(&found, "Couldn't find that field."));
    }
    let field = string_field(&found, "field");
    let mut note = String::new();
    if string_field(&found, "kind") != "select" {
        if text.is_empty() {
            input::press(page.cdp(), "Backspace")?;
        } else {
            input::insert_text(page.cdp(), text)?;
        }
        let check = page.run(scripts::ENSURE_FIELD_VALUE, json!([text]))?;
        if check.get("ok").and_then(Value::as_bool) != Some(true) {
            let shown = string_field(&check, "value");
            note = format!(" The field now shows \"{shown}\"; check it before you continue.");
        }
    }
    if submit {
        input::press(page.cdp(), "Enter")?;
        let (url, _) = settle(&mut page, Duration::from_millis(1200));
        return Ok(format!(
            "Typed into \"{field}\" and submitted. Now on {}.{note}",
            display_url(&url)
        ));
    }
    std::thread::sleep(Duration::from_millis(300));
    Ok(format!("Typed into \"{field}\".{note}"))
}

pub(crate) fn press_key(paths: &GxserverPaths, dot_id: &str, key: &str) -> Result<String, String> {
    secrets::scrub_result(dot_id, press_key_inner(paths, dot_id, key))
}

fn press_key_inner(paths: &GxserverPaths, dot_id: &str, key: &str) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let pressed = input::press(page.cdp(), key)?;
    let (url, title) = settle(&mut page, Duration::from_millis(400));
    Ok(format!(
        "Pressed {pressed}. Now on {}.",
        place(&url, &title)
    ))
}

/// Scrolls the page; positive `delta_y` scrolls down.
pub(crate) fn scroll(paths: &GxserverPaths, dot_id: &str, delta_y: i64) -> Result<String, String> {
    secrets::scrub_result(dot_id, scroll_inner(paths, dot_id, delta_y))
}

fn scroll_inner(paths: &GxserverPaths, dot_id: &str, delta_y: i64) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    let before = page
        .run(scripts::SCROLL_INFO, json!([]))?
        .get("y")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    input::mouse_wheel(
        page.cdp(),
        f64::from(SCREEN_WIDTH) / 2.0,
        f64::from(SCREEN_HEIGHT) / 2.0,
        0.0,
        delta_y as f64,
    )?;
    std::thread::sleep(Duration::from_millis(400));
    let info = page.run(scripts::SCROLL_INFO, json!([]))?;
    let y = info.get("y").and_then(Value::as_i64).unwrap_or(0);
    let height = info.get("height").and_then(Value::as_i64).unwrap_or(0);
    let viewport = info.get("viewport").and_then(Value::as_i64).unwrap_or(0);
    let direction = if delta_y < 0 { "up" } else { "down" };
    if y == before && delta_y != 0 {
        return Ok(format!(
            "The page did not move {direction}: it is already at the {} or this part scrolls inside its own box ({height}px page).",
            if delta_y < 0 { "top" } else { "bottom" }
        ));
    }
    let mut out = format!(
        "Scrolled {direction} {}px. The view now starts {y}px down a {height}px page.",
        (y - before).unsigned_abs()
    );
    if delta_y > 0 && y + viewport >= height {
        out.push_str(" This is the bottom.");
    } else if delta_y < 0 && y <= 0 {
        out.push_str(" This is the top.");
    }
    Ok(out)
}

pub(crate) fn go_back(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    secrets::scrub_result(dot_id, go_back_inner(paths, dot_id))
}

fn go_back_inner(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    if !history_step(page.cdp(), -1)? {
        return Ok("There is no earlier page in this tab.".to_string());
    }
    std::thread::sleep(Duration::from_millis(500));
    let (url, title) = settle(&mut page, Duration::ZERO);
    Ok(format!("Went back to {}.", place(&url, &title)))
}

/// A PNG of the current page for the model (starts the browser if needed).
pub(crate) fn screenshot_png(paths: &GxserverPaths, dot_id: &str) -> Result<Vec<u8>, String> {
    let browser = tool_browser(paths, dot_id)?;
    let mut page = tool_page(&browser, Start::Restore)?;
    capture(page.cdp(), "png", None).map_err(|error| secrets::scrub(dot_id, error))
}

/// The dot's current page URL, without starting a browser (an error when none runs).
pub(crate) fn current_url(paths: &GxserverPaths, dot_id: &str) -> Result<String, String> {
    let _ = paths;
    check_dot_id(dot_id)?;
    let not_running =
        || "The dot's browser is not running. Open the sign-in page first.".to_string();
    let browser = existing_browser(dot_id).ok_or_else(not_running)?;
    let mut page = browser
        .page(Lane::Tool, Start::IfRunning)?
        .ok_or_else(not_running)?;
    let (url, _) = page_place(&mut page);
    Ok(secrets::scrub(dot_id, url))
}

/// Fills the login form on the current page with a saved credential, only on an https page whose
/// host is `host` (the saved login's site) or one of its subdomains, checked inside the page in the
/// same script that fills the fields. The password never leaves this call; the result names only
/// the fields filled.
pub(crate) fn fill_login_for_host(
    paths: &GxserverPaths,
    dot_id: &str,
    username: &str,
    password: &str,
    host: &str,
) -> Result<String, String> {
    let Some(host) = login_host(host) else {
        return Err(format!(
            "The saved login's site \"{}\" is not a web address, so it was not filled. Save it again with the site's address (for example github.com).",
            host.trim()
        ));
    };
    fill(paths, dot_id, username, password, &host)
}

/// The visible label of the element a `page_snapshot` ref points to, read from the live page with
/// the same resolution `click` uses. `None` when no browser runs or the ref is gone.
pub(crate) fn ref_label(paths: &GxserverPaths, dot_id: &str, element_ref: &str) -> Option<String> {
    click_label(paths, dot_id, &ClickTarget::Ref(element_ref.to_string()))
}

/// Every name the element `click` would click for `target` goes by (its text, value, aria-label,
/// title and alt joined with " / "), read from the live page without clicking and without
/// starting a browser. `None` when no browser runs or nothing matches.
pub(crate) fn click_label(
    paths: &GxserverPaths,
    dot_id: &str,
    target: &ClickTarget,
) -> Option<String> {
    let _ = paths;
    check_dot_id(dot_id).ok()?;
    let browser = existing_browser(dot_id)?;
    let mut page = browser.page(Lane::Tool, Start::IfRunning).ok()??;
    let (mode, query, fallback) = click_query(&page, target);
    let found = page
        .run(scripts::CLICK_TARGET, json!([mode, query, fallback, true]))
        .ok()?;
    if found.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(secrets::scrub(dot_id, string_field(&found, "label")))
}

/// A saved login's site as the page host it may be filled on: `https://www.github.com/login`
/// gives `github.com`. `None` for anything that is not a dotted host name (a bare "com" would
/// match every .com site).
fn login_host(site: &str) -> Option<String> {
    let site = site.trim().to_ascii_lowercase();
    let site = site
        .strip_prefix("https://")
        .or_else(|| site.strip_prefix("http://"))
        .unwrap_or(&site);
    let site = site.split(['/', '?', '#']).next().unwrap_or_default();
    let site = site.rsplit('@').next().unwrap_or_default();
    let site = site.split(':').next().unwrap_or_default();
    let site = site.trim_start_matches("*.").trim_matches('.');
    let site = site.strip_prefix("www.").unwrap_or(site);
    let valid = site.contains('.')
        && site.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        });
    valid.then(|| site.to_string())
}

/// The login fill behind `fill_login_for_host`.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The https and host checks run inside the page, in the same script that fills the fields, because the page can navigate between any earlier check and the fill. After a fill the password is remembered for this dot in memory only and scrubbed from every browser tool's text, and the field is marked so it stays hidden from snapshots after a "Show password" button turns it into a text field.
fn fill(
    paths: &GxserverPaths,
    dot_id: &str,
    username: &str,
    password: &str,
    host: &str,
) -> Result<String, String> {
    // No error from here on may carry the password, so page and script errors are replaced.
    let scrub = |error: String| {
        if !password.is_empty() && error.contains(password) {
            "Couldn't fill the login form on this page.".to_string()
        } else {
            secrets::scrub(dot_id, error)
        }
    };
    let browser = tool_browser(paths, dot_id).map_err(scrub)?;
    let mut page = tool_page(&browser, Start::Restore).map_err(scrub)?;
    let filled = page
        .run(scripts::FILL_LOGIN, json!([username, password, host]))
        .map_err(|_| "Couldn't fill the login form on this page.".to_string())?;
    let shown_page = string_field(&filled, "page");
    match string_field(&filled, "refused").as_str() {
        "https" => {
            return Err(format!(
                "Refused: {} is not a secure (https) page, so the saved login was not filled. Open the site's https sign-in page first.",
                if shown_page.is_empty() { "this page" } else { shown_page.as_str() }
            ))
        }
        "host" => {
            return Err(format!(
                "Refused: this page is on {shown_page}, not {host}, so the saved login was not filled. Open {host}'s sign-in page first."
            ))
        }
        _ => {}
    }
    let user = filled.get("user").and_then(Value::as_bool) == Some(true);
    let pass = filled.get("pass").and_then(Value::as_bool) == Some(true);
    if pass {
        secrets::remember(dot_id, password);
    }
    std::thread::sleep(Duration::from_millis(300));
    if !user && !pass {
        return Ok(
            "No visible login fields found on this page. Navigate to the sign-in form first."
                .to_string(),
        );
    }
    let what = if user && pass {
        "username and password"
    } else {
        "the only login field visible (it may be a multi-step form; continue and call again)"
    };
    Ok(secrets::scrub(
        dot_id,
        format!(
            "Filled {what} for {username} on {shown_page}. Submit the form to continue (the password itself is hidden from you)."
        ),
    ))
}

/// The Computer tab's picture. `wake` starts the browser when it is not running (a fresh start
/// shows the dot's last page, or Google).
pub(crate) fn live_screenshot(
    paths: &GxserverPaths,
    dot_id: &str,
    wake: bool,
) -> Result<BrowserShot, String> {
    check_dot_id(dot_id)?;
    // The Computer tab is showing the screen: the user's take-over lease stays alive.
    session::renew_take_over(dot_id);
    let taken_over = is_taken_over(dot_id);
    let browser = if wake {
        Some(browser_for(profile_dir(paths, dot_id), dot_id)?)
    } else {
        existing_browser(dot_id)
    };
    let idle = |url: Option<String>| BrowserShot {
        jpeg: None,
        url: url.unwrap_or_default(),
        title: String::new(),
        live: false,
        taken_over,
    };
    let Some(browser) = browser else {
        return Ok(idle(read_last_url(&profile_dir(paths, dot_id))));
    };
    let start = if wake { Start::Wake } else { Start::IfRunning };
    let Some(mut page) = browser.page(Lane::Live, start)? else {
        return Ok(idle(browser.last_url()));
    };
    let jpeg = capture(page.cdp(), "jpeg", Some(70))?;
    page.remember_url(&page.url.clone());
    Ok(BrowserShot {
        jpeg: Some(jpeg),
        url: page.url.clone(),
        title: page.title.clone(),
        live: true,
        taken_over,
    })
}

/// The user's own input while they drive (`browserInput` params: `kind`, `x`, `y`, `text`,
/// `key`, `deltaX`, `deltaY`, `url`, optional `button`: left|right|middle). `x`/`y` are in the
/// 1280x800 page space. `key` is a DOM key name (`Enter`, `a`, `ArrowLeft`), optionally with
/// modifiers joined by `+` (`Meta+a`).
pub(crate) fn user_input(paths: &GxserverPaths, dot_id: &str, input: &Value) -> Result<(), String> {
    check_dot_id(dot_id)?;
    session::renew_take_over(dot_id);
    let browser = browser_for(profile_dir(paths, dot_id), dot_id)?;
    let mut page = browser
        .page(Lane::Live, Start::Restore)?
        .ok_or_else(|| "The dot's browser is not running.".to_string())?;
    let number = |key: &str| input.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let button = match input.get("button").and_then(Value::as_str) {
        Some("right") => "right",
        Some("middle") => "middle",
        _ => "left",
    };
    let cdp = page.cdp();
    match input
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "click" => input::mouse_click(cdp, number("x"), number("y"), button, 1),
        "doubleClick" => input::mouse_click(cdp, number("x"), number("y"), button, 2),
        "move" => input::mouse_move(cdp, number("x"), number("y")),
        "type" => input::type_text(cdp, &text("text")),
        "key" => input::press(cdp, &text("key")).map(|_| ()),
        "scroll" => input::mouse_wheel(
            cdp,
            number("x"),
            number("y"),
            number("deltaX"),
            number("deltaY"),
        ),
        "navigate" => {
            let url = normalize_user_url(&text("url"));
            // Like Open Dot's `waitUntil: "commit"`: the picture follows the page as it loads.
            cdp.call_with_timeout("Page.navigate", json!({ "url": url }), NAVIGATION_TIMEOUT)
                .map(|_| ())
        }
        "back" => history_step(cdp, -1).map(|_| ()),
        "forward" => history_step(cdp, 1).map(|_| ()),
        "reload" => cdp.call("Page.reload", json!({})).map(|_| ()),
        other => Err(format!("Unknown browser input \"{other}\".")),
    }
}

pub(crate) fn set_taken_over(dot_id: &str, taken_over: bool) {
    session::set_taken_over(dot_id, taken_over);
}

pub(crate) fn is_taken_over(dot_id: &str) -> bool {
    session::is_taken_over(dot_id)
}

/// Closes the dot's browser and deletes its profile (logins are lost).
pub(crate) fn reset(paths: &GxserverPaths, dot_id: &str) -> Result<(), String> {
    check_dot_id(dot_id)?;
    let profile = profile_dir(paths, dot_id);
    if let Some(browser) = remove_browser(dot_id) {
        browser.shutdown();
    }
    // A Chrome left from an earlier gxserver run may still hold the profile.
    chrome::free_profile(&profile);
    session::set_taken_over(dot_id, false);
    secrets::forget(dot_id);
    match std::fs::remove_dir_all(&profile) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Couldn't delete the dot's browser profile: {error}"
        )),
    }
}

/// Closes the Chrome of every dot browser nobody used for `idle_limit` (no tool call, user input or
/// Computer tab picture); the next call starts it again on the dot's last page. The reaper thread
/// runs this every minute with a 15-minute limit. Returns the dots whose Chrome was closed.
pub(crate) fn close_idle_browsers(idle_limit: Duration) -> Vec<String> {
    session::reap_idle(idle_limit)
}

/// Closes dot Chromes an earlier gxserver left running (it crashed or was killed before closing
/// them), found by their profile folder under this gxserver's state. Runs once per process: call
/// it at startup; the first browser start runs it otherwise. Returns how many were closed.
pub(crate) fn sweep_orphans(paths: &GxserverPaths) -> usize {
    let registry = session::registry_lock();
    sweep_locked(&browsers_root(paths), &registry)
}

/// The first browser start's sweep, under the registry lock it already holds.
fn sweep_orphans_once(profile: &Path, registry: &HashMap<String, Arc<DotBrowser>>) {
    if let Some(root) = profile.parent() {
        sweep_locked(root, registry);
    }
}

fn sweep_locked(root: &Path, registry: &HashMap<String, Arc<DotBrowser>>) -> usize {
    static SWEPT: AtomicBool = AtomicBool::new(false);
    if SWEPT.swap(true, Ordering::SeqCst) {
        return 0;
    }
    chrome::sweep_orphans(root, &session::running_chrome_pids(registry))
}

/// Closes every dot browser (gxserver shutdown, OpenDots turned off).
pub(crate) fn shutdown_all() {
    let closing: Vec<_> = drain_browsers()
        .into_iter()
        .map(|browser| std::thread::spawn(move || browser.shutdown()))
        .collect();
    for handle in closing {
        let _ = handle.join();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `<root_dir>/open-dots/browsers`: every dot's profile lives here.
fn browsers_root(paths: &GxserverPaths) -> PathBuf {
    paths.root_dir.join("open-dots").join("browsers")
}

/// `<root_dir>/open-dots/browsers/<dotId>`: Chrome's user data dir, plus `last-url`.
fn profile_dir(paths: &GxserverPaths, dot_id: &str) -> PathBuf {
    browsers_root(paths).join(dot_id)
}

/// Dot ids name a folder that `reset` deletes, so only plain ids are accepted.
fn check_dot_id(dot_id: &str) -> Result<(), String> {
    let plain = !dot_id.is_empty()
        && dot_id.len() <= 64
        && dot_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if plain {
        Ok(())
    } else {
        Err("Unknown dot.".to_string())
    }
}

/// The dot's browser for one of its own tools.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Every model-facing browser call refuses while the user has taken over, here rather than only in the tool executor, so a tool call that was already queued cannot click or type under the user's hands.
fn tool_browser(paths: &GxserverPaths, dot_id: &str) -> Result<std::sync::Arc<DotBrowser>, String> {
    check_dot_id(dot_id)?;
    if is_taken_over(dot_id) {
        return Err(TAKEN_OVER.to_string());
    }
    browser_for(profile_dir(paths, dot_id), dot_id)
}

/// The dot's tab on the tool lane. The take-over check runs again once the lane is ours: a call
/// that queued behind another one must not act after the user took over in between.
fn tool_page<'a>(browser: &'a DotBrowser, start: Start) -> Result<Page<'a>, String> {
    let page = browser
        .page(Lane::Tool, start)?
        .ok_or_else(|| "The dot's browser is not running.".to_string())?;
    if is_taken_over(page.dot_id()) {
        return Err(TAKEN_OVER.to_string());
    }
    Ok(page)
}

const TAKEN_OVER: &str =
    "The user has taken over this browser. Wait until they hand it back, then look at the page again.";

/// The page's URL and title (retrying through a navigation that swaps the document), recorded
/// as the dot's last place.
fn page_place(page: &mut Page<'_>) -> (String, String) {
    for _ in 0..5 {
        if let Ok(info) = page.run(scripts::PAGE_INFO, json!([])) {
            let url = string_field(&info, "url");
            page.remember_url(&url);
            return (url, string_field(&info, "title"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    (page.url.clone(), page.title.clone())
}

/// After an action: let it start whatever it triggers, wait for a new document to become
/// interactive, then report where the page is.
fn settle(page: &mut Page<'_>, pause: Duration) -> (String, String) {
    std::thread::sleep(pause);
    session::wait_ready(page.cdp(), Instant::now() + Duration::from_secs(10));
    page_place(page)
}

fn place(url: &str, title: &str) -> String {
    let url = display_url(url);
    if title.trim().is_empty() {
        url
    } else {
        format!("{url} (\"{}\")", title.trim())
    }
}

/// A URL as the model reads it: `data:` pages are cut short (their address is the whole page).
fn display_url(url: &str) -> String {
    if url.starts_with("data:") && url.chars().count() > 48 {
        let head: String = url.chars().take(48).collect();
        format!("{head}...")
    } else {
        url.to_string()
    }
}

fn capture(cdp: &mut cdp::Cdp, format: &str, quality: Option<u32>) -> Result<Vec<u8>, String> {
    let mut params = json!({ "format": format, "captureBeyondViewport": false });
    if let Some(quality) = quality {
        params["quality"] = json!(quality);
    }
    let shot = cdp.call("Page.captureScreenshot", params)?;
    let data = shot
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "The dot's browser returned no picture.".to_string())?;
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "The dot's browser returned a broken picture.".to_string())
}

/// Moves through the tab's history; false when there is no entry that way.
fn history_step(cdp: &mut cdp::Cdp, step: i64) -> Result<bool, String> {
    let history = cdp.call("Page.getNavigationHistory", json!({}))?;
    let index = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        + step;
    let entries = history
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(entry) = usize::try_from(index)
        .ok()
        .and_then(|index| entries.get(index))
    else {
        return Ok(false);
    };
    let entry_id = entry.get("id").cloned().unwrap_or(Value::Null);
    cdp.call(
        "Page.navigateToHistoryEntry",
        json!({ "entryId": entry_id }),
    )?;
    Ok(true)
}

fn script_error(result: &Value, fallback: &str) -> String {
    Some(string_field(result, "error"))
        .filter(|error| !error.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// `e12`, `@e12`, `[e12]` and `E12` all name snapshot ref `e12`.
fn normalize_ref(reference: &str) -> String {
    reference
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_start_matches('@')
        .trim()
        .to_ascii_lowercase()
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for ch in text.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(ch);
    }
    out
}

/// The dot's `open_url` target: web pages only. `about:blank` and `data:` pages are allowed;
/// other schemes (`file:`, `javascript:`, `chrome:`) are refused so the browser tools never reach
/// what the agent's own permission prompts guard. A bare host gets `https://` (`http://` for
/// localhost).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Browser tools run without the agent CLI's permission prompts, so `file:` URLs would let a dot read any file on the computer around those prompts.
fn normalize_tool_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("Give a URL to open.".to_string());
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower == "about:blank"
    {
        return Ok(url.to_string());
    }
    if let Some((scheme, rest)) = url.split_once(':') {
        let looks_like_scheme = !scheme.is_empty()
            && scheme
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
            && !rest.starts_with(|ch: char| ch.is_ascii_digit());
        if looks_like_scheme && !scheme.contains('.') {
            return Err("The dot's browser only opens web pages (http and https).".to_string());
        }
    }
    if lower.starts_with("localhost") || lower.starts_with("127.0.0.1") {
        return Ok(format!("http://{url}"));
    }
    Ok(format!("https://{url}"))
}

/// The Computer tab's address bar (Open Dot's `nav` input): a URL, a bare host, or a Google
/// search.
fn normalize_user_url(url: &str) -> String {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower == "about:blank" {
        return url.to_string();
    }
    let host_like = !url.is_empty()
        && !url.contains(char::is_whitespace)
        && url.contains('.')
        && !url.contains("://");
    if host_like {
        return normalize_tool_url(url).unwrap_or_else(|_| format!("https://{url}"));
    }
    let query: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
    format!("https://www.google.com/search?q={query}")
}
