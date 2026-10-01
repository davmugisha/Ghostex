//! The app side of the OpenDots view on the desktop: opening it on a page, drawing the cached page
//! in the view panel, reading the snapshot afresh when it opens or gxserver says it changed, and
//! what the page asks the app to do (open a conversation, a link or a file, post a notification).
//! The GPUI web build answers the same calls in `apps/gpui-web/src/app/web_host/open_dots.rs`.

use super::target::OpenDotsTarget;
use super::view::NativeOpenDotsView;
use crate::GhostexGpuiApp;
use crate::app::helpers::*;
use crate::app::model::*;
use ghostex_gx_core::SessionKey;
use gpui::{
    AnyElement, AnyView, AppContext as _, Entity, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ParentElement as _, StyleRefinement, Styled as _, Window, div,
};

impl GhostexGpuiApp {
    fn native_open_dots_view(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<NativeOpenDotsView> {
        if let Some(view) = self.native_open_dots.clone() {
            return view;
        }
        let app = cx.weak_entity();
        let view = cx.new(|cx| NativeOpenDotsView::new(app, cx));
        self.native_open_dots = Some(view.clone());
        view
    }

    /// Opens the OpenDots view (as a view tab) on `target`.
    pub(crate) fn open_open_dots_view(
        &mut self,
        target: OpenDotsTarget,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let view = self.native_open_dots_view(cx);
        view.update(cx, |view, cx| view.navigate(target, cx));
        if !self.open_view_tab(TitlebarMode::OpenDots, window, cx) {
            self.dispatch_gpui_workspace_action_toast(
                "info",
                "OpenDots is off",
                "Turn it on in Settings > Extensions.",
                cx,
            );
        }
    }

    /// The OpenDots view is the view showing.
    pub(crate) fn open_dots_view_showing(&self) -> bool {
        self.active_mode == TitlebarMode::OpenDots
    }

    /// The view became the one showing (`set_active_mode`): its snapshot is read afresh.
    pub(crate) fn native_open_dots_opened(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(view) = &self.native_open_dots {
            view.update(cx, |view, cx| view.load(cx));
        }
    }

    /// gxserver's `openDotsChanged` event (gx_store/host.rs).
    pub(crate) fn native_open_dots_changed(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(view) = &self.native_open_dots {
            view.update(cx, |view, cx| {
                if view.showing(cx) {
                    view.load(cx);
                }
            });
        }
    }

    /// The view panel's OpenDots page, created (and loaded) on its first draw.
    ///
    /// CDXC:OpenDots 2026-10-01 WHY:
    /// The page is a cached view like Automate's and the Bot automations feed's, so terminal output and other app redraws do not rebuild it; it redraws when it notifies itself or the window glass or appearance changed.
    pub(crate) fn render_native_open_dots_surface(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let fresh = self.native_open_dots.is_none();
        let view = self.native_open_dots_view(cx);
        if fresh {
            view.update(cx, |view, cx| view.load(cx));
        }
        let glass = window_glass_active_in(window);
        let light = CHROME_LIGHT_APPEARANCE.load(std::sync::atomic::Ordering::Relaxed);
        let palette = self.gpui_native_modal_palette();
        if view.update(cx, |view, _| view.sync(glass, light, palette)) {
            window.render_view_this_frame(view.entity_id());
        }
        div()
            .id("ghostex-gpui-native-open-dots-host")
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.focus_project_editor_surface(TitlebarMode::OpenDots, window, cx);
                }),
            )
            .child(AnyView::from(view).cached(StyleRefinement::default().size_full()))
            .into_any_element()
    }

    /// The settings or theme the page draws from changed.
    pub(crate) fn native_open_dots_notify_appearance(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(view) = self.native_open_dots.as_ref() {
            view.update(cx, |view, cx| {
                view.appearance_changed();
                cx.notify();
            });
        }
    }

    /// Opens one of a dot's conversations beside the view, as a sidebar row click does. A session
    /// the page just started (`created`) may not have reached the store yet, so it is opened the way
    /// a create opens its new session.
    pub(crate) fn open_dots_focus_session(
        &mut self,
        project_id: &str,
        session_id: &str,
        created: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if created {
            self.gx_store_focus_created_session(project_id, session_id, true, None, cx);
        } else {
            let row_id = SessionKey::local(project_id, session_id).to_sidebar_session_id();
            self.gx_store_focus_activated_session(&row_id, cx);
        }
        // An expanded view panel hides the sessions column; the conversation must be on screen.
        if !self.agents_workspace_visible() {
            self.toggle_view_panel_maximized(cx);
        }
        cx.notify();
    }

    pub(crate) fn open_dots_open_url(&mut self, url: &str, cx: &mut gpui::Context<Self>) {
        if let Err(error) = gpui_open_external_http_url(url) {
            self.dispatch_gpui_workspace_action_toast(
                "warning",
                "Could not open the link",
                &error,
                cx,
            );
        }
    }

    /// Opens a dot's file with its default app, or shows it in Finder when `reveal`.
    pub(crate) fn open_dots_open_path(
        &mut self,
        path: &str,
        reveal: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let result = if reveal {
            gpui_reveal_path_in_finder(std::path::Path::new(path))
        } else {
            gpui_spawn_os_open(std::ffi::OsStr::new(path))
        };
        if let Err(error) = result {
            self.dispatch_gpui_workspace_action_toast(
                "warning",
                "Could not open the file",
                &error,
                cx,
            );
        }
    }

    /// A dot needs you or posted a titled update. The banner is the session attention banner
    /// (one per lead window, gated by the same setting); clicking it opens the conversation.
    ///
    /// CDXC:OpenDots 2026-10-01 WHY:
    /// Ghostex already has one notification path users control (`showMacOSAttentionNotifications`), so OpenDots reuses it instead of adding a second switch; where the platform has no banner the page posts an in-app toast.
    pub(crate) fn open_dots_notify(
        &mut self,
        title: &str,
        body: &str,
        session: Option<(String, String)>,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.is_lead_window() || !gpui_macos_attention_notifications_enabled() {
            return;
        }
        if cfg!(target_os = "macos") {
            let session_id = session
                .map(|(project_id, session_id)| {
                    SessionKey::local(project_id, session_id).to_sidebar_session_id()
                })
                .unwrap_or_default();
            self.deliver_gpui_macos_session_attention_notification(
                GpuiSessionAttentionNotificationCandidate {
                    body: body.to_string(),
                    icon_data_url: None,
                    session_id,
                    title: title.to_string(),
                },
                cx,
            );
        } else {
            self.dispatch_gpui_workspace_action_toast("info", title, body, cx);
        }
    }
}
