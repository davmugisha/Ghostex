//! The OpenDots view's pages and the frame around them: a slim bar (OpenDots, where you are, New
//! dot, Team chat, Settings), the page notice, and one scrolling column.

mod approvals;
mod dot;
mod home;
mod new_dot;
mod settings;
mod setup;
mod team;
mod triggers;

use super::style::*;
use super::target::OpenDotsTarget;
use super::view::NativeOpenDotsView;
use crate::app::native_sidebar::dot_face::dots_mark_image;
use crate::app::window::native_modal_kit::ModalPalette;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Rgba, StatefulInteractiveElement as _, Styled as _, Window, div, img, px,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{h_flex, v_flex};

fn hex_of(color: gpui::Hsla) -> String {
    let rgba: Rgba = color.into();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(rgba.r),
        channel(rgba.g),
        channel(rgba.b)
    )
}

impl NativeOpenDotsView {
    fn render_nav(&self, p: &DotsPalette, cx: &mut Context<Self>) -> AnyElement {
        let crumb: Option<AnyElement> = match &self.page {
            OpenDotsTarget::Home => None,
            OpenDotsTarget::Dot { dot_id, .. } => self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.dot(dot_id))
                .map(|dot| {
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(face(&dot.look, dot.face_status(), 16.0))
                        .child(truncated(dot.name.clone()))
                        .into_any_element()
                }),
            OpenDotsTarget::NewDot => Some(div().child("New dot").into_any_element()),
            OpenDotsTarget::Settings => Some(div().child("Settings").into_any_element()),
            OpenDotsTarget::NewTeam => Some(div().child("New team chat").into_any_element()),
        };
        let needs_you = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.pending_approvals().count())
            .unwrap_or(0);
        let hover = p.hover;
        h_flex()
            .flex_shrink_0()
            .h(px(44.0))
            .px(px(12.0))
            .gap(px(6.0))
            .items_center()
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .id("dots-nav-home")
                    .role(gpui::Role::Button)
                    .aria_label("OpenDots home")
                    .h(px(28.0))
                    .px(px(8.0))
                    .gap(px(8.0))
                    .items_center()
                    .rounded(px(CONTROL_RADIUS))
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(OpenDotsTarget::Home, cx)))
                    .child(img(dots_mark_image(&hex_of(p.foreground), 18.0)).size(px(18.0)))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child("OpenDots"),
                    ),
            )
            .when_some(crumb, |this, crumb| {
                this.child(div().text_color(p.faint).text_size(px(13.0)).child("/"))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(13.0))
                            .text_color(p.foreground.opacity(0.85))
                            .child(crumb),
                    )
            })
            .child(div().flex_1())
            .when(needs_you > 0, |this| {
                this.child(
                    h_flex()
                        .id("dots-nav-needs-you")
                        .role(gpui::Role::Button)
                        .aria_label("Needs you")
                        .h(px(26.0))
                        .px(px(8.0))
                        .gap(px(6.0))
                        .items_center()
                        .rounded(px(CONTROL_RADIUS))
                        .bg(p.warning.opacity(0.14))
                        .text_size(px(12.0))
                        .text_color(p.warning)
                        .cursor_pointer()
                        .on_click(
                            cx.listener(|this, _, _, cx| this.navigate(OpenDotsTarget::Home, cx)),
                        )
                        .child(status_dot(p.warning, 6.0))
                        .child(format!("Needs you · {needs_you}")),
                )
            })
            .child(button(
                p,
                "dots-nav-new-dot",
                Some("New dot".into()),
                Some(ICON_PLUS),
                Tone::Quiet,
                true,
                false,
                |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::NewDot, cx),
                cx,
            ))
            .child(button(
                p,
                "dots-nav-team",
                Some("Team chat".into()),
                Some(ICON_TEAM),
                Tone::Quiet,
                true,
                false,
                |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::NewTeam, cx),
                cx,
            ))
            .child(button(
                p,
                "dots-nav-settings",
                None,
                Some(ICON_SETTINGS),
                Tone::Quiet,
                true,
                false,
                |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::Settings, cx),
                cx,
            ))
            .into_any_element()
    }

    fn render_notice(&self, p: &DotsPalette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notice = self.notice.clone()?;
        Some(
            h_flex()
                .mt(px(16.0))
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .gap(px(8.0))
                .items_center()
                .rounded(px(CONTROL_RADIUS))
                .border_1()
                .border_color(p.danger.opacity(0.4))
                .bg(p.danger.opacity(0.08))
                .child(icon(ICON_ALERT, 15.0, p.danger))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.0))
                        .text_color(p.danger)
                        .child(notice),
                )
                .child(remove_button(
                    p,
                    "dots-notice-dismiss",
                    "Dismiss",
                    |this: &mut Self, _, cx| {
                        this.notice = None;
                        cx.notify();
                    },
                    cx,
                ))
                .into_any_element(),
        )
    }

    /// The page before the first snapshot: a quiet sketch of Home, not a "Loading" sentence.
    fn render_skeleton(&self, p: &DotsPalette) -> AnyElement {
        let bar = |width: f32, height: f32| {
            div()
                .w(px(width))
                .max_w_full()
                .h(px(height))
                .rounded(px(6.0))
                .bg(p.selected)
        };
        v_flex()
            .w_full()
            .items_center()
            .gap(px(14.0))
            .pt(px(64.0))
            .child(div().size(px(96.0)).rounded_full().bg(p.selected))
            .child(bar(120.0, 12.0))
            .child(bar(320.0, 26.0))
            .child(bar(560.0, 96.0).mt(px(12.0)).rounded(px(CARD_RADIUS)))
            .child(
                h_flex()
                    .w_full()
                    .max_w(px(560.0))
                    .gap(px(12.0))
                    .mt(px(24.0))
                    .child(bar(274.0, 84.0).flex_1().rounded(px(CARD_RADIUS)))
                    .child(bar(274.0, 84.0).flex_1().rounded(px(CARD_RADIUS))),
            )
            .into_any_element()
    }

    fn render_load_error(
        &self,
        p: &DotsPalette,
        error: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .w_full()
            .items_center()
            .gap(px(12.0))
            .pt(px(96.0))
            .child(icon(ICON_ALERT, 24.0, p.muted))
            .child(heading(p, "OpenDots could not be read", 16.0))
            .child(body_text(p, error).text_center().max_w(px(420.0)))
            .child(button(
                p,
                "dots-retry",
                Some("Try again".into()),
                Some(ICON_REFRESH),
                Tone::Secondary,
                false,
                false,
                |this: &mut Self, _, cx| this.load(cx),
                cx,
            ))
            .into_any_element()
    }

    fn render_off(&self, p: &DotsPalette) -> AnyElement {
        v_flex()
            .w_full()
            .items_center()
            .gap(px(10.0))
            .pt(px(96.0))
            .child(heading(p, "OpenDots is off", 16.0))
            .child(body_text(p, "Turn it on in Settings > Extensions.").text_center())
            .into_any_element()
    }
}

impl Render for NativeOpenDotsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.run_pending_clears(window, cx);
        let p = self
            .palette
            .clone()
            .unwrap_or_else(|| DotsPalette::resolve(false, ModalPalette::resolve(false, None)));
        let max_width = match self.page {
            OpenDotsTarget::Home => 880.0,
            OpenDotsTarget::NewTeam => 720.0,
            _ => 1080.0,
        };
        let body: AnyElement = match (&self.snapshot, self.load_error.clone()) {
            (None, Some(error)) => self.render_load_error(&p, error, cx),
            (None, None) => self.render_skeleton(&p),
            (Some(snapshot), _) if !snapshot.enabled => self.render_off(&p),
            (Some(_), _) => match self.page.clone() {
                OpenDotsTarget::Home => self.render_home(&p, window, cx),
                OpenDotsTarget::Dot { dot_id, tab } => {
                    self.render_dot_page(&p, &dot_id, tab, window, cx)
                }
                OpenDotsTarget::NewDot => self.render_new_dot(&p, window, cx),
                OpenDotsTarget::Settings => self.render_settings(&p, window, cx),
                OpenDotsTarget::NewTeam => self.render_new_team(&p, window, cx),
            },
        };
        let notice = self.render_notice(&p, cx);
        let width = self.page_width.clone();
        let view_id = cx.entity_id();
        v_flex()
            .id("ghostex-open-dots")
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(p.page)
            .text_color(p.foreground)
            .child(self.render_nav(&p, cx))
            .child(
                div()
                    .id("dots-page-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .vertical_scrollbar(&self.scroll)
                    .child(
                        v_flex()
                            .relative()
                            .w_full()
                            .max_w(px(max_width))
                            .mx_auto()
                            .px(px(24.0))
                            .pb(px(56.0))
                            // The column's width picks one or two columns for the grids; a change
                            // draws the page again.
                            .child(
                                gpui::canvas(
                                    move |bounds, _, cx| {
                                        let measured = f32::from(bounds.size.width);
                                        if (width.get() - measured).abs() > 1.0 {
                                            width.set(measured);
                                            cx.notify(view_id);
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .children(notice)
                            .child(body),
                    ),
            )
    }
}
