//! The OpenDots view's colours and the small pieces every page shares (buttons, chips, cards,
//! sections, fields, the dot's face). The page tones come from the chat's `ChatAppearance` like the
//! Automate page, so OpenDots reads like the chat beside it; text fields, switches and segmented
//! controls are the native modal kit's.

use std::sync::Arc;

use crate::app::native_chat::appearance::ChatAppearance;
use crate::app::native_chat::markdown_style::{highlight_theme, text_style};
use crate::app::native_sidebar::dot_face::{dot_face_image, dot_face_image_for};
use crate::app::window::native_modal_kit::{
    ModalFieldSkin, ModalPalette, PressExt as _, modal_text_area_skinned, modal_text_input_skinned,
};
use ghostex_gx_core::open_dots::{DotFaceStatus, DotLook};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, Hsla, Img,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, div, img, px, rgb, svg,
};
use gpui_component::input::{InputState, TextareaState};
use gpui_component::text::TextViewStyle;
use gpui_component::{h_flex, v_flex};
use serde_json::Value;

pub(crate) const ICON_ARROW_UP: &str = "titlebar/arrow-up.svg";
pub(crate) const ICON_ARROW_UP_RIGHT: &str = "titlebar/arrow-up-right.svg";
pub(crate) const ICON_ARROW_LEFT: &str = "titlebar/arrow-left.svg";
pub(crate) const ICON_ARROW_RIGHT: &str = "titlebar/arrow-right.svg";
pub(crate) const ICON_PLUS: &str = "titlebar/plus.svg";
pub(crate) const ICON_X: &str = "titlebar/x.svg";
pub(crate) const ICON_CHECK: &str = "titlebar/check.svg";
pub(crate) const ICON_PLAY: &str = "titlebar/player-play.svg";
pub(crate) const ICON_PAUSE: &str = "titlebar/player-pause.svg";
pub(crate) const ICON_STOP: &str = "titlebar/player-stop.svg";
pub(crate) const ICON_REFRESH: &str = "titlebar/refresh.svg";
pub(crate) const ICON_RELOAD: &str = "titlebar/reload.svg";
pub(crate) const ICON_SETTINGS: &str = "titlebar/settings.svg";
pub(crate) const ICON_MESSAGE_PLUS: &str = "titlebar/message-plus.svg";
pub(crate) const ICON_MESSAGES: &str = "titlebar/messages.svg";
pub(crate) const ICON_DESKTOP: &str = "titlebar/device-desktop.svg";
pub(crate) const ICON_SLIDERS: &str = "titlebar/adjustments-horizontal.svg";
pub(crate) const ICON_KEY: &str = "titlebar/key.svg";
pub(crate) const ICON_LOCK: &str = "titlebar/lock.svg";
pub(crate) const ICON_FOLDER: &str = "titlebar/folder.svg";
pub(crate) const ICON_FOLDER_OPEN: &str = "titlebar/folder-open.svg";
pub(crate) const ICON_FILE: &str = "titlebar/file-text.svg";
pub(crate) const ICON_EXTERNAL: &str = "titlebar/external-link.svg";
pub(crate) const ICON_TEAM: &str = "titlebar/users-group.svg";
pub(crate) const ICON_CROWN: &str = "titlebar/coordinator-crown.svg";
pub(crate) const ICON_SHUFFLE: &str = "titlebar/wand.svg";
pub(crate) const ICON_POINTER: &str = "titlebar/pointer.svg";
pub(crate) const ICON_UNDO: &str = "titlebar/arrow-back-up.svg";
pub(crate) const ICON_TERMINAL: &str = "titlebar/terminal-2.svg";
pub(crate) const ICON_WORLD: &str = "titlebar/world.svg";
pub(crate) const ICON_CHEVRON_DOWN: &str = "titlebar/chevron-down.svg";
pub(crate) const ICON_ALERT: &str = "titlebar/alert-circle.svg";

pub(crate) const CARD_RADIUS: f32 = 12.0;
pub(crate) const CONTROL_RADIUS: f32 = 8.0;

/// Everything a page paints with, resolved when the appearance changes rather than per element.
#[derive(Clone)]
pub(crate) struct DotsPalette {
    pub(crate) modal: ModalPalette,
    /// The page fill; clear under window glass so the frosted work area shows through.
    pub(crate) page: Hsla,
    pub(crate) card: Hsla,
    pub(crate) raised: Hsla,
    pub(crate) border: Hsla,
    pub(crate) hover: Hsla,
    pub(crate) selected: Hsla,
    pub(crate) foreground: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) faint: Hsla,
    pub(crate) accent: Hsla,
    pub(crate) success: Hsla,
    pub(crate) warning: Hsla,
    pub(crate) danger: Hsla,
    pub(crate) primary: Hsla,
    pub(crate) primary_text: Hsla,
    pub(crate) markdown: TextViewStyle,
}

impl DotsPalette {
    pub(crate) fn resolve(glass: bool, modal: ModalPalette) -> Self {
        let chat = ChatAppearance::current(&Value::Null).on_window_glass(glass);
        let ink: Hsla = rgb(if chat.light { 0x000000 } else { 0xffffff }).into();
        let (page, card) = if glass {
            (
                gpui::transparent_black(),
                ink.opacity(if chat.light { 0.04 } else { 0.05 }),
            )
        } else {
            (chat.background, chat.card_panel)
        };
        let mut markdown = text_style(&chat);
        markdown.is_dark = !chat.light;
        markdown.highlight_theme = highlight_theme(chat.light);
        Self {
            modal,
            page,
            card,
            raised: ink.opacity(if chat.light { 0.05 } else { 0.07 }),
            border: if glass {
                ink.opacity(0.08)
            } else {
                chat.border
            },
            hover: ink.opacity(0.05),
            selected: ink.opacity(if glass { 0.09 } else { 0.07 }),
            foreground: chat.foreground,
            muted: chat.muted,
            faint: chat.foreground.opacity(0.4),
            accent: chat.accent,
            success: rgb(if chat.light { 0x15803d } else { 0x4ade80 }).into(),
            warning: rgb(if chat.light { 0xb45309 } else { 0xf59e0b }).into(),
            danger: chat.error(),
            primary: chat.foreground,
            primary_text: chat.background,
            markdown,
        }
    }

    pub(crate) fn status_color(&self, status: DotFaceStatus) -> Hsla {
        rgb(status.color_rgb()).into()
    }

    fn field_skin(&self) -> ModalFieldSkin {
        ModalFieldSkin {
            background: self.modal.raised,
            border: self.modal.hairline,
            focus_border: self.modal.focus_border,
            text_size: 14.0,
        }
    }
}

pub(crate) fn icon(path: &'static str, size: f32, color: Hsla) -> gpui::Svg {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(color)
}

/// A dot's face (`dot_face_image`), `size` points wide.
pub(crate) fn face(look: &Value, status: DotFaceStatus, size: f32) -> Img {
    img(dot_face_image(Some(look), status, size))
        .size(px(size))
        .flex_shrink_0()
}

/// The face for a look the page is editing.
pub(crate) fn face_for(look: &DotLook, size: f32) -> Img {
    let image: Arc<gpui::Image> = dot_face_image_for(look, DotFaceStatus::Idle, size);
    img(image).size(px(size)).flex_shrink_0()
}

/// Open Dot's mono eyebrow: a small uppercase label.
pub(crate) fn eyebrow(p: &DotsPalette, text: impl Into<SharedString>) -> Div {
    let text: SharedString = text.into();
    div()
        .text_size(px(11.0))
        .line_height(px(16.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted)
        .child(SharedString::from(text.to_uppercase()))
}

pub(crate) fn heading(p: &DotsPalette, text: impl Into<SharedString>, size: f32) -> Div {
    div()
        .text_size(px(size))
        .line_height(px(size * 1.25))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.foreground)
        .child(text.into())
}

pub(crate) fn body_text(p: &DotsPalette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(13.0))
        .line_height(px(19.0))
        .text_color(p.muted)
        .child(text.into())
}

/// Truncating single-line text.
pub(crate) fn truncated(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

pub(crate) fn status_dot(color: Hsla, size: f32) -> Div {
    div()
        .flex_shrink_0()
        .size(px(size))
        .rounded_full()
        .bg(color)
}

/// A small tinted label (`Needs you`, a rule's decision, `Connected`).
pub(crate) fn badge(text: impl Into<SharedString>, color: Hsla) -> Div {
    let text: SharedString = text.into();
    div()
        .flex_shrink_0()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(4.0))
        .bg(color.opacity(0.14))
        .text_size(px(10.5))
        .line_height(px(16.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(SharedString::from(text.to_uppercase()))
}

/// The page's surface: a rounded card on the page fill.
pub(crate) fn card(p: &DotsPalette) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .rounded(px(CARD_RADIUS))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
}

/// A card whose rows are split by hairlines.
pub(crate) fn rows_card(p: &DotsPalette, rows: Vec<AnyElement>) -> Div {
    let count = rows.len();
    card(p)
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(|(index, row)| {
            div()
                .w_full()
                .min_w_0()
                .when(index + 1 < count, |this| {
                    this.border_b_1().border_color(p.border)
                })
                .child(row)
        }))
}

/// Open Dot's dashed empty note.
pub(crate) fn empty_note(p: &DotsPalette, text: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .px(px(16.0))
        .py(px(18.0))
        .rounded(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(p.border)
        .text_center()
        .text_size(px(13.0))
        .line_height(px(19.0))
        .text_color(p.muted)
        .child(text.into())
}

/// Open Dot's settings `Section`: eyebrow, title and description over the section's controls.
pub(crate) fn section(
    p: &DotsPalette,
    eyebrow_text: &'static str,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    body: impl IntoElement,
) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(px(14.0))
        .pt(px(24.0))
        .pb(px(8.0))
        .border_t_1()
        .border_color(p.border)
        .child(
            v_flex()
                .gap(px(4.0))
                .child(eyebrow(p, eyebrow_text))
                .child(
                    div()
                        .text_size(px(15.0))
                        .line_height(px(20.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .child(title.into()),
                )
                .when_some(description, |this, description| {
                    this.child(body_text(p, description).max_w(px(620.0)))
                }),
        )
        .child(div().w_full().min_w_0().child(body))
}

/// A page header: eyebrow, title and an optional sentence (Open Dot's `PageHeader`).
pub(crate) fn page_header(
    p: &DotsPalette,
    eyebrow_text: &'static str,
    title: impl Into<SharedString>,
    description: Option<SharedString>,
) -> Div {
    v_flex()
        .gap(px(6.0))
        .pt(px(28.0))
        .pb(px(18.0))
        .child(eyebrow(p, eyebrow_text).text_color(p.accent))
        .child(heading(p, title, 24.0))
        .when_some(description, |this, description| {
            this.child(body_text(p, description).max_w(px(560.0)))
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Filled with the ink colour.
    Primary,
    /// Outlined.
    Secondary,
    /// No edge until hovered.
    Quiet,
    /// Outlined in the danger colour.
    Danger,
    /// Filled with the danger colour.
    DangerFilled,
}

/// A content-sized button, 32px tall (28 when `small`), with an optional leading icon. A button
/// with no label is square.
#[allow(clippy::too_many_arguments)]
pub(crate) fn button<V: 'static>(
    p: &DotsPalette,
    id: impl Into<ElementId>,
    label: Option<SharedString>,
    leading: Option<&'static str>,
    tone: Tone,
    small: bool,
    disabled: bool,
    on_click: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let height = if small { 28.0 } else { 32.0 };
    let (background, border, text, hover) = match tone {
        Tone::Primary => (
            p.primary,
            p.primary,
            p.primary_text,
            p.primary.opacity(0.85),
        ),
        Tone::Secondary => (p.raised, p.border, p.foreground, p.selected),
        Tone::Quiet => (
            gpui::transparent_black(),
            gpui::transparent_black(),
            p.foreground.opacity(0.8),
            p.hover,
        ),
        Tone::Danger => (
            gpui::transparent_black(),
            p.danger.opacity(0.45),
            p.danger,
            p.danger.opacity(0.12),
        ),
        Tone::DangerFilled => (p.danger, p.danger, p.primary_text, p.danger.opacity(0.85)),
    };
    let square = label.is_none();
    let aria: SharedString = label.clone().unwrap_or_else(|| {
        match leading {
            Some(ICON_SETTINGS) => "OpenDots settings",
            Some(ICON_REFRESH) => "Refresh",
            Some(ICON_ARROW_LEFT) => "Back",
            Some(ICON_ARROW_RIGHT) => "Forward",
            Some(ICON_RELOAD) => "Reload",
            Some(ICON_FOLDER_OPEN) => "Show in folder",
            _ => "Button",
        }
        .into()
    });
    h_flex()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(aria)
        .flex_shrink_0()
        .h(px(height))
        .when(square, |this| this.w(px(height)).justify_center())
        .when(!square, |this| this.px(px(if small { 10.0 } else { 12.0 })))
        .gap(px(6.0))
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_size(px(13.0))
        .line_height(px(18.0))
        .text_color(text)
        .whitespace_nowrap()
        .when(disabled, |this| this.opacity(0.4).cursor_default())
        .when(!disabled, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover))
                .on_press(cx, move |this, window, cx| {
                    cx.stop_propagation();
                    on_click(this, window, cx);
                })
        })
        .children(leading.map(|path| icon(path, if small { 14.0 } else { 15.0 }, text)))
        .children(label)
}

/// Open Dot's toggle chip: filled with the ink colour while selected.
pub(crate) fn chip<V: 'static>(
    p: &DotsPalette,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    leading: Option<AnyElement>,
    selected: bool,
    on_click: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let hover = p.selected;
    h_flex()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .aria_selected(selected)
        .flex_shrink_0()
        .h(px(30.0))
        .pl(px(if leading.is_some() { 4.0 } else { 10.0 }))
        .pr(px(10.0))
        .gap(px(6.0))
        .items_center()
        .rounded(px(CONTROL_RADIUS))
        .border_1()
        .text_size(px(13.0))
        .whitespace_nowrap()
        .cursor_pointer()
        .when(selected, |this| {
            this.border_color(p.primary)
                .bg(p.primary)
                .text_color(p.primary_text)
        })
        .when(!selected, |this| {
            this.border_color(p.border)
                .text_color(p.foreground.opacity(0.75))
                .hover(move |this| this.bg(hover))
        })
        .on_press(cx, move |this, window, cx| {
            cx.stop_propagation();
            on_click(this, window, cx);
        })
        .children(leading)
        .child(label)
}

/// A colour swatch for the look editor.
pub(crate) fn swatch<V: 'static>(
    p: &DotsPalette,
    id: impl Into<ElementId>,
    color: &str,
    selected: bool,
    on_click: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let fill: Hsla = parse_hex(color)
        .map(|rgb_value| rgb(rgb_value).into())
        .unwrap_or(p.muted);
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(SharedString::from(color.to_string()))
        .aria_selected(selected)
        .flex_shrink_0()
        .size(px(28.0))
        .p(px(if selected { 2.0 } else { 0.0 }))
        .rounded_full()
        .border_1()
        .border_color(if selected {
            p.foreground
        } else {
            gpui::transparent_black()
        })
        .cursor_pointer()
        .on_press(cx, move |this, window, cx| {
            cx.stop_propagation();
            on_click(this, window, cx);
        })
        .child(
            div()
                .size_full()
                .rounded_full()
                .bg(fill)
                .border_1()
                .border_color(gpui::Hsla::from(rgb(0x000000)).opacity(0.1)),
        )
}

fn parse_hex(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

/// The `×` that removes a row.
pub(crate) fn remove_button<V: 'static>(
    p: &DotsPalette,
    id: impl Into<ElementId>,
    label: &'static str,
    on_click: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let hover = p.hover;
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |this| this.bg(hover))
        .on_press(cx, move |this, window, cx| {
            cx.stop_propagation();
            on_click(this, window, cx);
        })
        .child(icon(ICON_X, 14.0, p.faint))
}

/// A single-line field in the modal kit's skin.
pub(crate) fn text_input(
    p: &DotsPalette,
    state: &Entity<InputState>,
    window: &Window,
    cx: &App,
) -> AnyElement {
    modal_text_input_skinned(&p.modal, state, p.field_skin(), 12.0, window, cx)
}

/// A multi-line field in the modal kit's skin.
pub(crate) fn text_area(
    p: &DotsPalette,
    state: &Entity<TextareaState>,
    min_height: f32,
    window: &Window,
    cx: &App,
) -> AnyElement {
    modal_text_area_skinned(
        &p.modal,
        state,
        p.field_skin(),
        min_height,
        false,
        window,
        cx,
    )
}

/// A labelled field: the eyebrow over the control.
pub(crate) fn labelled(
    p: &DotsPalette,
    label: impl Into<SharedString>,
    control: AnyElement,
) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .gap(px(6.0))
        .child(eyebrow(p, label))
        .child(control)
}

/// A small error line.
pub(crate) fn error_line(p: &DotsPalette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(18.0))
        .text_color(p.danger)
        .child(text.into())
}

/// A small note line.
pub(crate) fn note_line(p: &DotsPalette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.5))
        .line_height(px(18.0))
        .text_color(p.faint)
        .child(text.into())
}

/// A letter disc standing for an app (Composio logos would need an HTTP image client the app does
/// not install).
pub(crate) fn app_initial(p: &DotsPalette, name: &str, size: f32) -> Div {
    let letter = name
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    div()
        .flex_shrink_0()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(p.selected)
        .border_1()
        .border_color(p.border)
        .text_size(px(size * 0.45))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.foreground.opacity(0.8))
        .child(letter)
}

/// Overlapping app discs (Open Dot's logo stack). Sized explicitly: the overlap is a negative
/// margin, which the row would otherwise not count.
pub(crate) fn app_stack(p: &DotsPalette, names: &[&str], size: f32) -> Div {
    let step = size * 0.75;
    h_flex()
        .flex_shrink_0()
        .relative()
        .w(px(size + names.len().saturating_sub(1) as f32 * step))
        .h(px(size))
        .children(names.iter().enumerate().map(|(index, name)| {
            app_initial(p, name, size)
                .absolute()
                .top_0()
                .left(px(index as f32 * step))
        }))
}
