//! The OpenDots row pinned at the top of the Dots list, which opens the OpenDots view's Home (hand
//! off a task, approvals, your dots, latest updates).
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! Laid out like the Bots list's pinned Automations row (automations_row.rs), so OpenDots' home is one click from every dot, with a badge counting the approvals and questions every dot is waiting on, the number Open Dot shows on its own home.
//! SEE-ALSO: dots_host.rs (`open_dots_home_from_sidebar`), app/native_open_dots/ (the view).

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, img, px, rgb,
};
use gpui_component::h_flex;

use super::appearance::SidebarAppearance;
use crate::GhostexGpuiApp;

impl GhostexGpuiApp {
    /// Laid out like a dot row (project_header.rs) so the two line up, and selected while the
    /// OpenDots view shows.
    pub(crate) fn render_native_sidebar_open_dots_row(
        &self,
        needs_you: u64,
        appearance: &SidebarAppearance,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let scale = appearance.scale;
        let showing = self.open_dots_view_showing_for_sidebar();
        // A group's margins (render.rs) plus the header row's own inset (project_header.rs).
        h_flex()
            .id("native-sidebar-open-dots-row")
            .role(gpui::Role::Button)
            .aria_label(if needs_you > 0 {
                format!("OpenDots, {needs_you} waiting for you")
            } else {
                "OpenDots".to_string()
            })
            .aria_selected(showing)
            .relative()
            .h(px(30.0 * scale))
            .ml(px(21.0 * scale))
            .mr(px(8.0 * scale))
            .mb(px(10.0 * scale))
            .pl(px(5.0 * scale))
            .pr(px(6.0 * scale))
            .gap(px(10.0 * scale))
            .rounded(px(5.0 * scale))
            .cursor_default()
            .hover(|row| row.bg(appearance.session_hover))
            .when(showing, |row| {
                row.bg(appearance.selected)
                    .child(super::decorations::selected_outline(appearance))
            })
            .child(
                img(super::dot_face::dots_mark_image(
                    super::dots::DOTS_BRAND_HEX,
                    16.0 * scale,
                ))
                .size(px(16.0 * scale))
                .flex_shrink_0(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(gpui::FontWeight::LIGHT)
                    .child("OpenDots"),
            )
            .when(needs_you > 0, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .h(px(16.0 * scale))
                        .min_w(px(16.0 * scale))
                        .px(px(5.0 * scale))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(
                            ghostex_gx_core::open_dots::DotFaceStatus::Waiting.color_rgb()
                        ))
                        .text_color(rgb(0x1c1622))
                        .text_size(px(10.0 * scale))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(needs_you.to_string()),
                )
            })
            .on_click(cx.listener(|app, _, window, cx| {
                cx.stop_propagation();
                app.open_dots_home_from_sidebar(window, cx);
            }))
            .into_any_element()
    }
}
