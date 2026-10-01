//! The OpenDots view's state: the snapshot gxserver answers `snapshot` with, the page showing, the
//! forms, and every call to `/api/openDotsRpc`. The snapshot is read again after each action, on
//! the `openDotsChanged` event (`open_dots_changed`), and every 5s while the view shows.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::computer::ComputerState;
use super::forms::Forms;
use super::model::{DotChat, DotsSnapshot};
use super::style::DotsPalette;
use super::target::{OpenDotsDotTab, OpenDotsTarget};
use crate::GhostexGpuiApp;
use crate::app::gx_store::gx_rpc;
use crate::app::window::native_modal_kit::ModalPalette;
use gpui::{Context, ScrollHandle, Task, WeakEntity};
use serde_json::{Value, json};
use web_time::Instant;

const OPEN_DOTS_RPC_PATH: &str = "/api/openDotsRpc";
/// How often the showing view reads the snapshot again without an event: a dot's status follows
/// its sessions, which change without an `openDotsChanged`.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(250);

/// One call to `/api/openDotsRpc`; the error is the daemon's own customer-readable message.
pub(crate) async fn open_dots_rpc(method: &str, params: Value) -> Result<Value, String> {
    gx_rpc(
        None,
        OPEN_DOTS_RPC_PATH,
        json!({ "method": method, "params": params }),
    )
    .await
    .map_err(|error| error.message)
}

pub(crate) struct NativeOpenDotsView {
    pub(crate) app: WeakEntity<GhostexGpuiApp>,
    pub(crate) page: OpenDotsTarget,
    /// `None` until the first `snapshot` answers.
    pub(crate) snapshot: Option<DotsSnapshot>,
    /// The first load failed, so there is nothing to show but the error and Retry.
    pub(crate) load_error: Option<String>,
    loading: bool,
    reload_wanted: bool,
    last_load: Option<Instant>,
    /// The last action's failure, shown under the page header.
    pub(crate) notice: Option<String>,
    /// The actions in flight, by key, so their buttons dim and a second click is ignored.
    pub(crate) busy: HashSet<String>,
    appearance: Option<(bool, bool)>,
    pub(crate) palette: Option<DotsPalette>,
    pub(crate) scroll: ScrollHandle,
    /// The page column's width at the last paint, for one- or two-column grids.
    pub(crate) page_width: std::rc::Rc<std::cell::Cell<f32>>,
    pub(crate) forms: Forms,
    pub(crate) computer: ComputerState,
    /// A dot project's conversations (`/api/listSessions`), by project id.
    pub(crate) chats: HashMap<String, Vec<DotChat>>,
    chats_loading: HashSet<String>,
    chats_read_at: HashMap<String, Instant>,
    /// The update whose full text and files are open on Home.
    pub(crate) expanded_update: Option<String>,
    pub(crate) skill_open: Option<String>,
    pub(crate) confirm_delete: bool,
    pub(crate) agent_picker_open: bool,
    /// Approvals and updates already announced, so a refetch never repeats a notification.
    announced: HashSet<String>,
    _tick: Task<()>,
}

impl NativeOpenDotsView {
    pub(crate) fn new(app: WeakEntity<GhostexGpuiApp>, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            app,
            page: OpenDotsTarget::Home,
            snapshot: None,
            load_error: None,
            loading: false,
            reload_wanted: false,
            last_load: None,
            notice: None,
            busy: HashSet::new(),
            appearance: None,
            palette: None,
            scroll: ScrollHandle::new(),
            page_width: std::rc::Rc::new(std::cell::Cell::new(880.0)),
            forms: Forms::default(),
            computer: ComputerState::default(),
            chats: HashMap::new(),
            chats_loading: HashSet::new(),
            chats_read_at: HashMap::new(),
            expanded_update: None,
            skill_open: None,
            confirm_delete: false,
            agent_picker_open: false,
            announced: HashSet::new(),
            _tick: tick,
        }
    }

    /// Called by the app on every frame it hosts the view. True when the cached page must be
    /// redrawn because the window glass or the light appearance changed.
    pub(crate) fn sync(&mut self, glass: bool, light: bool, modal: ModalPalette) -> bool {
        let appearance = (glass, light);
        if self.appearance == Some(appearance) && self.palette.is_some() {
            return false;
        }
        self.appearance = Some(appearance);
        self.palette = Some(DotsPalette::resolve(glass, modal));
        true
    }

    /// The settings or theme the page draws from changed.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn appearance_changed(&mut self) {
        self.appearance = None;
    }

    pub(crate) fn showing(&self, cx: &Context<Self>) -> bool {
        self.app
            .upgrade()
            .is_some_and(|app| app.read(cx).open_dots_view_showing())
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.showing(cx) {
            return;
        }
        if self
            .last_load
            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL)
        {
            self.load(cx);
        }
        if let OpenDotsTarget::Dot { dot_id, tab } = self.page.clone() {
            match tab {
                OpenDotsDotTab::Computer => self.poll_screen_if_due(&dot_id, cx),
                OpenDotsDotTab::Chats => {
                    if let Some(project_id) = self.dot_project(&dot_id) {
                        let due = self
                            .chats_read_at
                            .get(&project_id)
                            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL);
                        if due {
                            self.load_chats(project_id, cx);
                        }
                    }
                }
                OpenDotsDotTab::Setup => {}
            }
        }
    }

    pub(crate) fn dot_project(&self, dot_id: &str) -> Option<String> {
        self.snapshot
            .as_ref()?
            .dot(dot_id)
            .map(|dot| dot.project_id.clone())
            .filter(|project| !project.is_empty())
    }

    /// Reads the snapshot. A read already in flight is followed by one more, so a burst of
    /// changes costs at most two reads.
    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            self.reload_wanted = true;
            return;
        }
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let result = open_dots_rpc("snapshot", json!({})).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.last_load = Some(Instant::now());
                match result.and_then(|value| {
                    serde_json::from_value::<DotsSnapshot>(value).map_err(|error| {
                        format!("OpenDots answered in a form this app cannot read: {error}")
                    })
                }) {
                    Ok(snapshot) => {
                        this.load_error = None;
                        this.apply_snapshot(snapshot, cx);
                    }
                    // A failed refresh keeps what is on screen.
                    Err(error) if this.snapshot.is_none() => this.load_error = Some(error),
                    Err(_) => {}
                }
                if std::mem::take(&mut this.reload_wanted) {
                    this.load(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_snapshot(&mut self, snapshot: DotsSnapshot, cx: &mut Context<Self>) {
        let first = self.snapshot.is_none();
        let mut fresh = Vec::new();
        for approval in snapshot.pending_approvals() {
            if self.announced.insert(approval.id.clone()) && !first {
                let name = snapshot
                    .dot(&approval.dot_id)
                    .map(|dot| dot.name.clone())
                    .unwrap_or_else(|| "A dot".to_string());
                fresh.push((
                    format!(
                        "{name} {}",
                        match approval.kind.as_str() {
                            "question" => "has a question",
                            "connect" => "wants to connect an app",
                            _ => "needs your approval",
                        }
                    ),
                    approval.title.clone(),
                    approval
                        .session_id
                        .clone()
                        .map(|session| (approval.project_id.clone(), session)),
                ));
            }
        }
        for update in &snapshot.updates {
            let Some(title) = update
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
            else {
                continue;
            };
            if self.announced.insert(update.id.clone()) && !first && !update.read {
                let name = snapshot
                    .dot(&update.dot_id)
                    .map(|dot| dot.name.clone())
                    .unwrap_or_else(|| "A dot".to_string());
                fresh.push((
                    title,
                    format!("From {name}"),
                    update
                        .session_id
                        .clone()
                        .map(|session| (update.project_id.clone(), session)),
                ));
            }
        }
        // A dot that is gone takes its page with it.
        if let OpenDotsTarget::Dot { dot_id, .. } = &self.page
            && snapshot.dot(dot_id).is_none()
            && self
                .snapshot
                .as_ref()
                .is_some_and(|old| old.dot(dot_id).is_some())
        {
            self.page = OpenDotsTarget::Home;
        }
        self.snapshot = Some(snapshot);
        if !fresh.is_empty() {
            let app = self.app.clone();
            cx.defer(move |cx| {
                let _ = app.update(cx, |app, cx| {
                    for (title, body, session) in fresh {
                        app.open_dots_notify(&title, &body, session, cx);
                    }
                });
            });
        }
    }

    /// Shows `target`. Called by the app (the sidebar's rows, a notification) and by the page's
    /// own links.
    pub(crate) fn navigate(&mut self, target: OpenDotsTarget, cx: &mut Context<Self>) {
        let same_dot = matches!(
            (&self.page, &target),
            (OpenDotsTarget::Dot { dot_id: old, .. }, OpenDotsTarget::Dot { dot_id: new, .. }) if old == new
        );
        if !same_dot {
            self.confirm_delete = false;
            self.agent_picker_open = false;
            self.skill_open = None;
            self.reset_computer(cx);
            self.forms.trigger_flow = None;
            self.forms.url = None;
            self.forms.url_subscription = None;
        }
        if self.page != target {
            self.scroll
                .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        self.notice = None;
        self.page = target;
        if let OpenDotsTarget::Dot { dot_id, tab } = self.page.clone() {
            match tab {
                OpenDotsDotTab::Chats => {
                    if let Some(project_id) = self.dot_project(&dot_id) {
                        self.load_chats(project_id, cx);
                    }
                }
                OpenDotsDotTab::Computer => {
                    self.computer.next_poll_at = None;
                    self.load_files(&dot_id, cx);
                }
                OpenDotsDotTab::Setup => {}
            }
        }
        self.load(cx);
        cx.notify();
    }

    /// Runs one OpenDots method. `key` names the button it dims while in flight; `on_result` gets
    /// the answer (a failure goes to the page notice unless `on_result` takes it). The snapshot is
    /// read again after every call.
    pub(crate) fn call(
        &mut self,
        key: impl Into<String>,
        method: &'static str,
        params: Value,
        on_result: impl FnOnce(&mut Self, Result<Value, String>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        if !self.busy.insert(key.clone()) {
            return;
        }
        self.notice = None;
        cx.spawn(async move |this, cx| {
            let result = open_dots_rpc(method, params).await;
            let _ = this.update(cx, |this, cx| {
                this.busy.remove(&key);
                on_result(this, result, cx);
                if method != "browserInput" && method != "browserScreenshot" {
                    this.load(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// [`Self::call`] whose failure shows in the page notice.
    pub(crate) fn act(
        &mut self,
        key: impl Into<String>,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
    ) {
        self.call(
            key,
            method,
            params,
            |this, result, _| {
                if let Err(error) = result {
                    this.notice = Some(error);
                }
            },
            cx,
        );
    }

    /// How many cards a grid row holds at the page's width.
    pub(crate) fn grid_columns(&self) -> u16 {
        if self.page_width.get() >= 640.0 { 2 } else { 1 }
    }

    pub(crate) fn is_busy(&self, key: &str) -> bool {
        self.busy.contains(key)
    }

    /// Starts a conversation with a dot and opens it.
    pub(crate) fn start_chat(
        &mut self,
        dot_id: String,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut params = json!({ "dotId": dot_id });
        if let Some(message) = message.filter(|message| !message.trim().is_empty()) {
            params["message"] = Value::String(message);
        }
        self.call(
            format!("startChat:{dot_id}"),
            "startChat",
            params,
            |this, result, cx| match result {
                Ok(value) => this.focus_session_from(&value, cx),
                Err(error) => this.notice = Some(error),
            },
            cx,
        );
    }

    /// Opens the session a `startChat`, `startTeamChat` or `runRoutineNow` answered with.
    pub(crate) fn focus_session_from(&mut self, value: &Value, cx: &mut Context<Self>) {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        if let (Some(project_id), Some(session_id)) = (text("projectId"), text("sessionId")) {
            self.open_session(project_id, session_id, true, cx);
        }
    }

    /// Opens one of the dot's existing conversations.
    pub(crate) fn focus_session(
        &mut self,
        project_id: String,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        self.open_session(project_id, session_id, false, cx);
    }

    /// `created`: the session was just started here, so its row may not have reached the store.
    fn open_session(
        &mut self,
        project_id: String,
        session_id: String,
        created: bool,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        cx.defer(move |cx| {
            let _ = app.update(cx, |app, cx| {
                app.open_dots_focus_session(&project_id, &session_id, created, cx)
            });
        });
    }

    pub(crate) fn open_url(&mut self, url: String, cx: &mut Context<Self>) {
        let app = self.app.clone();
        cx.defer(move |cx| {
            let _ = app.update(cx, |app, cx| app.open_dots_open_url(&url, cx));
        });
    }

    pub(crate) fn open_path(&mut self, path: String, reveal: bool, cx: &mut Context<Self>) {
        let app = self.app.clone();
        cx.defer(move |cx| {
            let _ = app.update(cx, |app, cx| app.open_dots_open_path(&path, reveal, cx));
        });
    }

    /// A method that answers `{ url }` to open in the user's browser (Composio sign-in, app
    /// connections). A `null` url means there is nothing to open (already signed in).
    pub(crate) fn call_and_open_url(
        &mut self,
        key: impl Into<String>,
        method: &'static str,
        params: Value,
        cx: &mut Context<Self>,
    ) {
        self.call(
            key,
            method,
            params,
            |this, result, cx| match result {
                Ok(value) => {
                    if let Some(url) = value.get("url").and_then(Value::as_str) {
                        this.open_url(url.to_string(), cx);
                    }
                }
                Err(error) => this.notice = Some(error),
            },
            cx,
        );
    }

    /// The dot project's conversations, for the Chats tab.
    pub(crate) fn load_chats(&mut self, project_id: String, cx: &mut Context<Self>) {
        if !self.chats_loading.insert(project_id.clone()) {
            return;
        }
        self.chats_read_at
            .insert(project_id.clone(), Instant::now());
        cx.spawn(async move |this, cx| {
            let result = gx_rpc(
                None,
                "/api/listSessions",
                json!({ "projectId": project_id, "includeStopped": false }),
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                this.chats_loading.remove(&project_id);
                if let Ok(value) = result {
                    let chats = value
                        .get("sessions")
                        .and_then(Value::as_array)
                        .map(|rows| rows.iter().filter_map(DotChat::from_session_row).collect())
                        .unwrap_or_default();
                    this.chats.insert(project_id, chats);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
