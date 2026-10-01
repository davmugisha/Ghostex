//! New dot (Open Dot's `NewDot.tsx`): a live preview beside the form (name, job, a starter to begin
//! from, the look, and the agent and model it works with), then Create.

use super::super::forms::{NewDotForm, input, textarea};
use super::super::look_editor::LookTarget;
use super::super::model::now_ms;
use super::super::style::*;
use super::super::target::{OpenDotsDotTab, OpenDotsTarget};
use super::super::view::NativeOpenDotsView;
use ghostex_gx_core::open_dots::{DOT_NAME_IDEAS, DOT_STARTERS, DotLook};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::{Value, json};

impl NativeOpenDotsView {
    fn create_dot(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.new_dot.as_ref() else {
            return;
        };
        let typed = form.name.read(cx).value().trim().to_string();
        let name = if typed.is_empty() {
            form.placeholder.to_string()
        } else {
            typed
        };
        let mut params = json!({
            "name": name,
            "purpose": form.purpose.read(cx).value().trim().to_string(),
            "look": form.look.to_value(),
        });
        if let Some(agent_id) = form.agent_id.clone() {
            params["agentId"] = Value::String(agent_id);
        }
        if let Some(model) = form.model.clone() {
            params["model"] = Value::String(model);
        }
        self.call(
            "create-dot",
            "createDot",
            params,
            |this, result, cx| match result {
                Ok(value) => {
                    this.forms.new_dot = None;
                    if let Some(dot_id) = value.pointer("/dot/id").and_then(Value::as_str) {
                        this.navigate(
                            OpenDotsTarget::Dot {
                                dot_id: dot_id.to_string(),
                                tab: OpenDotsDotTab::Chats,
                            },
                            cx,
                        );
                    }
                }
                Err(error) => this.notice = Some(error),
            },
            cx,
        );
    }

    pub(super) fn render_new_dot(
        &mut self,
        p: &DotsPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.forms.new_dot.is_none() {
            let placeholder = DOT_NAME_IDEAS[(now_ms() as usize / 1000) % DOT_NAME_IDEAS.len()];
            self.forms.new_dot = Some(NewDotForm {
                name: input(window, cx, placeholder),
                purpose: textarea(window, cx, "What should it help with?", "", 3, 8),
                look: DotLook::default(),
                placeholder,
                agent_id: None,
                model: None,
            });
        }
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let form = self.forms.new_dot.as_ref().expect("made above");
        let name_state = form.name.clone();
        let purpose_state = form.purpose.clone();
        let look = form.look.clone();
        let placeholder = form.placeholder;
        let agent_id = form
            .agent_id
            .clone()
            .unwrap_or_else(|| snapshot.settings.default_agent_id.clone());
        let model = form.model.clone();
        let typed_name = name_state.read(cx).value().trim().to_string();
        let shown_name = if typed_name.is_empty() {
            placeholder.to_string()
        } else {
            typed_name
        };
        let typed_purpose = purpose_state.read(cx).value().trim().to_string();
        let shown_purpose = if typed_purpose.is_empty() {
            "Works on its own, in its own folder and browser.".to_string()
        } else {
            typed_purpose
        };

        let stage = v_flex()
            .flex_grow_1()
            .flex_basis(px(320.0))
            .min_w(px(260.0))
            .items_center()
            .justify_center()
            .pt(px(14.0))
            .pb(px(32.0))
            .gap(px(10.0))
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(eyebrow(p, "Preview").self_start().ml(px(16.0)))
            .child(face_for(&look, 220.0))
            .child(heading(p, shown_name.clone(), 22.0))
            .child(
                body_text(p, shown_purpose)
                    .text_center()
                    .max_w(px(380.0))
                    .px(px(16.0)),
            );

        let hover = p.hover;
        let starters = rows_card(
            p,
            DOT_STARTERS
                .iter()
                .enumerate()
                .map(|(index, starter)| {
                    let starter_look = (starter.look)();
                    let name = starter.name;
                    let purpose = starter.purpose;
                    let applied_look = starter_look.clone();
                    h_flex()
                        .id(("dots-starter", index))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(format!("Start from {name}")))
                        .w_full()
                        .px(px(12.0))
                        .py(px(8.0))
                        .gap(px(10.0))
                        .items_center()
                        .cursor_pointer()
                        .hover(move |this| this.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(form) = this.forms.new_dot.as_mut() {
                                form.look = applied_look.clone();
                                let (name_state, purpose_state) =
                                    (form.name.clone(), form.purpose.clone());
                                name_state
                                    .update(cx, |state, cx| state.set_value(name, window, cx));
                                purpose_state
                                    .update(cx, |state, cx| state.set_value(purpose, window, cx));
                            }
                            cx.notify();
                        }))
                        .child(face_for(&starter_look, 28.0))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(px(14.0)).child(name))
                                .child(truncated(purpose).text_size(px(12.0)).text_color(p.muted)),
                        )
                        .child(icon(ICON_ARROW_RIGHT, 14.0, p.faint))
                        .into_any_element()
                })
                .collect(),
        );

        let agent_chips = snapshot
            .settings
            .agents
            .iter()
            .map(|agent| {
                let id = agent.id.clone();
                chip(
                    p,
                    SharedString::from(format!("dots-new-agent-{}", agent.id)),
                    agent.name.clone(),
                    None,
                    agent.id == agent_id,
                    move |this: &mut Self, _, cx| {
                        if let Some(form) = this.forms.new_dot.as_mut() {
                            form.agent_id = Some(id.clone());
                            form.model = None;
                        }
                        cx.notify();
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let models = snapshot
            .agent(&agent_id)
            .map(|agent| agent.models.clone())
            .unwrap_or_default();
        let model_chips = std::iter::once((None, "Default".to_string()))
            .chain(
                models
                    .into_iter()
                    .map(|model| (Some(model.id), model.label)),
            )
            .map(|(choice, label)| {
                let selected = choice == model;
                let id = SharedString::from(format!(
                    "dots-new-model-{}",
                    choice.clone().unwrap_or_else(|| "default".to_string())
                ));
                chip(
                    p,
                    id,
                    label,
                    None,
                    selected,
                    move |this: &mut Self, _, cx| {
                        if let Some(form) = this.forms.new_dot.as_mut() {
                            form.model = choice.clone();
                        }
                        cx.notify();
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();

        let editor = self.render_look_editor(p, LookTarget::NewDot, &look, cx);
        let creating = self.is_busy("create-dot");
        let form_column = v_flex()
            .flex_grow_1()
            .flex_basis(px(420.0))
            .min_w(px(300.0))
            .gap(px(16.0))
            .child(
                v_flex()
                    .gap(px(6.0))
                    .child(eyebrow(p, "New dot").text_color(p.accent))
                    .child(heading(p, "Meet a new teammate", 24.0))
                    .child(body_text(
                        p,
                        "Give it a name and a job. It gets its own folder, browser, memory, and rules.",
                    )),
            )
            .child(labelled(p, "Name", text_input(p, &name_state, window, cx)))
            .child(labelled(p, "Job", text_area(p, &purpose_state, 80.0, window, cx)))
            .child(labelled(p, "Or start from", starters.into_any_element()))
            .child(labelled(p, "Appearance", editor))
            .when(!snapshot.settings.agents.is_empty(), |this| {
                this.child(labelled(
                    p,
                    "Works with",
                    h_flex().flex_wrap().gap(px(6.0)).children(agent_chips).into_any_element(),
                ))
                .child(labelled(
                    p,
                    "Model",
                    h_flex().flex_wrap().gap(px(6.0)).children(model_chips).into_any_element(),
                ))
            })
            .child(
                button(
                    p,
                    "dots-new-create",
                    Some(if creating {
                        "Waking up…".into()
                    } else {
                        format!("Create {shown_name}").into()
                    }),
                    None,
                    Tone::Primary,
                    false,
                    creating,
                    |this: &mut Self, _, cx| this.create_dot(cx),
                    cx,
                )
                .w_full()
                .justify_center()
                .h(px(40.0)),
            );

        h_flex()
            .w_full()
            .pt(px(28.0))
            .flex_wrap()
            .gap(px(24.0))
            .items_start()
            .child(stage)
            .child(form_column)
            .into_any_element()
    }
}
