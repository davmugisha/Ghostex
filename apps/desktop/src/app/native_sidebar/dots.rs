//! The Dots button that swaps the sidebar between the projects and the dots (OpenDots), and what a
//! dot row draws in place of a project's icon and status: the dot's face and a status dot.
//!
//! CDXC:OpenDots 2026-10-01 WHY:
//! Built like the Hermes button (bots.rs): with Spaces on, the Dots entry is the last slot of the Space row (after the Hermes slot, behind the same hairline) and can't be edited, deleted or dragged like a Space; with Spaces off it is a toggle in the sidebar's top row. It exists only while OpenDots is on in Settings > Extensions, draws the three-circle dots mark in Open Dot's brand blue, and draws a back arrow while Dots is showing so it reads as the way back to the projects.
//! SEE-ALSO: packages/gx-core/src/sidebar_view/assemble.rs (what Dots mode lists), open_dots_row.rs (the pinned OpenDots row), dots_host.rs (where the OpenDots view opens).

use ghostex_gx_core::open_dots::DotFaceStatus;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, img, px, rgb,
};
use gpui_component::tooltip::ManagedTooltipExt as _;
use gpui_component::tooltip::ManagedTooltipPlacement;
use serde_json::{Value, json};
use std::sync::Arc;

use super::appearance::SidebarAppearance;
use super::model::NativeSidebarGroup;
use crate::GhostexGpuiApp;
use crate::app::helpers::*;

/// The top-row toggle's width, which the row's compaction math counts.
pub(crate) const DOTS_TOGGLE_WIDTH: f32 = 30.0;
/// Open Dot's brand blue, which the dots mark is drawn in on the Space row and the OpenDots row.
pub(crate) const DOTS_BRAND_HEX: &str = "#51a2ff";

impl GhostexGpuiApp {
    /// Swaps the list between the projects and the dots.
    pub(crate) fn toggle_native_sidebar_dots_mode(&mut self, cx: &mut gpui::Context<Self>) {
        let entering = !self
            .native_sidebar
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.dots_mode);
        self.dispatch_native_sidebar_ui(
            json!({"type": "setSidebarMode", "mode": if entering { "dots" } else { "projects" }}),
            cx,
        );
    }

    /// The last slot of the Space row: a Space-sized tile, selected while Dots is showing. It draws
    /// its own hairline only when no Hermes slot sits before it.
    pub(crate) fn render_native_sidebar_dots_space_slot(
        &self,
        dots_mode: bool,
        with_divider: bool,
        appearance: &SidebarAppearance,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let scale = appearance.scale;
        let (selected_background, selected_outline) =
            super::selectors::space_tile_selected_colors(appearance);
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(4.0 * scale))
            .when(with_divider, |slot| {
                slot.child(
                    div()
                        .w(px(1.0))
                        .h(px(16.0 * scale))
                        .bg(appearance.foreground.opacity(0.12)),
                )
            })
            .child(
                self.native_sidebar_dots_button(
                    "native-sidebar-dots-space-slot",
                    dots_mode,
                    DOTS_BRAND_HEX.to_string(),
                    appearance,
                    cx,
                )
                .size(px(28.0 * scale))
                .rounded(px(6.0 * scale))
                .border_1()
                .border_color(appearance.foreground.opacity(0.08))
                .cursor_pointer()
                .when(dots_mode, |slot| {
                    slot.bg(selected_background)
                        .border_color(selected_outline)
                        .when(appearance.light && !appearance.glass, |slot| {
                            slot.shadow_sm()
                        })
                }),
            )
            .into_any_element()
    }

    /// The top-row toggle, filled while Dots is showing.
    pub(crate) fn render_native_sidebar_dots_toggle(
        &self,
        dots_mode: bool,
        appearance: &SidebarAppearance,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let scale = appearance.scale;
        self.native_sidebar_dots_button(
            "native-sidebar-dots-toggle",
            dots_mode,
            hsla_hex(appearance.muted),
            appearance,
            cx,
        )
        .when(cfg!(target_os = "windows"), |button| button.occlude())
        .h(px(28.0 * scale))
        .w(px(DOTS_TOGGLE_WIDTH * scale))
        .rounded(px(5.0 * scale))
        .cursor_default()
        .when(dots_mode, |button| button.bg(appearance.selected))
        .into_any_element()
    }

    /// What both entries share: the button's identity and state, the dots mark (a back arrow while
    /// Dots is showing), the toggle and its tooltip. `mark_hex` colors only the mark.
    fn native_sidebar_dots_button(
        &self,
        id: &'static str,
        dots_mode: bool,
        mark_hex: String,
        appearance: &SidebarAppearance,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let tooltip = if dots_mode {
            "Show projects"
        } else {
            "Show dots"
        };
        let scale = appearance.scale;
        div()
            .id(id)
            .role(gpui::Role::Button)
            .aria_label(tooltip)
            .aria_selected(dots_mode)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .hover(|button| button.bg(appearance.hover))
            .child(if dots_mode {
                titlebar_svg_icon(
                    "titlebar/arrow-left.svg",
                    15.0 * scale,
                    appearance.foreground,
                )
                .into_any_element()
            } else {
                img(super::dot_face::dots_mark_image(&mark_hex, 17.0 * scale))
                    .size(px(17.0 * scale))
                    .flex_shrink_0()
                    .into_any_element()
            })
            .on_click(cx.listener(|app, _, _, cx| {
                cx.stop_propagation();
                app.toggle_native_sidebar_dots_mode(cx);
            }))
            .managed_discrete_tooltip_with_placement(
                ManagedTooltipPlacement::Right,
                appearance.tooltip_delay,
                move |window, cx| titlebar_tooltip(gpui::SharedString::from(tooltip), window, cx),
            )
    }
}

impl NativeSidebarGroup {
    /// The dot's state, and `None` on every group that is not a dot's: the snapshot sets
    /// `dotStatus` on dot projects only (gx_store/sidebar_snapshot.rs).
    pub(crate) fn dot_status(&self) -> Option<DotFaceStatus> {
        self.project_context
            .as_ref()?
            .get("dotStatus")?
            .as_str()
            .map(DotFaceStatus::parse)
    }

    /// The dot's face for this row, `display_px` points wide; `None` on every group that is not a
    /// dot's.
    pub(crate) fn dot_face(&self, display_px: f32) -> Option<Arc<gpui::Image>> {
        let status = self.dot_status()?;
        let look = self
            .project_context
            .as_ref()
            .and_then(|project| project.get("dotLook"));
        Some(super::dot_face::dot_face_image(look, status, display_px))
    }

    /// How many approvals and questions of this dot wait for the user.
    pub(crate) fn dot_needs_you(&self) -> u64 {
        self.project_context
            .as_ref()
            .and_then(|project| project.get("dotNeedsYou"))
            .and_then(Value::as_u64)
            .unwrap_or_default()
    }
}

/// A dot row's status dot (working blue, waiting amber, paused grey, idle green), in the slot a
/// bot row draws its gateway dot, after the number of approvals and questions waiting for the user.
pub(crate) fn dot_status_badge(
    status: DotFaceStatus,
    needs_you: u64,
    appearance: &SidebarAppearance,
) -> AnyElement {
    let scale = appearance.scale;
    gpui_component::h_flex()
        .flex_shrink_0()
        .gap(px(5.0 * scale))
        .when(needs_you > 0, |badge| {
            badge.child(
                div()
                    .text_size(px(11.0 * scale))
                    .text_color(rgb(DotFaceStatus::Waiting.color_rgb()))
                    .child(needs_you.to_string()),
            )
        })
        .child(
            div()
                .size(px(7.0 * scale))
                .rounded_full()
                .bg(rgb(status.color_rgb())),
        )
        .into_any_element()
}

/// `#rrggbb` for a theme color, for the SVG marks that take a hex fill.
fn hsla_hex(color: gpui::Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b)
    )
}
