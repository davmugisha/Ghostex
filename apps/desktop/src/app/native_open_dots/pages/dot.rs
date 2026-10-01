//! A dot's page (Open Dot's `DotView.tsx`): its face, name and status, its agent and model,
//! Pause/Resume and Stop, and the Chats | Computer | Setup tabs. Chats lists the dot's
//! conversations (ordinary sessions in its project) and starts new ones; with none yet it shows
//! Open Dot's chat welcome.

use super::super::forms::{PendingClear, textarea};
use super::super::model::{Dot, DotsSnapshot, iso_time_ago, now_ms};
use super::super::style::*;
use super::super::target::{OpenDotsDotTab, OpenDotsTarget};
use super::super::view::NativeOpenDotsView;
use crate::app::window::native_modal_kit::{ModalSegmentedItem, modal_segmented_control};
use ghostex_gx_core::open_dots::DotFaceStatus;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::{Value, json};

/// Open Dot's chat welcome suggestions.
const SUGGESTIONS: [(&str, &str); 3] = [
    (
        "Research",
        "Research the best noise-cancelling headphones under $300 and give me a shortlist with sources",
    ),
    (
        "Routine",
        "Every weekday at 8am, send me a short briefing on the top AI news",
    ),
    (
        "Browse",
        "Open Hacker News in your browser and tell me the top 5 stories",
    ),
];

impl NativeOpenDotsView {
    fn send_dot_message(&mut self, dot_id: String, cx: &mut Context<Self>) {
        let Some(state) = self.forms.chat_message.clone() else {
            return;
        };
        let message = state.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.forms
            .pending_clears
            .push(PendingClear::Textarea(state));
        self.start_chat(dot_id, Some(message), cx);
    }

    pub(super) fn render_dot_page(
        &mut self,
        p: &DotsPalette,
        dot_id: &str,
        tab: OpenDotsDotTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(snapshot) = self.snapshot.clone() else {
            return div().into_any_element();
        };
        let Some(dot) = snapshot.dot(dot_id).cloned() else {
            return v_flex()
                .pt(px(96.0))
                .items_center()
                .child(body_text(p, "This dot doesn't exist anymore."))
                .into_any_element();
        };
        let content = match tab {
            OpenDotsDotTab::Chats => self.render_chats(p, &snapshot, &dot, window, cx),
            OpenDotsDotTab::Computer => self.render_computer(p, &dot, window, cx),
            OpenDotsDotTab::Setup => self.render_setup(p, &snapshot, &dot, window, cx),
        };
        v_flex()
            .w_full()
            .gap(px(18.0))
            .child(self.render_dot_header(p, &snapshot, &dot, cx))
            .when(self.agent_picker_open, |this| {
                this.child(self.render_agent_picker(p, &snapshot, &dot, cx))
            })
            .child(self.render_dot_tabs(p, &dot, tab, cx))
            .child(content)
            .into_any_element()
    }

    fn render_dot_header(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = dot.face_status();
        let agent_name = snapshot
            .agent(&dot.agent_id)
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| dot.agent_id.clone());
        let model_label = dot.model.as_ref().map(|model| {
            snapshot
                .agent(&dot.agent_id)
                .and_then(|agent| agent.models.iter().find(|candidate| &candidate.id == model))
                .map(|candidate| candidate.label.clone())
                .unwrap_or_else(|| model.clone())
        });
        let picker_label = match model_label {
            Some(model) => format!("{agent_name} · {model}"),
            None => agent_name,
        };
        let dot_id = dot.id.clone();
        let busy = self.is_busy(&format!("state:{dot_id}"));
        let hover = p.hover;
        h_flex()
            .w_full()
            .pt(px(24.0))
            .gap(px(14.0))
            .items_center()
            .flex_wrap()
            .child(face(&dot.look, status, 52.0))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(160.0))
                    .gap(px(3.0))
                    .child(heading(p, dot.name.clone(), 22.0))
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_center()
                            .child(status_dot(p.status_color(status), 7.0))
                            .child(
                                truncated(dot.status_line())
                                    .text_size(px(13.0))
                                    .text_color(p.muted),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .flex_wrap()
                    .child(
                        h_flex()
                            .id("dots-agent-picker")
                            .role(gpui::Role::Button)
                            .aria_label("Agent and model")
                            .aria_expanded(self.agent_picker_open)
                            .h(px(28.0))
                            .px(px(10.0))
                            .gap(px(6.0))
                            .items_center()
                            .rounded(px(CONTROL_RADIUS))
                            .border_1()
                            .border_color(p.border)
                            .text_size(px(12.5))
                            .text_color(p.foreground.opacity(0.85))
                            .cursor_pointer()
                            .hover(move |this| this.bg(hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.agent_picker_open = !this.agent_picker_open;
                                cx.notify();
                            }))
                            .child(picker_label)
                            .child(icon(ICON_CHEVRON_DOWN, 13.0, p.faint)),
                    )
                    .when(status == DotFaceStatus::Working, |this| {
                        let dot_id = dot_id.clone();
                        this.child(button(
                            p,
                            "dots-stop",
                            Some("Stop".into()),
                            Some(ICON_STOP),
                            Tone::Secondary,
                            true,
                            busy,
                            move |this: &mut Self, _, cx| {
                                this.act(
                                    format!("state:{dot_id}"),
                                    "stopDot",
                                    json!({ "dotId": dot_id }),
                                    cx,
                                )
                            },
                            cx,
                        ))
                    })
                    .child(if dot.paused {
                        let dot_id = dot_id.clone();
                        button(
                            p,
                            "dots-resume",
                            Some("Resume".into()),
                            Some(ICON_PLAY),
                            Tone::Primary,
                            true,
                            busy,
                            move |this: &mut Self, _, cx| {
                                this.act(
                                    format!("state:{dot_id}"),
                                    "resumeDot",
                                    json!({ "dotId": dot_id }),
                                    cx,
                                )
                            },
                            cx,
                        )
                    } else {
                        let dot_id = dot_id.clone();
                        button(
                            p,
                            "dots-pause",
                            Some("Pause".into()),
                            Some(ICON_PAUSE),
                            Tone::Secondary,
                            true,
                            busy,
                            move |this: &mut Self, _, cx| {
                                this.act(
                                    format!("state:{dot_id}"),
                                    "pauseDot",
                                    json!({ "dotId": dot_id }),
                                    cx,
                                )
                            },
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }

    /// The agent and model the dot's conversations start with.
    fn render_agent_picker(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agents = snapshot.settings.agents.clone();
        let current = snapshot.agent(&dot.agent_id).cloned();
        let busy = self.is_busy(&format!("agent:{}", dot.id));
        let agent_chips = agents
            .iter()
            .map(|agent| {
                let dot_id = dot.id.clone();
                let agent_id = agent.id.clone();
                chip(
                    p,
                    SharedString::from(format!("dots-agent-{}", agent.id)),
                    agent.name.clone(),
                    None,
                    agent.id == dot.agent_id,
                    move |this: &mut Self, _, cx| {
                        this.act(
                            format!("agent:{dot_id}"),
                            "updateDot",
                            json!({ "dotId": dot_id, "agentId": agent_id, "model": Value::Null }),
                            cx,
                        )
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let models = current.map(|agent| agent.models).unwrap_or_default();
        let model_chips = std::iter::once((None, "Default".to_string()))
            .chain(models.iter().map(|model| (Some(model.id.clone()), model.label.clone())))
            .map(|(model, label)| {
                let dot_id = dot.id.clone();
                let selected = model == dot.model;
                let id = SharedString::from(format!(
                    "dots-model-{}",
                    model.clone().unwrap_or_else(|| "default".to_string())
                ));
                chip(
                    p,
                    id,
                    label,
                    None,
                    selected,
                    move |this: &mut Self, _, cx| {
                        this.act(
                            format!("agent:{dot_id}"),
                            "updateDot",
                            json!({ "dotId": dot_id, "model": model.clone().map(Value::String).unwrap_or(Value::Null) }),
                            cx,
                        )
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        card(p)
            .p(px(14.0))
            .gap(px(12.0))
            .when(busy, |this| this.opacity(0.6))
            .child(labelled(p, "Agent", h_flex().flex_wrap().gap(px(6.0)).children(agent_chips).into_any_element()))
            .child(labelled(p, "Model", h_flex().flex_wrap().gap(px(6.0)).children(model_chips).into_any_element()))
            .child(note_line(p, "New conversations start with this agent and model. Conversations already open keep theirs."))
            .into_any_element()
    }

    fn render_dot_tabs(
        &mut self,
        p: &DotsPalette,
        dot: &Dot,
        tab: OpenDotsDotTab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items = OpenDotsDotTab::ALL.map(|tab| ModalSegmentedItem {
            icon: Some(match tab {
                OpenDotsDotTab::Chats => ICON_MESSAGES,
                OpenDotsDotTab::Computer => ICON_DESKTOP,
                OpenDotsDotTab::Setup => ICON_SLIDERS,
            }),
            label: tab.label(),
        });
        let selected = OpenDotsDotTab::ALL
            .iter()
            .position(|candidate| *candidate == tab)
            .unwrap_or(0);
        let dot_id = dot.id.clone();
        div()
            .w_full()
            .max_w(px(420.0))
            .child(modal_segmented_control(
                &p.modal,
                "dots-dot-tabs",
                &items,
                selected,
                move |this: &mut Self, index, _, cx| {
                    let tab = OpenDotsDotTab::ALL[index.min(2)];
                    this.navigate(
                        OpenDotsTarget::Dot {
                            dot_id: dot_id.clone(),
                            tab,
                        },
                        cx,
                    );
                },
                cx,
            ))
            .into_any_element()
    }

    fn render_chats(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = now_ms();
        let composer = match self.forms.chat_message.clone() {
            Some(state) => state,
            None => {
                let state = textarea(window, cx, "", "", 2, 8);
                state.update(cx, |state, cx| state.set_submit_on_enter(true, cx));
                self.forms.chat_message = Some(state.clone());
                state
            }
        };
        if self.forms.chat_placeholder_dot.as_deref() != Some(dot.id.as_str()) {
            self.forms.chat_placeholder_dot = Some(dot.id.clone());
            let placeholder = format!(
                "Ask {} to research, browse, plan, or build something…",
                dot.name
            );
            composer.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            });
            let dot_id = dot.id.clone();
            self.submit_on_enter("chat", &composer, window, cx, move |this, _, cx| {
                this.send_dot_message(dot_id.clone(), cx)
            });
        }
        let typed = !composer.read(cx).value().trim().is_empty();
        let sending = self.is_busy(&format!("startChat:{}", dot.id));
        let dot_id = dot.id.clone();
        let composer_card = card(p)
            .p(px(12.0))
            .gap(px(8.0))
            .child(text_area(p, &composer, 60.0, window, cx))
            .child(h_flex().justify_end().child(button(
                p,
                "dots-chat-start",
                Some(if sending {
                    "Starting…".into()
                } else {
                    "Start chat".into()
                }),
                Some(ICON_ARROW_UP),
                Tone::Primary,
                true,
                !typed || sending,
                move |this: &mut Self, _, cx| this.send_dot_message(dot_id.clone(), cx),
                cx,
            )));

        let approvals: Vec<_> = snapshot
            .pending_approvals()
            .filter(|approval| approval.dot_id == dot.id)
            .cloned()
            .collect();
        let approval_cards: Vec<AnyElement> = approvals
            .iter()
            .map(|approval| self.render_approval_card(p, approval, None, now, window, cx))
            .collect();

        let chats = self.chats.get(&dot.project_id).cloned();
        let hover = p.hover;
        let list: AnyElement = match chats {
            None => note_line(p, "Loading conversations…").into_any_element(),
            Some(chats) if chats.is_empty() => self.render_chat_welcome(p, dot, cx),
            Some(chats) => rows_card(
                p,
                chats
                    .iter()
                    .map(|chat| {
                        let project_id = dot.project_id.clone();
                        let session_id = chat.session_id.clone();
                        h_flex()
                            .id(SharedString::from(format!("dots-chat-{}", chat.session_id)))
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(chat.title.clone()))
                            .w_full()
                            .px(px(14.0))
                            .py(px(10.0))
                            .gap(px(10.0))
                            .items_center()
                            .cursor_pointer()
                            .hover(move |this| this.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.focus_session(project_id.clone(), session_id.clone(), cx)
                            }))
                            .child(icon(ICON_MESSAGES, 15.0, p.faint))
                            .child(truncated(chat.title.clone()).flex_1().text_size(px(14.0)))
                            .when(chat.sleeping, |this| this.child(badge("Sleeping", p.faint)))
                            .child(note_line(p, iso_time_ago(&chat.updated, now)))
                            .into_any_element()
                    })
                    .collect(),
            )
            .into_any_element(),
        };
        let chat_count = self
            .chats
            .get(&dot.project_id)
            .map(|chats| chats.len())
            .unwrap_or(0);
        let new_chat_dot = dot.id.clone();

        let handoffs: Vec<_> = snapshot
            .handoffs
            .iter()
            .filter(|handoff| handoff.from_dot_id == dot.id || handoff.to_dot_id == dot.id)
            .take(8)
            .cloned()
            .collect();
        let handoff_rows: Vec<AnyElement> = handoffs
            .iter()
            .map(|handoff| {
                let name = |id: &str| {
                    snapshot
                        .dot(id)
                        .map(|dot| dot.name.clone())
                        .unwrap_or_else(|| "A dot".to_string())
                };
                let status_color = match handoff.status.as_str() {
                    "replied" => p.success,
                    "failed" => p.danger,
                    _ => p.accent,
                };
                v_flex()
                    .px(px(14.0))
                    .py(px(10.0))
                    .gap(px(4.0))
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .items_center()
                            .child(div().text_size(px(13.0)).child(format!(
                                "{} → {}",
                                name(&handoff.from_dot_id),
                                name(&handoff.to_dot_id)
                            )))
                            .child(badge(handoff.status.clone(), status_color))
                            .child(div().flex_1())
                            .child(note_line(
                                p,
                                super::super::model::time_ago(handoff.created_at, now),
                            )),
                    )
                    .child(body_text(p, handoff.message.clone()).line_clamp(2))
                    .when_some(handoff.reply.clone(), |this, reply| {
                        this.child(body_text(p, format!("Reply: {reply}")).line_clamp(3))
                    })
                    .into_any_element()
            })
            .collect();

        v_flex()
            .w_full()
            .gap(px(16.0))
            .children(approval_cards)
            .child(composer_card)
            .child(
                v_flex()
                    .w_full()
                    .gap(px(10.0))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(eyebrow(p, format!("Chats · {chat_count}")))
                            .child(button(
                                p,
                                "dots-new-chat",
                                Some("New chat".into()),
                                Some(ICON_MESSAGE_PLUS),
                                Tone::Quiet,
                                true,
                                sending,
                                move |this: &mut Self, _, cx| {
                                    this.start_chat(new_chat_dot.clone(), None, cx)
                                },
                                cx,
                            )),
                    )
                    .child(list),
            )
            .when(!handoff_rows.is_empty(), |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .gap(px(10.0))
                        .child(eyebrow(p, "Hand-offs between dots"))
                        .child(rows_card(p, handoff_rows)),
                )
            })
            .into_any_element()
    }

    /// Open Dot's chat `Welcome`, for a dot with no conversations yet.
    fn render_chat_welcome(
        &mut self,
        p: &DotsPalette,
        dot: &Dot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = p.selected;
        v_flex()
            .w_full()
            .items_center()
            .pt(px(16.0))
            .gap(px(8.0))
            .child(face(&dot.look, dot.face_status(), 120.0))
            .child(eyebrow(p, "Your dot").text_color(p.accent).mt(px(8.0)))
            .child(heading(p, format!("Hi, I'm {}", dot.name), 22.0))
            .when(!dot.purpose.trim().is_empty(), |this| {
                this.child(
                    h_flex()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.card)
                        .child(eyebrow(p, "Job"))
                        .child(div().text_size(px(13.0)).text_color(p.foreground.opacity(0.8)).child(dot.purpose.clone())),
                )
            })
            .child(
                body_text(
                    p,
                    "I work in my own folder and browser, remember what matters, and ask before doing anything important.",
                )
                .text_center()
                .max_w(px(460.0)),
            )
            .child(
                div()
                    .w_full()
                    .mt(px(12.0))
                    .grid()
                    .grid_cols(if self.page_width.get() >= 760.0 { 3 } else { 1 })
                    .gap(px(10.0))
                    .children(SUGGESTIONS.iter().enumerate().map(|(index, (label, text))| {
                        let dot_id = dot.id.clone();
                        let text = text.to_string();
                        v_flex()
                            .id(("dots-suggestion", index))
                            .role(gpui::Role::Button)
                            .aria_label(SharedString::from(text.clone()))
                            .w_full()
                            .p(px(14.0))
                            .gap(px(6.0))
                            .rounded(px(CARD_RADIUS))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.card)
                            .cursor_pointer()
                            .hover(move |this| this.bg(hover))
                            .on_click(cx.listener({
                                let text = text.clone();
                                move |this, _, _, cx| this.start_chat(dot_id.clone(), Some(text.clone()), cx)
                            }))
                            .child(eyebrow(p, *label))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(19.0))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(p.foreground.opacity(0.8))
                                    .child(text),
                            )
                    })),
            )
            .into_any_element()
    }
}
