//! The OpenDots view in the browser: the desktop's own view (`app/native_open_dots/`), shown in
//! the page's work area in place of the open session, and the page's answers to what the view asks
//! the app to do. A link opens in a new tab; a dot's files and folders live on the computer
//! running Ghostex, so opening or revealing one answers with a toast.
//! SEE-ALSO: apps/desktop/src/app/native_open_dots/host.rs (the desktop's answers).

use ghostex_gx_core::SessionKey;
use gpui::{AnyElement, AppContext as _, Context, Entity, IntoElement, Window};

use crate::GhostexGpuiApp;
use crate::app::helpers::window_glass_active_in;
use crate::app::native_open_dots::{NativeOpenDotsView, OpenDotsTarget};

/// What the page holds for the view.
#[derive(Default)]
pub(crate) struct WebOpenDots {
    pub(crate) view: Option<Entity<NativeOpenDotsView>>,
    /// The work area shows the view instead of the open session. Opening a session clears it.
    pub(crate) showing: bool,
    /// The page's `?openDots=` link was followed.
    linked_opened: bool,
}

impl GhostexGpuiApp {
    fn web_open_dots_view(&mut self, cx: &mut Context<Self>) -> Entity<NativeOpenDotsView> {
        if let Some(view) = self.web_host.open_dots.view.clone() {
            return view;
        }
        let app = cx.weak_entity();
        let view = cx.new(|cx| NativeOpenDotsView::new(app, cx));
        self.web_host.open_dots.view = Some(view.clone());
        view
    }

    /// Opens the OpenDots view in the work area on `target`.
    pub(crate) fn open_open_dots_view(
        &mut self,
        target: OpenDotsTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.web_open_dots_view(cx);
        view.update(cx, |view, cx| view.navigate(target, cx));
        self.web_host.open_dots.showing = true;
        cx.notify();
    }

    /// `?openDots=home|new|settings|team|dot:<dotId>[:chats|computer|setup]` opens the view on that
    /// page once the page is connected: a deep link, and what the screenshot driver uses.
    pub(crate) fn web_open_linked_open_dots(&mut self, cx: &mut Context<Self>) {
        if self.web_host.open_dots.linked_opened {
            return;
        }
        self.web_host.open_dots.linked_opened = true;
        let Some(search) = web_sys::window().and_then(|window| window.location().search().ok())
        else {
            return;
        };
        let Some(value) = search
            .trim_start_matches('?')
            .split('&')
            .find_map(|pair| pair.strip_prefix("openDots="))
            .map(|value| {
                js_sys::decode_uri_component(value)
                    .map(String::from)
                    .unwrap_or_else(|_| value.to_string())
            })
        else {
            return;
        };
        let mut parts = value.split(':');
        let page = parts.next().unwrap_or("home");
        let message = serde_json::json!({
            "page": page,
            "dotId": parts.next().unwrap_or_default(),
            "tab": parts.next().unwrap_or("chats"),
        });
        let Some(target) = OpenDotsTarget::from_message(&message) else {
            return;
        };
        let view = self.web_open_dots_view(cx);
        view.update(cx, |view, cx| view.navigate(target, cx));
        self.web_host.open_dots.showing = true;
        cx.notify();
    }

    pub(crate) fn open_dots_view_showing(&self) -> bool {
        self.web_host.open_dots.showing
    }

    /// gxserver's `openDotsChanged` event (gx_store/host.rs).
    pub(crate) fn native_open_dots_changed(&mut self, cx: &mut Context<Self>) {
        if !self.web_host.open_dots.showing {
            return;
        }
        if let Some(view) = &self.web_host.open_dots.view {
            view.update(cx, |view, cx| view.load(cx));
        }
    }

    /// The work area's view, when it shows OpenDots.
    pub(crate) fn render_web_open_dots(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.web_host.open_dots.showing {
            return None;
        }
        let view = self.web_open_dots_view(cx);
        let glass = window_glass_active_in(window);
        let light =
            crate::app::helpers::CHROME_LIGHT_APPEARANCE.load(std::sync::atomic::Ordering::Relaxed);
        let palette = self.gpui_native_modal_palette();
        view.update(cx, |view, _| view.sync(glass, light, palette));
        Some(view.into_any_element())
    }

    /// The page opens a just-started session the same way as an existing one: the work area
    /// waits for its row.
    pub(crate) fn open_dots_focus_session(
        &mut self,
        project_id: &str,
        session_id: &str,
        _created: bool,
        cx: &mut Context<Self>,
    ) {
        self.web_open_session_in_work_area(SessionKey::local(project_id, session_id), false, cx);
    }

    pub(crate) fn open_dots_open_url(&mut self, url: &str, cx: &mut Context<Self>) {
        let opened = web_sys::window()
            .and_then(|window| window.open_with_url_and_target(url, "_blank").ok())
            .flatten()
            .is_some();
        if !opened {
            self.dispatch_gpui_workspace_action_toast(
                "warning",
                "Could not open the link",
                "The browser blocked the new tab. Allow pop-ups for this page and try again.",
                cx,
            );
        }
    }

    pub(crate) fn open_dots_open_path(
        &mut self,
        _path: &str,
        _reveal: bool,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_gpui_workspace_action_toast(
            "info",
            "Not available in the browser",
            "A dot's files are on the computer running Ghostex. Open them in the Ghostex app.",
            cx,
        );
    }

    /// The page has no system notifications; a toast says it instead.
    pub(crate) fn open_dots_notify(
        &mut self,
        title: &str,
        body: &str,
        _session: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_gpui_workspace_action_toast("info", title, body, cx);
    }
}
