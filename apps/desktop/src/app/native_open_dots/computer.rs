//! A dot page's Computer tab: the dot's own browser, shown from polled screenshots (about two a
//! second while it is live or you are driving, every few seconds otherwise), Take over / Hand back
//! with your clicks, scrolling and typing forwarded in page coordinates, the dot's files, and where
//! it runs. Ported from Open Dot's `ComputerPane.tsx`, `LiveBrowser.tsx` and `FilesPanel.tsx`.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use super::model::{Dot, DotFileEntry, DotScreen, file_size};
use super::style::*;
use super::view::{NativeOpenDotsView, open_dots_rpc};
use base64::Engine as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Bounds, Context, FocusHandle, ImageSource,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, RenderImage, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, img, px,
};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::{h_flex, v_flex};
use serde_json::{Value, json};
use web_time::Instant;

/// The dot's browser viewport (CONTRACT §2 `browserScreenshot`); clicks are scaled to it.
const PAGE_WIDTH: f32 = 1280.0;
const PAGE_HEIGHT: f32 = 800.0;
const LIVE_POLL: Duration = Duration::from_millis(500);
const IDLE_POLL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct ComputerState {
    pub(crate) image: Option<Arc<RenderImage>>,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) live: bool,
    pub(crate) taken_over: bool,
    pub(crate) in_flight: bool,
    pub(crate) next_poll_at: Option<Instant>,
    pub(crate) error: Option<String>,
    pub(crate) confirm_reset: bool,
    pub(crate) files: Option<Result<Vec<DotFileEntry>, String>>,
    pub(crate) files_loading: bool,
    /// Where the screen was last painted, to map a click to page coordinates.
    pub(crate) bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    inputs: VecDeque<Value>,
    sending: bool,
    pub(crate) focus: Option<FocusHandle>,
    /// Bumped when the page leaves the dot, so a screenshot answered after that is dropped.
    generation: u64,
}

/// A screenshot decoded for GPUI, which wants BGRA.
fn decode_screen(base64_jpeg: &str) -> Option<Arc<RenderImage>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_jpeg.trim())
        .ok()?;
    let mut rgba = image::load_from_memory(&bytes).ok()?.into_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

/// A key as the DOM names it, for `browserInput` `key`.
fn dom_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "enter" => "Enter",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "escape" => "Escape",
        "delete" => "Delete",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        _ => return None,
    })
}

impl NativeOpenDotsView {
    /// Leaves the dot's screen: the picture is released from the GPU atlas, and answers still in
    /// flight are dropped.
    pub(crate) fn reset_computer(&mut self, cx: &mut Context<Self>) {
        let generation = self.computer.generation + 1;
        let focus = self.computer.focus.take();
        if let Some(old) = std::mem::take(&mut self.computer).image {
            cx.drop_image(old, None);
        }
        self.computer.generation = generation;
        self.computer.focus = focus;
    }

    pub(crate) fn poll_screen_if_due(&mut self, dot_id: &str, cx: &mut Context<Self>) {
        if self.computer.in_flight
            || self
                .computer
                .next_poll_at
                .is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        self.poll_screen(dot_id.to_string(), false, cx);
    }

    /// One screenshot. `wake` starts the dot's browser when it is not running (Refresh).
    pub(crate) fn poll_screen(&mut self, dot_id: String, wake: bool, cx: &mut Context<Self>) {
        self.computer.in_flight = true;
        let generation = self.computer.generation;
        cx.spawn(async move |this, cx| {
            let result = open_dots_rpc(
                "browserScreenshot",
                json!({ "dotId": dot_id, "wake": wake }),
            )
            .await
            .and_then(|value| {
                serde_json::from_value::<DotScreen>(value).map_err(|error| error.to_string())
            });
            let decoded = match &result {
                Ok(screen) => match screen.image.clone() {
                    Some(data) => {
                        cx.background_executor()
                            .spawn(async move { decode_screen(&data) })
                            .await
                    }
                    None => None,
                },
                Err(_) => None,
            };
            let _ = this.update(cx, |this, cx| {
                if this.computer.generation != generation {
                    if let Some(image) = decoded {
                        cx.drop_image(image, None);
                    }
                    return;
                }
                this.computer.in_flight = false;
                match result {
                    Ok(screen) => {
                        this.computer.error = None;
                        this.computer.live = screen.live;
                        this.computer.taken_over = screen.taken_over;
                        this.computer.url = screen.url;
                        this.computer.title = screen.title;
                        if let Some(image) = decoded
                            && let Some(old) = this.computer.image.replace(image)
                        {
                            cx.drop_image(old, None);
                        }
                    }
                    Err(error) => this.computer.error = Some(error),
                }
                let wait = if this.computer.live || this.computer.taken_over {
                    LIVE_POLL
                } else {
                    IDLE_POLL
                };
                this.computer.next_poll_at = Some(Instant::now() + wait);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_files(&mut self, dot_id: &str, cx: &mut Context<Self>) {
        if self.computer.files_loading {
            return;
        }
        self.computer.files_loading = true;
        let dot_id = dot_id.to_string();
        let generation = self.computer.generation;
        cx.spawn(async move |this, cx| {
            let result = open_dots_rpc("listFiles", json!({ "dotId": dot_id })).await;
            let _ = this.update(cx, |this, cx| {
                if this.computer.generation != generation {
                    return;
                }
                this.computer.files_loading = false;
                this.computer.files = Some(result.map(|value| {
                    value
                        .get("files")
                        .and_then(Value::as_array)
                        .map(|rows| {
                            rows.iter()
                                .filter_map(|row| serde_json::from_value(row.clone()).ok())
                                .collect()
                        })
                        .unwrap_or_default()
                }));
                cx.notify();
            });
        })
        .detach();
    }

    /// Queues one input for the dot's browser. Inputs go out one at a time, in order; scroll ticks
    /// that pile up while one is in flight merge into one, so a fast wheel does not build a backlog.
    fn send_input(&mut self, dot_id: &str, mut input: Value, cx: &mut Context<Self>) {
        input["dotId"] = Value::String(dot_id.to_string());
        if input["kind"] == "scroll"
            && let Some(last) = self.computer.inputs.back_mut()
            && last["kind"] == "scroll"
        {
            for key in ["deltaX", "deltaY"] {
                let sum = last[key].as_f64().unwrap_or(0.0) + input[key].as_f64().unwrap_or(0.0);
                last[key] = json!(sum);
            }
            return;
        }
        self.computer.inputs.push_back(input);
        self.drain_inputs(cx);
    }

    fn drain_inputs(&mut self, cx: &mut Context<Self>) {
        if self.computer.sending {
            return;
        }
        let Some(next) = self.computer.inputs.pop_front() else {
            return;
        };
        self.computer.sending = true;
        cx.spawn(async move |this, cx| {
            let result = open_dots_rpc("browserInput", next).await;
            let _ = this.update(cx, |this, cx| {
                this.computer.sending = false;
                if let Err(error) = result {
                    this.computer.error = Some(error);
                }
                // The page just changed: show it soon.
                this.computer.next_poll_at = None;
                this.drain_inputs(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// A window position as a point on the dot's 1280x800 page.
    fn page_point(&self, position: gpui::Point<Pixels>) -> Option<(f32, f32)> {
        let bounds = self.computer.bounds.get()?;
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let x =
            (f32::from(position.x - bounds.origin.x) / width * PAGE_WIDTH).clamp(0.0, PAGE_WIDTH);
        let y = (f32::from(position.y - bounds.origin.y) / height * PAGE_HEIGHT)
            .clamp(0.0, PAGE_HEIGHT);
        Some((x.round(), y.round()))
    }

    fn take_over(&mut self, dot_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.computer.focus.clone() {
            window.focus(&focus, cx);
        }
        self.call(
            format!("takeover:{dot_id}"),
            "browserTakeOver",
            json!({ "dotId": dot_id }),
            |this, result, _| match result {
                Ok(value) => {
                    this.computer.taken_over = value
                        .get("takenOver")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    this.computer.next_poll_at = None;
                }
                Err(error) => this.computer.error = Some(error),
            },
            cx,
        );
    }

    fn hand_back(&mut self, dot_id: String, cx: &mut Context<Self>) {
        self.call(
            format!("takeover:{dot_id}"),
            "browserHandBack",
            json!({ "dotId": dot_id }),
            |this, result, _| match result {
                Ok(_) => this.computer.taken_over = false,
                Err(error) => this.computer.error = Some(error),
            },
            cx,
        );
    }

    fn ensure_url_input(&mut self, dot_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.forms.url.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search or enter address"));
        let dot_id = dot_id.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let url = state.read(cx).value().trim().to_string();
                    if !url.is_empty() {
                        this.send_input(&dot_id, json!({ "kind": "navigate", "url": url }), cx);
                        state.update(cx, |state, cx| state.set_value("", window, cx));
                    }
                }
            },
        );
        self.forms.url = Some(input);
        self.forms.url_subscription = Some(subscription);
    }

    pub(crate) fn render_computer(
        &mut self,
        p: &DotsPalette,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.computer.focus.is_none() {
            self.computer.focus = Some(cx.focus_handle());
        }
        let focus = self.computer.focus.clone().expect("set above");
        let taken_over = self.computer.taken_over;
        if taken_over {
            self.ensure_url_input(&dot.id, window, cx);
        }
        let live = self.computer.live || taken_over;
        let dot_id = dot.id.clone();
        let name = dot.name.clone();
        let busy_take = self.is_busy(&format!("takeover:{dot_id}"));

        let controls = h_flex()
            .gap(px(8.0))
            .flex_wrap()
            .child(button(
                p,
                "dots-computer-refresh",
                Some("Refresh".into()),
                Some(ICON_REFRESH),
                Tone::Secondary,
                true,
                self.computer.in_flight,
                {
                    let dot_id = dot_id.clone();
                    move |this: &mut Self, _, cx| this.poll_screen(dot_id.clone(), true, cx)
                },
                cx,
            ))
            .child(if taken_over {
                button(
                    p,
                    "dots-computer-hand-back",
                    Some(format!("Hand back to {name}").into()),
                    Some(ICON_UNDO),
                    Tone::Primary,
                    true,
                    busy_take,
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, _, cx| this.hand_back(dot_id.clone(), cx)
                    },
                    cx,
                )
            } else {
                button(
                    p,
                    "dots-computer-take-over",
                    Some("Take over".into()),
                    Some(ICON_POINTER),
                    Tone::Primary,
                    true,
                    busy_take,
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, window, cx| {
                            this.take_over(dot_id.clone(), window, cx)
                        }
                    },
                    cx,
                )
            });

        let description = if taken_over {
            format!(
                "You're driving. Click and type right here; logins stay saved in {name}'s browser."
            )
        } else {
            format!(
                "{name}'s own browser. Watch it work, or take over to sign in to a site yourself."
            )
        };

        let status_label = if taken_over {
            "You're in control"
        } else if live {
            "Live"
        } else {
            "Idle"
        };
        let status_color = if taken_over {
            p.warning
        } else if live {
            p.accent
        } else {
            p.faint
        };
        let address: SharedString = if self.computer.url.trim().is_empty() {
            format!("{}.browser", name.to_lowercase()).into()
        } else {
            self.computer.url.clone().into()
        };

        let toolbar = h_flex()
            .h(px(36.0))
            .px(px(12.0))
            .gap(px(10.0))
            .items_center()
            .border_b_1()
            .border_color(p.border)
            .bg(p.raised)
            .child(
                h_flex()
                    .gap(px(5.0))
                    .children((0..3).map(|_| status_dot(p.foreground.opacity(0.15), 9.0))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(24.0))
                    .px(px(10.0))
                    .gap(px(6.0))
                    .items_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .text_size(px(11.5))
                    .text_color(p.muted)
                    .child(icon(ICON_WORLD, 12.0, p.faint))
                    .child(truncated(address)),
            )
            .child(
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(status_dot(status_color, 6.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(p.muted)
                            .child(status_label),
                    ),
            );

        let driving_bar = taken_over.then(|| {
            let url_input = self.forms.url.clone();
            h_flex()
                .h(px(40.0))
                .px(px(8.0))
                .gap(px(4.0))
                .items_center()
                .border_b_1()
                .border_color(p.border)
                .child(button(
                    p,
                    "dots-browser-back",
                    None,
                    Some(ICON_ARROW_LEFT),
                    Tone::Quiet,
                    true,
                    false,
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, _, cx| {
                            this.send_input(&dot_id, json!({ "kind": "back" }), cx)
                        }
                    },
                    cx,
                ))
                .child(button(
                    p,
                    "dots-browser-forward",
                    None,
                    Some(ICON_ARROW_RIGHT),
                    Tone::Quiet,
                    true,
                    false,
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, _, cx| {
                            this.send_input(&dot_id, json!({ "kind": "forward" }), cx)
                        }
                    },
                    cx,
                ))
                .child(button(
                    p,
                    "dots-browser-reload",
                    None,
                    Some(ICON_RELOAD),
                    Tone::Quiet,
                    true,
                    false,
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, _, cx| {
                            this.send_input(&dot_id, json!({ "kind": "reload" }), cx)
                        }
                    },
                    cx,
                ))
                .children(url_input.map(|input| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(text_input(p, &input, window, cx))
                }))
        });

        let bounds_cell = self.computer.bounds.clone();
        let screen_content: AnyElement = match self.computer.image.clone() {
            Some(image) => img(ImageSource::Render(image)).size_full().into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .px(px(24.0))
                .child(icon(ICON_DESKTOP, 24.0, p.faint))
                .child(
                    body_text(
                        p,
                        format!(
                            "Nothing on screen yet. Press Refresh to start {name}'s browser, or ask it to open a website."
                        ),
                    )
                    .text_center()
                    .max_w(px(420.0)),
                )
                .into_any_element(),
        };
        let screen = div()
            .id("dots-computer-screen")
            .relative()
            .w_full()
            .aspect_ratio(PAGE_WIDTH / PAGE_HEIGHT)
            .bg(p.raised)
            .overflow_hidden()
            .track_focus(&focus)
            .when(taken_over, |this| {
                let click_dot = dot_id.clone();
                let wheel_dot = dot_id.clone();
                let key_dot = dot_id.clone();
                this.cursor_default()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if let Some(focus) = this.computer.focus.clone() {
                                window.focus(&focus, cx);
                            }
                            let Some((x, y)) = this.page_point(event.position) else {
                                return;
                            };
                            let kind = if event.click_count >= 2 {
                                "doubleClick"
                            } else {
                                "click"
                            };
                            this.send_input(
                                &click_dot,
                                json!({ "kind": kind, "x": x, "y": y }),
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    )
                    .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
                        let Some((x, y)) = this.page_point(event.position) else {
                            return;
                        };
                        let Some(bounds) = this.computer.bounds.get() else {
                            return;
                        };
                        let scale = PAGE_WIDTH / f32::from(bounds.size.width).max(1.0);
                        let delta = event.delta.pixel_delta(px(20.0));
                        this.send_input(
                            &wheel_dot,
                            json!({
                                "kind": "scroll",
                                "x": x,
                                "y": y,
                                "deltaX": -f32::from(delta.x) * scale,
                                "deltaY": -f32::from(delta.y) * scale,
                            }),
                            cx,
                        );
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        let keystroke = &event.keystroke;
                        let modifiers = keystroke.modifiers;
                        if (modifiers.platform || modifiers.control) && keystroke.key == "v" {
                            if let Some(text) =
                                cx.read_from_clipboard().and_then(|item| item.text())
                                && !text.is_empty()
                            {
                                this.send_input(
                                    &key_dot,
                                    json!({ "kind": "type", "text": text }),
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                            return;
                        }
                        if !modifiers.platform
                            && !modifiers.control
                            && let Some(text) =
                                keystroke.key_char.clone().filter(|text| !text.is_empty())
                            && dom_key(&keystroke.key).is_none()
                        {
                            this.send_input(&key_dot, json!({ "kind": "type", "text": text }), cx);
                            cx.stop_propagation();
                            return;
                        }
                        if keystroke.key == "space" {
                            this.send_input(&key_dot, json!({ "kind": "type", "text": " " }), cx);
                            cx.stop_propagation();
                            return;
                        }
                        if let Some(key) = dom_key(&keystroke.key) {
                            this.send_input(&key_dot, json!({ "kind": "key", "key": key }), cx);
                            cx.stop_propagation();
                        }
                    }))
            })
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(screen_content)
            .when(taken_over && focus.is_focused(window), |this| {
                this.border_2().border_color(p.accent.opacity(0.5))
            });

        let screen_card = card(p)
            .overflow_hidden()
            .child(toolbar)
            .children(driving_bar)
            .child(screen);

        v_flex()
            .w_full()
            .gap(px(16.0))
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(px(12.0))
                    .items_end()
                    .justify_between()
                    .child(
                        v_flex()
                            .min_w(px(240.0))
                            .flex_1()
                            .gap(px(4.0))
                            .child(eyebrow(p, "Computer"))
                            .child(heading(p, format!("{name}'s computer"), 20.0))
                            .child(body_text(p, description)),
                    )
                    .child(controls),
            )
            .when_some(self.computer.error.clone(), |this, error| {
                this.child(error_line(p, error))
            })
            .child(screen_card)
            .child(self.render_files(p, dot, cx))
            .child(self.render_where_it_runs(p, dot, cx))
            .into_any_element()
    }

    fn render_files(&mut self, p: &DotsPalette, dot: &Dot, cx: &mut Context<Self>) -> AnyElement {
        let folder = dot.folder.trim_end_matches('/').to_string();
        let body: AnyElement = match &self.computer.files {
            None => note_line(p, "Loading files…")
                .px(px(16.0))
                .py(px(14.0))
                .into_any_element(),
            Some(Err(error)) => error_line(p, error.clone())
                .px(px(16.0))
                .py(px(14.0))
                .into_any_element(),
            Some(Ok(files)) if files.is_empty() => {
                note_line(p, format!("{}'s folder is empty.", dot.name))
                    .px(px(16.0))
                    .py(px(14.0))
                    .into_any_element()
            }
            Some(Ok(files)) => v_flex()
                .w_full()
                .children(files.iter().enumerate().map(|(index, file)| {
                    let depth = file.path.matches('/').count() as f32;
                    let file_name = file
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&file.path)
                        .to_string();
                    let absolute = format!("{folder}/{}", file.path);
                    let open_path = absolute.clone();
                    h_flex()
                        .id(("dots-file", index))
                        .w_full()
                        .min_w_0()
                        .h(px(30.0))
                        .pl(px(16.0 + depth * 16.0))
                        .pr(px(8.0))
                        .gap(px(8.0))
                        .items_center()
                        .child(icon(
                            if file.is_dir { ICON_FOLDER } else { ICON_FILE },
                            14.0,
                            p.faint,
                        ))
                        .child(
                            truncated(file_name)
                                .flex_1()
                                .text_size(px(13.0))
                                .text_color(if file.is_dir { p.muted } else { p.foreground }),
                        )
                        .when(!file.is_dir, |row| {
                            row.child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(p.faint)
                                    .child(file_size(file.size)),
                            )
                            .child(button(
                                p,
                                ("dots-file-open", index),
                                Some("Open".into()),
                                None,
                                Tone::Quiet,
                                true,
                                false,
                                move |this: &mut Self, _, cx| {
                                    this.open_path(open_path.clone(), false, cx)
                                },
                                cx,
                            ))
                        })
                        .child(button(
                            p,
                            ("dots-file-reveal", index),
                            None,
                            Some(ICON_FOLDER_OPEN),
                            Tone::Quiet,
                            true,
                            false,
                            move |this: &mut Self, _, cx| {
                                this.open_path(absolute.clone(), true, cx)
                            },
                            cx,
                        ))
                        .into_any_element()
                }))
                .into_any_element(),
        };
        let dot_id = dot.id.clone();
        card(p)
            .overflow_hidden()
            .child(
                h_flex()
                    .px(px(16.0))
                    .py(px(10.0))
                    .gap(px(8.0))
                    .items_center()
                    .border_b_1()
                    .border_color(p.border)
                    .child(icon(ICON_FOLDER_OPEN, 16.0, p.muted))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .text_color(p.foreground)
                            .child("Files"),
                    )
                    .child(
                        truncated(format!("in {}'s folder", dot.name))
                            .flex_1()
                            .text_size(px(12.5))
                            .text_color(p.faint),
                    )
                    .child(button(
                        p,
                        "dots-files-refresh",
                        Some("Refresh".into()),
                        Some(ICON_REFRESH),
                        Tone::Quiet,
                        true,
                        self.computer.files_loading,
                        move |this: &mut Self, _, cx| this.load_files(&dot_id, cx),
                        cx,
                    )),
            )
            .child(
                div()
                    .id("dots-files-list")
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_where_it_runs(
        &mut self,
        p: &DotsPalette,
        dot: &Dot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder = dot.folder.clone();
        let chrome = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.settings.chrome.is_some())
            .unwrap_or(true);
        let dot_id = dot.id.clone();
        let reset = if self.computer.confirm_reset {
            h_flex()
                .gap(px(8.0))
                .flex_wrap()
                .items_center()
                .child(body_text(
                    p,
                    "Closes its browser and deletes its saved logins and cookies. Files stay.",
                ))
                .child(button(
                    p,
                    "dots-reset-confirm",
                    Some("Reset browser".into()),
                    None,
                    Tone::DangerFilled,
                    true,
                    self.is_busy(&format!("reset:{dot_id}")),
                    {
                        let dot_id = dot_id.clone();
                        move |this: &mut Self, _, cx| {
                            this.computer.confirm_reset = false;
                            this.act(
                                format!("reset:{dot_id}"),
                                "browserReset",
                                json!({ "dotId": dot_id }),
                                cx,
                            );
                            this.computer.next_poll_at = None;
                        }
                    },
                    cx,
                ))
                .child(button(
                    p,
                    "dots-reset-cancel",
                    Some("Cancel".into()),
                    None,
                    Tone::Quiet,
                    true,
                    false,
                    |this: &mut Self, _, cx| {
                        this.computer.confirm_reset = false;
                        cx.notify();
                    },
                    cx,
                ))
                .into_any_element()
        } else {
            button(
                p,
                "dots-reset",
                Some("Reset browser".into()),
                Some(ICON_REFRESH),
                Tone::Secondary,
                true,
                false,
                |this: &mut Self, _, cx| {
                    this.computer.confirm_reset = true;
                    cx.notify();
                },
                cx,
            )
            .into_any_element()
        };
        card(p)
            .p(px(16.0))
            .gap(px(10.0))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(icon(ICON_TERMINAL, 16.0, p.muted))
                    .child(div().flex_1().text_size(px(15.0)).text_color(p.foreground).child("Where it runs"))
                    .child(badge("This computer", p.warning)),
            )
            .child(body_text(
                p,
                format!(
                    "{} works in its own folder on this computer, under its agent's own permission prompts, and browses with its own Chrome profile.",
                    dot.name
                ),
            ))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(
                        truncated(folder.clone())
                            .flex_1()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(p.raised)
                            .text_size(px(12.0))
                            .text_color(p.muted),
                    )
                    .child(button(
                        p,
                        "dots-folder-reveal",
                        Some("Show folder".into()),
                        Some(ICON_FOLDER_OPEN),
                        Tone::Secondary,
                        true,
                        folder.is_empty(),
                        move |this: &mut Self, _, cx| this.open_path(folder.clone(), true, cx),
                        cx,
                    )),
            )
            .when(!chrome, |this| {
                this.child(error_line(
                    p,
                    "Google Chrome is not installed, so dots cannot browse. Install Chrome and press Refresh.",
                ))
            })
            .child(h_flex().child(reset))
            .into_any_element()
    }
}
