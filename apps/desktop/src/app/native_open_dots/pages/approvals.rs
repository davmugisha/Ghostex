//! A pending approval, question or app-connect card (Open Dot's chat `CardRow`), shown on Home's
//! "Needs you" list and on the dot's Chats tab. The same approval is also the question card in the
//! session's own chat (CONTRACT §6); answering either resolves both.

use super::super::forms::input;
use super::super::model::{Dot, DotApproval, time_ago};
use super::super::style::*;
use super::super::view::NativeOpenDotsView;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::{Value, json};

impl NativeOpenDotsView {
    fn resolve_approval(
        &mut self,
        approval_id: String,
        choice: &'static str,
        answer: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let mut params = json!({ "approvalId": approval_id, "choice": choice });
        if let Some(answer) = answer {
            params["answer"] = Value::String(answer);
        }
        self.act(
            format!("approval:{approval_id}"),
            "resolveApproval",
            params,
            cx,
        );
    }

    pub(super) fn render_approval_card(
        &mut self,
        p: &DotsPalette,
        approval: &DotApproval,
        dot: Option<&Dot>,
        now: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = approval.id.clone();
        let busy = self.is_busy(&format!("approval:{id}"));
        let key = |suffix: &str| SharedString::from(format!("dots-approval-{suffix}-{id}"));
        let header = h_flex()
            .px(px(14.0))
            .py(px(8.0))
            .gap(px(8.0))
            .items_center()
            .border_b_1()
            .border_color(p.border)
            .bg(p.raised)
            .child(status_dot(p.warning, 6.0))
            .child(eyebrow(p, approval.heading()))
            .child(div().flex_1())
            .when_some(dot, |this, dot| {
                this.child(
                    h_flex()
                        .gap(px(6.0))
                        .items_center()
                        .child(face(&dot.look, dot.face_status(), 18.0))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .text_color(p.muted)
                                .child(dot.name.clone()),
                        ),
                )
            })
            .child(note_line(p, time_ago(approval.created_at, now)));

        let mut actions = h_flex()
            .mt(px(12.0))
            .gap(px(8.0))
            .flex_wrap()
            .items_center();
        match approval.kind.as_str() {
            "connect" => {
                let url = approval.url.clone();
                let open_id = id.clone();
                let done_id = id.clone();
                let deny_id = id.clone();
                actions = actions
                    .child(button(
                        p,
                        key("connect"),
                        Some(approval.title.clone().into()),
                        Some(ICON_EXTERNAL),
                        Tone::Primary,
                        true,
                        url.is_none(),
                        move |this: &mut Self, _, cx| {
                            let _ = &open_id;
                            if let Some(url) = url.clone() {
                                this.open_url(url, cx);
                            }
                        },
                        cx,
                    ))
                    .child(button(
                        p,
                        key("connected"),
                        Some("I've connected".into()),
                        None,
                        Tone::Secondary,
                        true,
                        busy,
                        move |this: &mut Self, _, cx| {
                            this.resolve_approval(done_id.clone(), "approve", None, cx)
                        },
                        cx,
                    ))
                    .child(button(
                        p,
                        key("not-now"),
                        Some("Not now".into()),
                        None,
                        Tone::Quiet,
                        true,
                        busy,
                        move |this: &mut Self, _, cx| {
                            this.resolve_approval(deny_id.clone(), "deny", None, cx)
                        },
                        cx,
                    ));
            }
            "question" => {
                let options = approval.options.clone();
                if !options.is_empty() {
                    actions =
                        actions.children(options.into_iter().enumerate().map(|(index, option)| {
                            let answer_id = id.clone();
                            let label = option.clone();
                            button(
                                p,
                                SharedString::from(format!("dots-approval-option-{index}-{id}")),
                                Some(label.into()),
                                None,
                                Tone::Secondary,
                                true,
                                busy,
                                move |this: &mut Self, _, cx| {
                                    this.resolve_approval(
                                        answer_id.clone(),
                                        "answer",
                                        Some(option.clone()),
                                        cx,
                                    )
                                },
                                cx,
                            )
                        }));
                }
            }
            _ => {
                let approve_id = id.clone();
                let deny_id = id.clone();
                actions = actions
                    .child(button(
                        p,
                        key("approve"),
                        Some("Approve".into()),
                        Some(ICON_CHECK),
                        Tone::Primary,
                        true,
                        busy,
                        move |this: &mut Self, _, cx| {
                            this.resolve_approval(approve_id.clone(), "approve", None, cx)
                        },
                        cx,
                    ))
                    .child(button(
                        p,
                        key("deny"),
                        Some("Deny".into()),
                        None,
                        Tone::Secondary,
                        true,
                        busy,
                        move |this: &mut Self, _, cx| {
                            this.resolve_approval(deny_id.clone(), "deny", None, cx)
                        },
                        cx,
                    ));
                if approval.rule_action.is_some() {
                    let always_id = id.clone();
                    actions = actions.child(button(
                        p,
                        key("always"),
                        Some("Always allow".into()),
                        None,
                        Tone::Quiet,
                        true,
                        busy,
                        move |this: &mut Self, _, cx| {
                            this.resolve_approval(always_id.clone(), "always", None, cx)
                        },
                        cx,
                    ));
                }
            }
        }
        if let Some(session_id) = approval.session_id.clone() {
            let project_id = approval.project_id.clone();
            actions = actions.child(div().flex_1()).child(button(
                p,
                key("open"),
                Some("Open chat".into()),
                Some(ICON_MESSAGES),
                Tone::Quiet,
                true,
                false,
                move |this: &mut Self, _, cx| {
                    this.focus_session(project_id.clone(), session_id.clone(), cx)
                },
                cx,
            ));
        }

        // A question always takes a typed answer too.
        let answer_row = (approval.kind == "question").then(|| {
            let state = match self.forms.answers.get(&id) {
                Some(state) => state.clone(),
                None => {
                    let state = input(window, cx, "Type your own answer");
                    let submit_id = id.clone();
                    let submit_state = state.clone();
                    self.submit_on_enter(
                        format!("answer:{id}"),
                        &state,
                        window,
                        cx,
                        move |this, _, cx| {
                            let answer = submit_state.read(cx).value().trim().to_string();
                            if !answer.is_empty() {
                                this.resolve_approval(
                                    submit_id.clone(),
                                    "answer",
                                    Some(answer),
                                    cx,
                                );
                            }
                        },
                    );
                    self.forms.answers.insert(id.clone(), state.clone());
                    state
                }
            };
            let typed = state.read(cx).value().trim().to_string();
            let send_id = id.clone();
            h_flex()
                .mt(px(10.0))
                .gap(px(8.0))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(text_input(p, &state, window, cx)),
                )
                .child(button(
                    p,
                    key("send"),
                    Some("Send".into()),
                    None,
                    Tone::Primary,
                    false,
                    busy || typed.is_empty(),
                    move |this: &mut Self, _, cx| {
                        let answer = state.read(cx).value().trim().to_string();
                        if !answer.is_empty() {
                            this.resolve_approval(send_id.clone(), "answer", Some(answer), cx);
                        }
                    },
                    cx,
                ))
        });

        card(p)
            .overflow_hidden()
            .child(header)
            .child(
                v_flex()
                    .p(px(14.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(px(21.0))
                            .text_color(p.foreground)
                            .child(approval.title.clone()),
                    )
                    .when_some(
                        approval
                            .detail
                            .clone()
                            .filter(|detail| !detail.trim().is_empty()),
                        |this, detail| this.child(body_text(p, detail).mt(px(4.0))),
                    )
                    .child(actions)
                    .children(answer_row),
            )
            .into_any_element()
    }
}
