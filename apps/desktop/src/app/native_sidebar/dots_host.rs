//! Where the Dots sidebar opens the OpenDots view (app/native_open_dots/). The GPUI web build
//! compiles this file too: its `open_open_dots_view` (web_host/open_dots.rs) shows the same view in
//! the page's work area.

use serde_json::Value;

use crate::GhostexGpuiApp;
use crate::app::native_open_dots::OpenDotsTarget;

impl GhostexGpuiApp {
    /// A sidebar `openDotsView` message (`{"page","dotId"?,"tab"?}`, gx_store/create/dot.rs).
    pub(crate) fn open_dots_view_from_sidebar(
        &mut self,
        message: &Value,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(target) = OpenDotsTarget::from_message(message) {
            self.open_open_dots_view(target, window, cx);
        }
    }

    /// The pinned OpenDots row: the view's Home.
    pub(crate) fn open_dots_home_from_sidebar(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_open_dots_view(OpenDotsTarget::Home, window, cx);
    }

    /// The OpenDots view is the one showing, which selects the pinned row.
    pub(crate) fn open_dots_view_showing_for_sidebar(&self) -> bool {
        self.open_dots_view_showing()
    }
}
