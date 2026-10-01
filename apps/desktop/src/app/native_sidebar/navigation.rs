use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::ElementExt as _;
use gpui_component::h_flex;
use gpui_component::tooltip::ManagedTooltipExt as _;
use gpui_component::tooltip::ManagedTooltipPlacement;
use serde_json::json;

use super::appearance::SidebarAppearance;
use super::menu_state::SidebarMenuState;
use crate::app::consts::*;
use crate::app::helpers::*;
use crate::app::render::window_drag_region::window_drag_region;
use crate::*;

impl GhostexGpuiApp {
    /// CDXC:Sidebar 2026-09-19 DECISION:
    /// User: the Search row and the Commands row are each one pixel taller.
    /// Both rows keep those heights now that the hairlines are gone; the pixel each border used to
    /// take out of the border-box went back into the padding, so neither row's content moved.
    pub(crate) fn render_native_sidebar_navigation(
        &self,
        appearance: &SidebarAppearance,
        footer: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let scale = appearance.scale;
        let label = if footer { "Commands" } else { "Search" };
        let action_id = if footer { "commands" } else { "sessions" };
        let snapshot = self
            .native_sidebar
            .snapshot
            .as_ref()
            .expect("navigation follows a snapshot");
        let shortcut = if footer {
            &snapshot.commands_shortcut
        } else {
            &snapshot.search_shortcut
        };
        let mut more_menu = snapshot.more_menu.clone();
        /*
        CDXC:Sidebar 2026-09-20 DECISION:
        User: with the titlebar row deleted, the sidebar's Search row is what sits in the window's
        top-left corner, so it reserves the macOS traffic lights and is the window's drag handle
        there. It does so only while the sidebar is docked: the hover-reveal panel renders this same
        row below the workarea header, where there are no lights to clear.
        CDXC:Sidebar 2026-09-23 DECISION:
        User: on Windows and Linux too, the Toggle sidebar and Agents Panel buttons sit at the top
        left of the sidebar, and dragging the sidebar's top row moves the window. So this row owns
        the corner on every OS; only the traffic-light reserve is macOS's own
        (`SIDEBAR_TOGGLE_LEADING_X`). This supersedes the 2026-09-20 rule that kept the row a plain
        row off macOS, with both toggles in the workarea header.
        CDXC:Sidebar 2026-09-23 WHY:
        On Windows WM_NCHITTEST reports the Drag region as caption wherever no interactive child
        occludes it, and a caption press never reaches an `on_click`, so every button in this row
        occludes there.
        */
        let owns_window_corner = !footer && !self.sidebar_collapsed;
        /*
        CDXC:Sidebar 2026-09-21 DECISION:
        User: once the sidebar is narrower than its row's threshold (`SIDEBAR_COMPACT_COMMANDS_WIDTH`), the Search and the Commands rows drop their label and their shortcut hint and become icon-only buttons with the
        same icon they already show, the label and shortcut move into the tooltip ("Search (⌘P)",
        "Commands (⌘⇧P)"), and every button in both rows aligns right, against the sidebar's
        trailing edge. This supersedes the 2026-09-20 rule that aligned them left.
        CDXC:Sidebar 2026-09-28 DECISION:
        User: the Search button is always icon-only, at every sidebar width, with the label and
        shortcut in its tooltip. Only the Commands row still switches at its width threshold.
        */
        let compact = !footer || self.sidebar_width < SIDEBAR_COMPACT_COMMANDS_WIDTH * scale;
        /*
        CDXC:Sidebar 2026-09-23 DECISION:
        User: when the top row has no room for all its buttons, Search and Notifications leave the
        row and become the top two items of the sidebar menu, with a separator below them. They
        move together, and the menu button always stays because it now holds both. This
        supersedes the 2026-09-20 rule that kept Search and dropped the bell first, then the menu.
        */
        let compact_button_slot = 38.0 * scale;
        /*
        CDXC:Sidebar 2026-09-21 DECISION:
        User: the Toggle sidebar button sits left of Search, so it stays in exactly the same spot
        always. While the sidebar is docked this row owns the window's top-left corner, so it draws
        the button (right after the traffic lights on macOS) at the unscaled x the collapsed
        workarea header draws it (`SIDEBAR_TOGGLE_LEADING_X`); the header only draws it while
        collapsed.
        */
        // Two buttons of the same shape: Toggle sidebar and the Agents Panel toggle after it.
        let sidebar_toggle_width = if owns_window_corner {
            2.0 * (TITLEBAR_BUTTON_HORIZONTAL_PADDING * 2.0
                + TITLEBAR_SIDEBAR_COLLAPSE_ICON_LEFT_OFFSET
                + TITLEBAR_SIDEBAR_COLLAPSE_ICON_SIZE)
                + 4.0
        } else {
            0.0
        };
        // With Spaces on, the Hermes and Dots buttons are the Space row's last slots instead.
        let bots_toggle = (!footer && snapshot.bots_enabled && !snapshot.spaces_enabled)
            .then_some(snapshot.bots_mode);
        let dots_toggle = (!footer && snapshot.dots_enabled && !snapshot.spaces_enabled)
            .then_some(snapshot.dots_mode);
        let compact_room = self.sidebar_width
            - if owns_window_corner {
                SIDEBAR_TOGGLE_LEADING_X
            } else {
                5.0 * scale
            }
            - sidebar_toggle_width
            - bots_toggle.map_or(0.0, |_| (super::bots::BOTS_TOGGLE_WIDTH + 4.0) * scale)
            - dots_toggle.map_or(0.0, |_| (super::dots::DOTS_TOGGLE_WIDTH + 4.0) * scale)
            - 5.0 * scale;
        let bell_visible = !footer && self.titlebar_notification_bell_visible();
        let overflowed = !footer
            && compact
            && compact_room < (if bell_visible { 3.0 } else { 2.0 }) * compact_button_slot;
        if overflowed && let Some(items) = more_menu.as_array_mut() {
            let search_label = match shortcut.as_deref() {
                Some(shortcut) if !shortcut.is_empty() => format!("{label} ({shortcut})"),
                _ => label.to_owned(),
            };
            let mut leading = vec![json!({
                "label": search_label,
                "icon": "search",
                "command": {"type": "sidebarAction", "action": action_id},
            })];
            if bell_visible {
                let unread = self.notification_feed_state.unread_count;
                leading.push(json!({
                    "label": if unread > 0 {
                        format!("Notifications ({})", crate::notification_feed::notification_feed_badge_label(unread))
                    } else {
                        "Notifications".to_owned()
                    },
                    "icon": "bell",
                    "command": {"type": "openNotifications"},
                }));
            }
            leading.push(json!({"separator": true}));
            items.splice(0..0, leading);
        }
        let icon_path = if footer {
            "titlebar/bolt.svg"
        } else {
            BROWSER_ICON_SEARCH
        };
        let tooltip_label: gpui::SharedString = match shortcut.as_deref() {
            Some(shortcut) if !shortcut.is_empty() => format!("{label} ({shortcut})").into(),
            _ => label.into(),
        };
        let tooltip_delay = appearance.tooltip_delay;
        h_flex()
            .w_full()
            .h(px((if footer { 36.0 } else { 35.0 }) * scale))
            .pt(px(5.0 * scale))
            .pb(px(3.0 * scale))
            .when(!footer || compact, |row| {
                row.px(px(5.0 * scale)).gap(px(4.0 * scale))
            })
            .when(compact, |row| row.justify_end())
            .when(owns_window_corner, |row| {
                window_drag_region(row.pl(px(SIDEBAR_TOGGLE_LEADING_X)))
            })
            .overflow_hidden()
            /*
            CDXC:Sidebar 2026-09-20 DECISION:
            User: there is no rule under the Search row and none above the usage strip or the
            Commands row. The session list fades out at its bottom end only
            (native_sidebar/scroll_fade.rs) and nothing shades its top, so these rows draw no border
            at all. This supersedes the 2026-09-19 rule that framed the list with a hairline at each
            end.
            */
            .flex_shrink_0()
            .text_color(titlebar_active_text_color().opacity(0.52))
            /*
            CDXC:Sidebar 2026-09-23 DECISION:
            User: the Toggle sidebar and Agents Panel buttons at the top of the sidebar have the
            same color as the notification bell there, so this row draws their icons in the
            sidebar's muted color rather than the header's bright one.
            */
            .when(owns_window_corner, |row| {
                // The row's unscaled leading padding puts the button on the header's x. The spacer
                // keeps it left of a compact row, whose other buttons align right.
                row.child(
                    div()
                        .flex_shrink_0()
                        .child(self.render_sidebar_collapse_button(Some(appearance.muted), cx)),
                )
                // Right of Toggle sidebar, as in the collapsed header (workarea_header/breadcrumb.rs),
                // which puts it 4pt after that button: the margin tops the scaled row gap up to it.
                .child(
                    div().flex_shrink_0().ml(px(4.0 - 4.0 * scale)).child(
                        self.render_workarea_header_agents_toggle(Some(appearance.muted), cx),
                    ),
                )
                .when(compact, |row| row.child(div().flex_1()))
            })
            .when_some(bots_toggle, |row, bots_mode| {
                row.child(self.render_native_sidebar_bots_toggle(bots_mode, appearance, cx))
            })
            .when_some(dots_toggle, |row, dots_mode| {
                row.child(self.render_native_sidebar_dots_toggle(dots_mode, appearance, cx))
            })
            /*
            CDXC:Sidebar 2026-09-20 WHY:
            The sidebar can be dragged down to `SIDEBAR_MIN_WIDTH`, and this row now carries more
            than it used to: the macOS traffic-light reserve, the notification bell and the Settings
            gear. Without a clip and without fixed-size trailing controls the glyphs simply painted
            over each other there, so the label is the one thing that shrinks and is clipped, and
            everything beside it keeps its own box. Below the compact width the label stops being
            drawn at all rather than being clipped to nothing.
            */
            .when(!overflowed, |row| {
                row.child(if compact {
                    div()
                        .id(format!("native-sidebar-{label}"))
                        .role(gpui::Role::Button)
                        .aria_label(tooltip_label.clone())
                        .when(cfg!(target_os = "windows"), |button| button.occlude())
                        .h(px(28.0 * scale))
                        .w(px(34.0 * scale))
                        .rounded(px(5.0 * scale))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .hover(|row| row.bg(appearance.hover))
                        .child(titlebar_svg_icon(
                            icon_path,
                            15.0 * scale,
                            titlebar_active_text_color().opacity(0.52),
                        ))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            cx.stop_propagation();
                            app.dispatch_native_sidebar_ui(
                                json!({"type": "sidebarAction", "action": action_id}),
                                cx,
                            );
                        }))
                        .managed_discrete_tooltip_with_placement(
                            ManagedTooltipPlacement::Right,
                            tooltip_delay,
                            move |window, cx| titlebar_tooltip(tooltip_label.clone(), window, cx),
                        )
                        .into_any_element()
                } else {
                    h_flex()
                        .id(format!("native-sidebar-{label}"))
                        .role(gpui::Role::Button)
                        .aria_label(tooltip_label.clone())
                        .when(cfg!(target_os = "windows"), |button| button.occlude())
                        .flex_1()
                        .h(px((if footer { 28.0 } else { 27.0 }) * scale))
                        .min_w_0()
                        .overflow_hidden()
                        .pl(px((if footer { 12.0 } else { 7.0 }) * scale))
                        .pr(px(15.0 * scale))
                        .gap(px(11.0 * scale))
                        .cursor_default()
                        .hover(|row| row.text_color(titlebar_active_text_color()))
                        .child(div().flex().flex_shrink_0().items_center().child(
                            titlebar_svg_icon(
                                icon_path,
                                15.0 * scale,
                                titlebar_active_text_color().opacity(0.52),
                            ),
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(titlebar_active_text_color().opacity(0.38))
                                .text_size(px(11.0 * scale))
                                .child(shortcut.clone().unwrap_or_default()),
                        )
                        .on_click(cx.listener(move |app, _, _, cx| {
                            cx.stop_propagation();
                            app.dispatch_native_sidebar_ui(
                                json!({"type": "sidebarAction", "action": action_id}),
                                cx,
                            );
                        }))
                        .into_any_element()
                })
            })
            // CDXC:Notifications 2026-09-20 DECISION:
            // User: the notification bell sits in the sidebar's top row, before the sidebar menu button.
            .when(bell_visible && !overflowed, |row| {
                row.child(self.render_sidebar_notification_bell(appearance, cx))
            })
            .when(!footer, |row| {
                let more_open = self
                    .native_sidebar
                    .menu
                    .as_ref()
                    .is_some_and(SidebarMenuState::dropped_from_trigger);
                row.child(
                    div()
                        .id("native-sidebar-more")
                        .role(gpui::Role::Button)
                        .aria_label("Sidebar menu")
                        .aria_expanded(more_open)
                        .when(cfg!(target_os = "windows"), |button| button.occlude())
                        .h_full()
                        .w(px((if compact { 34.0 } else { 40.0 }) * scale))
                        .rounded(px(5.0 * scale))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .when(more_open, |row| row.bg(appearance.hover))
                        .hover(|row| row.bg(appearance.hover))
                        .child(titlebar_svg_icon(
                            "titlebar/menu-2.svg",
                            15.0 * scale,
                            appearance.muted,
                        ))
                        .on_prepaint({
                            let bounds = self.native_sidebar.more_button_bounds.clone();
                            move |painted, _, _| bounds.set(Some(painted))
                        })
                        .on_mouse_down(MouseButton::Left, {
                            let bounds = self.native_sidebar.more_button_bounds.clone();
                            cx.listener(move |app, event: &gpui::MouseDownEvent, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                                let trigger = bounds.get().unwrap_or_else(|| gpui::Bounds {
                                    origin: event.position,
                                    size: gpui::size(gpui::px(1.0), gpui::px(1.0)),
                                });
                                app.toggle_native_sidebar_more_menu(
                                    &more_menu, trigger, scale, window, cx,
                                );
                            })
                        }),
                )
            })
            /*
            CDXC:Sidebar 2026-09-21 DECISION:
            User: Settings gets a one-click gear immediately to the right of the Commands
            row, and the Commands row keeps its full-width shape and its shortcut hint
            rather than shrinking to an icon while the sidebar is wide enough. The gear is
            now the sidebar's only Settings entry: the sidebar menu no longer carries
            Settings or Hotkeys (packages/gx-core/src/sidebar_menu/navigation.rs). This
            supersedes the 2026-09-20 rule that kept both entries in the menu as a
            deliberate duplicate.
            */
            .when(footer, |row| {
                row.children(self.render_native_sidebar_usage_toggle(appearance, cx))
            })
            .when(footer, |row| {
                row.child(
                    div()
                        .id("native-sidebar-settings")
                        .role(gpui::Role::Button)
                        .aria_label("Settings")
                        .h(px(28.0 * scale))
                        .w(px(34.0 * scale))
                        // The compact row already insets its trailing edge like the Search row.
                        .mr(px((if compact { 0.0 } else { 6.0 }) * scale))
                        .rounded(px(5.0 * scale))
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .cursor_default()
                        .hover(|row| row.bg(appearance.hover))
                        .child(titlebar_svg_icon(
                            TITLEBAR_ICON_SETTINGS,
                            15.0 * scale,
                            appearance.muted,
                        ))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            cx.stop_propagation();
                            app.dispatch_native_sidebar_ui(
                                json!({"type": "sidebarAction", "action": "settings"}),
                                cx,
                            );
                        }))
                        .managed_discrete_tooltip_with_placement(
                            ManagedTooltipPlacement::Right,
                            appearance.tooltip_delay,
                            |window, cx| {
                                titlebar_tooltip(
                                    titlebar_tooltip_label("Settings", "openSettings"),
                                    window,
                                    cx,
                                )
                            },
                        ),
                )
            })
            .into_any_element()
    }
}
