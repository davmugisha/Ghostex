//! The process-global registry of dot browsers and the connection each call drives.
//!
//! Each dot has one Chrome and two CDP connections to its current tab: the `Tool` lane serves the
//! dot's own browser tools, the `Live` lane serves the Computer tab (screenshots and the user's
//! input). Separate lanes keep the Computer tab's picture moving while a dot's tool waits for a
//! slow page. Lock order is always lane, then `shared`.
//!
//! A take-over is a lease the user's input and the Computer tab's pictures renew, and a Chrome no
//! tool or person used for a while is closed by the idle reaper (the next call starts it again on
//! the dot's last page).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::cdp::{http_json, http_put_json, Cdp};
use super::chrome::{self, ChromeProc};
use super::scripts;

/// Which connection a call uses.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    Tool,
    Live,
}

/// Whether a call may start Chrome, and what a freshly started Chrome shows first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Start {
    /// Only use a Chrome that is already running.
    IfRunning,
    /// Start Chrome if needed; leave a fresh one blank (the caller navigates).
    Launch,
    /// Start Chrome if needed; a fresh one reopens the dot's last real page.
    Restore,
    /// Like `Restore`, and a fresh Chrome with no last page shows Google (the Computer tab).
    Wake,
}

/// What `page_snapshot` listed under a ref.
#[derive(Clone)]
pub(super) struct RefEntry {
    pub(super) selector: String,
    /// The page the snapshot was taken on.
    pub(super) url: String,
}

struct Shared {
    chrome: Option<ChromeProc>,
    target_id: Option<String>,
    known_targets: HashSet<String>,
    last_url: Option<String>,
    /// Set by `shutdown`: this entry left the registry and must never start Chrome again.
    retired: bool,
}

struct PageConn {
    cdp: Cdp,
    target_id: String,
    /// The isolated world the page scripts run in: the document's loader id and the context id.
    world: Option<(String, i64)>,
}

/// How long a take-over lasts without the user's input or a Computer tab picture.
const TAKE_OVER_LEASE: Duration = Duration::from_secs(2 * 60);
/// How long a dot's Chrome may sit unused (no tool call, no input, no Computer tab picture).
pub(super) const IDLE_LIMIT: Duration = Duration::from_secs(15 * 60);
const REAPER_EVERY: Duration = Duration::from_secs(60);

pub(super) struct DotBrowser {
    dot_id: String,
    profile: PathBuf,
    chrome_path: PathBuf,
    shared: Mutex<Shared>,
    tool: Mutex<Option<PageConn>>,
    live: Mutex<Option<PageConn>>,
    refs: Mutex<HashMap<String, RefEntry>>,
    last_activity: Mutex<Instant>,
}

/// A connected tab, holding its lane's lock for the length of one call.
pub(super) struct Page<'a> {
    guard: MutexGuard<'a, Option<PageConn>>,
    browser: &'a DotBrowser,
    pub(super) url: String,
    pub(super) title: String,
}

impl Page<'_> {
    fn conn(&mut self) -> &mut PageConn {
        self.guard
            .as_mut()
            .expect("a Page always holds a connection")
    }

    pub(super) fn cdp(&mut self) -> &mut Cdp {
        &mut self.conn().cdp
    }

    /// Evaluates a script from `scripts` with JSON arguments in the page's isolated world; the
    /// script's return value.
    ///
    /// CDXC:OpenDots 2026-10-01 WHY:
    /// The scripts run in an isolated world (their own copy of every JavaScript built-in over the page's DOM), not in the page's own world, because a page can replace `String.prototype.endsWith`, `querySelector` or an element's `innerText` there: that would pass the sign-in host check on the wrong site, or show the risky-click check a harmless label for a "Pay" button.
    pub(super) fn run(&mut self, source: &str, args: Value) -> Result<Value, String> {
        let args = serde_json::to_string(&args).map_err(|error| error.to_string())?;
        let expression = format!("({source})(...{args})");
        let mut outcome = Err("The page refused that.".to_string());
        for _ in 0..2 {
            let context = self.isolated_world()?;
            outcome = evaluate_in(self.cdp(), &expression, Some(context));
            match &outcome {
                // The document changed under the cached world: make a new one and try once more.
                Err(error) if context_gone(error) => self.conn().world = None,
                _ => return outcome,
            }
        }
        outcome
    }

    /// The execution context of this document's isolated world, created once per document.
    fn isolated_world(&mut self) -> Result<i64, String> {
        let tree = self.cdp().call("Page.getFrameTree", json!({}))?;
        let frame_id = tree
            .pointer("/frameTree/frame/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let loader_id = tree
            .pointer("/frameTree/frame/loaderId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if frame_id.is_empty() {
            return Err("The dot's page is not ready.".to_string());
        }
        if let Some((known_loader, context)) = &self.conn().world {
            if *known_loader == loader_id {
                return Ok(*context);
            }
        }
        let created = self.cdp().call(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame_id, "worldName": "ghostex-dot", "grantUniveralAccess": false }),
        )?;
        let context = created
            .get("executionContextId")
            .and_then(Value::as_i64)
            .ok_or_else(|| "The dot's page is not ready.".to_string())?;
        self.conn().world = Some((loader_id, context));
        Ok(context)
    }

    /// Records the page as the dot's last real place (restored on the next fresh start).
    pub(super) fn remember_url(&self, url: &str) {
        self.browser.remember_url(url);
    }

    pub(super) fn set_refs(&self, refs: HashMap<String, RefEntry>) {
        *lock(&self.browser.refs) = refs;
    }

    /// The snapshot's record of a ref, only while the tab still shows the page it was taken on
    /// (its selector would name a different element on another page).
    pub(super) fn ref_entry(&self, reference: &str) -> Option<RefEntry> {
        lock(&self.browser.refs)
            .get(reference)
            .filter(|entry| entry.url == self.url)
            .cloned()
    }

    pub(super) fn dot_id(&self) -> &str {
        &self.browser.dot_id
    }
}

impl Drop for Page<'_> {
    fn drop(&mut self) {
        self.browser.touch();
    }
}

fn context_gone(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("cannot find context") || error.contains("context was destroyed")
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn registry() -> &'static Mutex<HashMap<String, Arc<DotBrowser>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<DotBrowser>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Dots the user took over, with when their lease was last renewed.
fn taken_over_dots() -> &'static Mutex<HashMap<String, Instant>> {
    static TAKEN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    TAKEN.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn set_taken_over(dot_id: &str, taken_over: bool) {
    let mut taken = lock(taken_over_dots());
    if taken_over {
        taken.insert(dot_id.to_string(), Instant::now());
    } else {
        taken.remove(dot_id);
    }
}

/// True while the user's take-over lease runs; a lapsed lease hands the browser back.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// A take-over is a lease, not a flag: a person who took over from the Computer tab and then closed the app, or lost their phone's connection, would otherwise leave the dot's browser tools refusing forever. Their input and the Computer tab's pictures renew it; two minutes without either hands the browser back.
pub(super) fn is_taken_over(dot_id: &str) -> bool {
    let mut taken = lock(taken_over_dots());
    match taken.get(dot_id) {
        Some(renewed) if renewed.elapsed() < TAKE_OVER_LEASE => true,
        Some(_) => {
            taken.remove(dot_id);
            false
        }
        None => false,
    }
}

/// The user is still driving (their input, or the Computer tab showing the screen).
pub(super) fn renew_take_over(dot_id: &str) {
    if let Some(renewed) = lock(taken_over_dots())
        .get_mut(dot_id)
        .filter(|renewed| renewed.elapsed() < TAKE_OVER_LEASE)
    {
        *renewed = Instant::now();
    }
}

/// The dot's browser entry, created (not started) on first use.
pub(super) fn browser_for(profile: PathBuf, dot_id: &str) -> Result<Arc<DotBrowser>, String> {
    let mut registry = lock(registry());
    if let Some(browser) = registry.get(dot_id) {
        return Ok(browser.clone());
    }
    let chrome_path = chrome::find_chrome().ok_or_else(|| {
        "Google Chrome is not installed on this computer. Install it to give your dots a browser."
            .to_string()
    })?;
    super::sweep_orphans_once(&profile, &registry);
    start_reaper();
    let last_url = read_last_url(&profile);
    let browser = Arc::new(DotBrowser {
        dot_id: dot_id.to_string(),
        profile,
        chrome_path,
        shared: Mutex::new(Shared {
            chrome: None,
            target_id: None,
            known_targets: HashSet::new(),
            last_url,
            retired: false,
        }),
        tool: Mutex::new(None),
        live: Mutex::new(None),
        refs: Mutex::new(HashMap::new()),
        last_activity: Mutex::new(Instant::now()),
    });
    registry.insert(dot_id.to_string(), browser.clone());
    Ok(browser)
}

/// The process ids of the Chromes this gxserver runs (kept by the orphan sweep).
pub(super) fn running_chrome_pids(registry: &HashMap<String, Arc<DotBrowser>>) -> Vec<u32> {
    registry
        .values()
        .filter_map(|browser| lock(&browser.shared).chrome.as_ref().map(ChromeProc::pid))
        .collect()
}

pub(super) fn registry_lock() -> MutexGuard<'static, HashMap<String, Arc<DotBrowser>>> {
    lock(registry())
}

/// Closes the Chrome of every dot browser unused for `limit` (no tool call, user input or
/// Computer tab picture). The entry stays, so the next call starts Chrome again on the last page.
/// Returns the dots whose Chrome was closed.
pub(super) fn reap_idle(limit: Duration) -> Vec<String> {
    let browsers: Vec<Arc<DotBrowser>> = lock(registry()).values().cloned().collect();
    browsers
        .into_iter()
        .filter(|browser| browser.close_if_idle(limit))
        .map(|browser| browser.dot_id.clone())
        .collect()
}

/// Starts the idle reaper thread once per process.
fn start_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("open-dots-browser-reaper".to_string())
            .spawn(|| loop {
                std::thread::sleep(REAPER_EVERY);
                super::close_idle_browsers(IDLE_LIMIT);
            });
    });
}

/// A lane's lock without waiting: `None` while a call holds it.
fn try_lock<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match mutex.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

pub(super) fn existing_browser(dot_id: &str) -> Option<Arc<DotBrowser>> {
    lock(registry()).get(dot_id).cloned()
}

pub(super) fn remove_browser(dot_id: &str) -> Option<Arc<DotBrowser>> {
    lock(registry()).remove(dot_id)
}

pub(super) fn drain_browsers() -> Vec<Arc<DotBrowser>> {
    lock(registry())
        .drain()
        .map(|(_, browser)| browser)
        .collect()
}

fn last_url_file(profile: &Path) -> PathBuf {
    profile.join("last-url")
}

pub(super) fn is_real_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub(super) fn read_last_url(profile: &Path) -> Option<String> {
    std::fs::read_to_string(last_url_file(profile))
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| is_real_url(url))
}

/// One page target from `/json/list`.
struct Target {
    id: String,
    url: String,
    title: String,
    ws_url: String,
}

impl DotBrowser {
    /// The dot's current tab on `lane`, starting Chrome as `start` allows. `Ok(None)` only for
    /// `Start::IfRunning` when Chrome is not running.
    pub(super) fn page(&self, lane: Lane, start: Start) -> Result<Option<Page<'_>>, String> {
        let mut guard = lock(match lane {
            Lane::Tool => &self.tool,
            Lane::Live => &self.live,
        });
        self.touch();
        let (target, launched) = {
            let mut shared = lock(&self.shared);
            if shared.retired {
                return Err("The dot's browser was closed.".to_string());
            }
            let mut launched = false;
            let running = shared.chrome.as_mut().is_some_and(ChromeProc::alive);
            if !running {
                if let Some(dead) = shared.chrome.take() {
                    dead.shutdown();
                }
                if start == Start::IfRunning {
                    return Ok(None);
                }
                self.start_chrome(&mut shared)?;
                launched = true;
            }
            let target = match self.resolve_target(&mut shared) {
                Ok(target) => target,
                Err(error) if launched || start == Start::IfRunning => return Err(error),
                // Chrome runs but no longer answers: replace it once.
                Err(_) => {
                    if let Some(stuck) = shared.chrome.take() {
                        stuck.shutdown();
                    }
                    self.start_chrome(&mut shared)?;
                    launched = true;
                    self.resolve_target(&mut shared)?
                }
            };
            (target, launched)
        };
        let reconnect = match guard.as_ref() {
            Some(conn) => conn.target_id != target.id || conn.cdp.is_closed(),
            None => true,
        };
        if reconnect {
            if let Some(mut old) = guard.take() {
                old.cdp.close();
            }
            let mut cdp = Cdp::connect(&target.ws_url)?;
            // Headless tabs opened by a page (popups) start at their own size; the dot's screen
            // is always the fixed 1280x800 the Computer tab and the model's coordinates assume.
            let _ = cdp.call(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": super::SCREEN_WIDTH,
                    "height": super::SCREEN_HEIGHT,
                    "deviceScaleFactor": 1,
                    "mobile": false,
                }),
            );
            let _ = cdp.call("Page.bringToFront", json!({}));
            *guard = Some(PageConn {
                cdp,
                target_id: target.id.clone(),
                world: None,
            });
        }
        let mut page = Page {
            guard,
            browser: self,
            url: target.url,
            title: target.title,
        };
        if launched && start != Start::Launch {
            let resume = lock(&self.shared).last_url.clone();
            let resume = resume
                .or_else(|| (start == Start::Wake).then(|| "https://www.google.com/".to_string()));
            if let Some(url) = resume {
                let _ = navigate(page.cdp(), &url, Duration::from_secs(15));
                if let Ok(info) = page.run(scripts::PAGE_INFO, json!([])) {
                    page.url = string_field(&info, "url");
                    page.title = string_field(&info, "title");
                }
            }
        }
        Ok(Some(page))
    }

    fn start_chrome(&self, shared: &mut Shared) -> Result<(), String> {
        shared.chrome = Some(chrome::launch(&self.chrome_path, &self.profile)?);
        shared.target_id = None;
        shared.known_targets.clear();
        Ok(())
    }

    /// Picks the tab to drive: a tab the page just opened (a sign-in popup) wins, then the tab
    /// already in use, then the most recently active one. Opens a blank tab when none is left.
    fn resolve_target(&self, shared: &mut Shared) -> Result<Target, String> {
        let port = shared
            .chrome
            .as_ref()
            .map(|chrome| chrome.port)
            .ok_or_else(|| "The dot's browser is not running.".to_string())?;
        let list = http_json(port, "/json/list", Duration::from_secs(3))?;
        let mut pages: Vec<Target> = list
            .as_array()
            .map(|targets| targets.iter().filter_map(page_target).collect())
            .unwrap_or_default();
        if pages.is_empty() {
            let created = http_put_json(port, "/json/new?about:blank", Duration::from_secs(5))?;
            pages.extend(page_target(&created));
        }
        let fresh_index = if shared.known_targets.is_empty() {
            None
        } else {
            pages
                .iter()
                .position(|page| !shared.known_targets.contains(&page.id))
        };
        let current_index = shared
            .target_id
            .as_ref()
            .and_then(|id| pages.iter().position(|page| &page.id == id));
        let chosen = fresh_index
            .or(current_index)
            .unwrap_or(0)
            .min(pages.len().saturating_sub(1));
        shared.known_targets = pages.iter().map(|page| page.id.clone()).collect();
        if pages.is_empty() {
            return Err("The dot's browser has no tab open.".to_string());
        }
        let target = pages.swap_remove(chosen);
        shared.target_id = Some(target.id.clone());
        Ok(target)
    }

    fn remember_url(&self, url: &str) {
        // `last-url` is a file: a URL carrying the filled password (a login form sent with GET)
        // is not written to disk.
        if !is_real_url(url) || super::secrets::contains_secret(&self.dot_id, url) {
            return;
        }
        let mut shared = lock(&self.shared);
        if shared.last_url.as_deref() == Some(url) {
            return;
        }
        shared.last_url = Some(url.to_string());
        let _ = std::fs::write(last_url_file(&self.profile), url);
    }

    pub(super) fn last_url(&self) -> Option<String> {
        lock(&self.shared).last_url.clone()
    }

    fn touch(&self) {
        *lock(&self.last_activity) = Instant::now();
    }

    /// Closes Chrome when nothing used it for `limit` and no call is running on either lane.
    fn close_if_idle(&self, limit: Duration) -> bool {
        let Some(mut tool) = try_lock(&self.tool) else {
            return false;
        };
        let Some(mut live) = try_lock(&self.live) else {
            return false;
        };
        if lock(&self.last_activity).elapsed() < limit {
            return false;
        }
        let chrome = {
            let mut shared = lock(&self.shared);
            shared.target_id = None;
            shared.known_targets.clear();
            shared.chrome.take()
        };
        for conn in [tool.take(), live.take()].into_iter().flatten() {
            let mut cdp = conn.cdp;
            cdp.close();
        }
        lock(&self.refs).clear();
        match chrome {
            Some(chrome) => {
                chrome.shutdown();
                true
            }
            None => false,
        }
    }

    /// Stops Chrome for good (reset, dot deleted, gxserver exit).
    pub(super) fn shutdown(&self) {
        let chrome = {
            let mut shared = lock(&self.shared);
            shared.retired = true;
            shared.chrome.take()
        };
        if let Some(chrome) = chrome {
            chrome.shutdown();
        }
    }
}

fn page_target(target: &Value) -> Option<Target> {
    if target.get("type").and_then(Value::as_str) != Some("page") {
        return None;
    }
    let url = string_field(target, "url");
    if url.starts_with("devtools://") || url.starts_with("chrome-extension://") {
        return None;
    }
    let ws_url = string_field(target, "webSocketDebuggerUrl");
    if ws_url.is_empty() {
        return None;
    }
    Some(Target {
        id: string_field(target, "id"),
        url,
        title: string_field(target, "title"),
        ws_url,
    })
}

pub(super) fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `Runtime.evaluate` in the page's own world, returning the value (by value), or the page's
/// exception as an error.
pub(super) fn evaluate(cdp: &mut Cdp, expression: &str) -> Result<Value, String> {
    evaluate_in(cdp, expression, None)
}

/// `evaluate` in a given execution context (an isolated world), or the page's own when `None`.
fn evaluate_in(cdp: &mut Cdp, expression: &str, context: Option<i64>) -> Result<Value, String> {
    let mut params = json!({
        "expression": expression,
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": true,
    });
    if let Some(context) = context {
        params["contextId"] = json!(context);
    }
    let result = cdp.call("Runtime.evaluate", params)?;
    if let Some(details) = result.get("exceptionDetails") {
        let message = details
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| details.get("text").and_then(Value::as_str))
            .unwrap_or("The page script failed.");
        let first_line = message.lines().next().unwrap_or(message);
        return Err(format!("The page refused that: {first_line}"));
    }
    Ok(result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

/// Navigates and waits until the new document is interactive (Playwright's `domcontentloaded`).
pub(super) fn navigate(cdp: &mut Cdp, url: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let result = cdp.call_with_timeout("Page.navigate", json!({ "url": url }), timeout)?;
    if let Some(error) = result
        .get("errorText")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
    {
        return Err(format!("Couldn't open {url} ({error})."));
    }
    wait_ready(cdp, deadline);
    Ok(())
}

/// Waits (best effort) until the document is at least interactive.
pub(super) fn wait_ready(cdp: &mut Cdp, deadline: Instant) {
    loop {
        if let Ok(Value::String(state)) = evaluate(cdp, "document.readyState") {
            if state == "interactive" || state == "complete" {
                return;
            }
        }
        if Instant::now() >= deadline || cdp.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
