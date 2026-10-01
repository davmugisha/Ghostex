//! A dot's Setup tab (Open Dot's `SetupPane.tsx`): identity and look, rules, routines, triggers,
//! memory, skills and the danger zone. The rule editor (Open Dot's `RuleEditor`) is also the
//! Settings page's "Rules for all dots".

use super::super::forms::{PendingClear, RoutineForm, RuleForm, input, input_with};
use super::super::look_editor::LookTarget;
use super::super::model::{Dot, DotsSnapshot, weekday_time};
use super::super::style::*;
use super::super::target::OpenDotsTarget;
use super::super::view::NativeOpenDotsView;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::json;

/// Open Dot's schedule presets.
const SCHEDULES: [(&str, &str); 4] = [
    ("Every morning", "0 8 * * *"),
    ("Weekdays 9am", "0 9 * * 1-5"),
    ("Every hour", "0 * * * *"),
    ("Mondays", "0 9 * * 1"),
];

/// Open Dot's rule decisions.
const DECISIONS: [(&str, &str); 3] = [
    ("allow", "Allow automatically"),
    ("ask", "Ask first"),
    ("never", "Never allow"),
];

impl NativeOpenDotsView {
    fn decision_color(p: &DotsPalette, decision: &str) -> gpui::Hsla {
        match decision {
            "allow" => p.success,
            "never" => p.danger,
            _ => p.warning,
        }
    }

    fn add_rule(&mut self, dot_id: Option<String>, cx: &mut Context<Self>) {
        let Some(form) = self.forms.rules.get(&dot_id) else {
            return;
        };
        let action = form.action.read(cx).value().trim().to_string();
        if action.is_empty() {
            return;
        }
        let decision = form.decision;
        self.forms
            .pending_clears
            .push(PendingClear::Input(form.action.clone()));
        let key = format!("rule:{}", dot_id.clone().unwrap_or_default());
        self.act(
            key,
            "addRule",
            json!({ "dotId": dot_id, "action": action, "decision": decision }),
            cx,
        );
    }

    /// Open Dot's `RuleEditor`: the rules for one dot (`dot_id`) or for every dot (`None`).
    pub(super) fn render_rule_editor(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot_id: Option<String>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.forms.rules.contains_key(&dot_id) {
            let action = input(window, cx, "reply to emails for me");
            let submit_dot = dot_id.clone();
            self.submit_on_enter(
                format!("rule:{}", dot_id.clone().unwrap_or_default()),
                &action,
                window,
                cx,
                move |this, _, cx| this.add_rule(submit_dot.clone(), cx),
            );
            self.forms.rules.insert(
                dot_id.clone(),
                RuleForm {
                    action,
                    decision: "ask",
                },
            );
        }
        let form_action = self.forms.rules[&dot_id].action.clone();
        let form_decision = self.forms.rules[&dot_id].decision;
        let typed = !form_action.read(cx).value().trim().is_empty();
        let prefix = dot_id.clone().unwrap_or_else(|| "all".to_string());
        let rules: Vec<AnyElement> = snapshot
            .rules
            .iter()
            .filter(|rule| rule.dot_id == dot_id)
            .map(|rule| {
                let label = DECISIONS
                    .iter()
                    .find(|(value, _)| *value == rule.decision)
                    .map(|(_, label)| *label)
                    .unwrap_or("Ask first");
                let rule_id = rule.id.clone();
                h_flex()
                    .w_full()
                    .pl(px(14.0))
                    .pr(px(6.0))
                    .py(px(8.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .flex_wrap()
                            .gap(px(4.0))
                            .text_size(px(13.0))
                            .text_color(p.muted)
                            .child(format!("When {name} wants to"))
                            .child(div().text_color(p.foreground).child(rule.action.clone())),
                    )
                    .child(badge(label, Self::decision_color(p, &rule.decision)))
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-rule-delete-{}", rule.id)),
                        "Delete rule",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("rule-delete:{rule_id}"),
                                "deleteRule",
                                json!({ "ruleId": rule_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let decision_chips = DECISIONS
            .iter()
            .map(|(value, label)| {
                let value = *value;
                let rule_dot = dot_id.clone();
                chip(
                    p,
                    SharedString::from(format!("dots-rule-decision-{prefix}-{value}")),
                    *label,
                    None,
                    value == form_decision,
                    move |this: &mut Self, _, cx| {
                        if let Some(form) = this.forms.rules.get_mut(&rule_dot) {
                            form.decision = value;
                        }
                        cx.notify();
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let add_dot = dot_id.clone();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .when(!rules.is_empty(), |this| this.child(rows_card(p, rules)))
            .child(
                card(p)
                    .p(px(14.0))
                    .gap(px(12.0))
                    .child(labelled(p, format!("When {name} wants to"), text_input(p, &form_action, window, cx)))
                    .child(labelled(
                        p,
                        "It should",
                        h_flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .items_center()
                            .children(decision_chips)
                            .child(div().flex_1())
                            .child(button(
                                p,
                                SharedString::from(format!("dots-rule-add-{prefix}")),
                                Some("Add rule".into()),
                                None,
                                Tone::Primary,
                                true,
                                !typed || self.is_busy(&format!("rule:{}", dot_id.clone().unwrap_or_default())),
                                move |this: &mut Self, _, cx| this.add_rule(add_dot.clone(), cx),
                                cx,
                            ))
                            .into_any_element(),
                    )),
            )
            .child(note_line(
                p,
                "One short, plain-language rule per action. \u{201c}Ask first\u{201d} wins if rules conflict. Built-in safety checks always apply.",
            ))
            .into_any_element()
    }

    fn save_identity(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.setup.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_string();
        let purpose = form.purpose.read(cx).value().trim().to_string();
        let instructions = form.instructions.read(cx).value().to_string();
        let look = form.look.clone();
        let dot_id = form.dot_id.clone();
        let saved = (
            name.clone(),
            purpose.clone(),
            instructions.clone(),
            look.clone(),
        );
        self.call(
            format!("identity:{dot_id}"),
            "updateDot",
            json!({
                "dotId": dot_id,
                "name": name,
                "purpose": purpose,
                "instructions": instructions,
                "look": look.to_value(),
            }),
            move |this, result, _| match result {
                Ok(_) => {
                    if let Some(form) = this.forms.setup.as_mut() {
                        form.saved = saved;
                    }
                }
                Err(error) => this.notice = Some(error),
            },
            cx,
        );
    }

    fn add_routine(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.routine.as_ref() else {
            return;
        };
        let name = form.name.read(cx).value().trim().to_string();
        let instruction = form.instruction.read(cx).value().trim().to_string();
        let schedule = form.schedule.read(cx).value().trim().to_string();
        if instruction.is_empty() {
            return;
        }
        let dot_id = form.dot_id.clone();
        let clears = [form.name.clone(), form.instruction.clone()];
        let name = if name.is_empty() {
            instruction.chars().take(40).collect()
        } else {
            name
        };
        self.call(
            format!("routine-add:{dot_id}"),
            "addRoutine",
            json!({ "dotId": dot_id, "name": name, "instruction": instruction, "schedule": schedule }),
            move |this, result, _| {
                let error = result.err();
                if error.is_none() {
                    this.forms
                        .pending_clears
                        .extend(clears.into_iter().map(PendingClear::Input));
                }
                if let Some(form) = this.forms.routine.as_mut() {
                    form.error = error;
                }
            },
            cx,
        );
    }

    fn add_memory(&mut self, dot_id: String, cx: &mut Context<Self>) {
        let Some(state) = self.forms.memory.clone() else {
            return;
        };
        let text = state.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.forms.pending_clears.push(PendingClear::Input(state));
        self.act(
            format!("memory-add:{dot_id}"),
            "addMemory",
            json!({ "dotId": dot_id, "text": text }),
            cx,
        );
    }

    pub(super) fn render_setup(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_setup_form(dot, window, cx);
        let name = dot.name.clone();
        v_flex()
            .w_full()
            .child(page_header(
                p,
                "Setup",
                format!("How {name} works"),
                Some("Personality, rules, routines, memory, and skills. Your dot also updates these itself as you work together.".into()),
            ))
            .child(self.render_identity(p, window, cx))
            .child(section(
                p,
                "Approvals",
                "Rules",
                Some(format!("{name} knows when to take action and when to ask for approval. Add rules for more control.").into()),
                self.render_rule_editor(p, snapshot, Some(dot.id.clone()), &name, window, cx),
            ))
            .child(section(
                p,
                "Automation",
                "Routines",
                Some(format!("Recurring tasks {name} runs on a schedule. You can also just ask it in a chat.").into()),
                self.render_routines(p, snapshot, dot, window, cx),
            ))
            .child(section(
                p,
                "Automation",
                "Triggers",
                Some(format!("Wake {name} when something happens in your apps, like a new email or a Slack mention. It runs in its own chat and still asks before sending anything.").into()),
                self.render_dot_triggers(p, snapshot, dot, window, cx),
            ))
            .child(section(
                p,
                "Memory",
                "What it knows about you",
                Some(format!("{name} adds to this as you work together. Remove anything that's wrong.").into()),
                self.render_memory(p, snapshot, dot, window, cx),
            ))
            .child(section(
                p,
                "Skills",
                "Repeatable know-how",
                Some("Markdown instructions it follows for tasks it repeats. Ask your dot to save one after it figures something out.".into()),
                self.render_skills(p, snapshot, dot, cx),
            ))
            .child(section(
                p,
                "Danger zone",
                format!("Delete {name}"),
                Some("Removes its rules, memory, routines, triggers and browser, and closes its project. Its folder stays on this computer. This can't be undone.".into()),
                self.render_delete(p, dot, cx),
            ))
            .into_any_element()
    }

    fn render_identity(
        &mut self,
        p: &DotsPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = self.forms.setup.as_ref() else {
            return div().into_any_element();
        };
        let name = form.name.clone();
        let purpose = form.purpose.clone();
        let instructions = form.instructions.clone();
        let look = form.look.clone();
        let current = (
            name.read(cx).value().trim().to_string(),
            purpose.read(cx).value().trim().to_string(),
            instructions.read(cx).value().to_string(),
            look.clone(),
        );
        let dirty = current != form.saved;
        let busy = self.is_busy(&format!("identity:{}", form.dot_id));
        let editor = self.render_look_editor(p, LookTarget::Setup, &look, cx);
        section(
            p,
            "Identity",
            "Personality & appearance",
            Some("Its name, job, and how you want it to work.".into()),
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(px(24.0))
                .items_start()
                .child(
                    v_flex()
                        .flex_grow_1()
                        .flex_basis(px(420.0))
                        .min_w(px(280.0))
                        .gap(px(14.0))
                        .child(labelled(p, "Name", text_input(p, &name, window, cx)))
                        .child(labelled(p, "Job", text_input(p, &purpose, window, cx)))
                        .child(labelled(
                            p,
                            "Instructions",
                            text_area(p, &instructions, 110.0, window, cx),
                        ))
                        .child(editor),
                )
                .child(
                    v_flex()
                        .w(px(220.0))
                        .flex_shrink_0()
                        .gap(px(10.0))
                        .child(
                            card(p)
                                .items_center()
                                .py(px(18.0))
                                .child(face_for(&look, 160.0)),
                        )
                        .child(
                            button(
                                p,
                                "dots-setup-save",
                                Some(if dirty {
                                    "Save changes".into()
                                } else {
                                    "Saved".into()
                                }),
                                None,
                                Tone::Primary,
                                false,
                                !dirty || busy,
                                |this: &mut Self, _, cx| this.save_identity(cx),
                                cx,
                            )
                            .w_full()
                            .justify_center(),
                        ),
                ),
        )
        .border_t_0()
        .pt(px(4.0))
        .into_any_element()
    }

    fn render_routines(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self
            .forms
            .routine
            .as_ref()
            .is_none_or(|form| form.dot_id != dot.id)
        {
            self.forms.routine = Some(RoutineForm {
                dot_id: dot.id.clone(),
                name: input(window, cx, "Name"),
                instruction: input(window, cx, "What to do each time"),
                schedule: input_with(window, cx, "Cron schedule", SCHEDULES[0].1),
                error: None,
            });
        }
        let form = self.forms.routine.as_ref().expect("made above");
        let (name_input, instruction_input, schedule_input) = (
            form.name.clone(),
            form.instruction.clone(),
            form.schedule.clone(),
        );
        let error = form.error.clone();
        let schedule_now = schedule_input.read(cx).value().trim().to_string();
        let typed = !instruction_input.read(cx).value().trim().is_empty();
        let routines: Vec<AnyElement> = snapshot
            .routines
            .iter()
            .filter(|routine| routine.dot_id == dot.id)
            .map(|routine| {
                let mut when = if routine.enabled {
                    routine
                        .next_run_at
                        .map(|at| format!("Next {}", weekday_time(at)))
                        .unwrap_or_else(|| "Scheduled".to_string())
                } else {
                    "Paused".to_string()
                };
                if let Some(last) = routine.last_run_at {
                    when.push_str(&format!(" · Last {}", weekday_time(last)));
                }
                let run_id = routine.id.clone();
                let toggle_id = routine.id.clone();
                let delete_id = routine.id.clone();
                let enabled = routine.enabled;
                h_flex()
                    .w_full()
                    .pl(px(14.0))
                    .pr(px(6.0))
                    .py(px(10.0))
                    .gap(px(10.0))
                    .items_start()
                    .child(
                        status_dot(if routine.enabled { p.success } else { p.faint }, 7.0)
                            .mt(px(6.0)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.0))
                            .child(
                                h_flex()
                                    .gap(px(8.0))
                                    .items_center()
                                    .child(div().text_size(px(14.0)).child(routine.name.clone()))
                                    .child(
                                        div()
                                            .px(px(6.0))
                                            .rounded(px(4.0))
                                            .bg(p.raised)
                                            .text_size(px(11.0))
                                            .text_color(p.muted)
                                            .child(routine.schedule.clone()),
                                    ),
                            )
                            .child(body_text(p, routine.instruction.clone()).line_clamp(2))
                            .child(note_line(p, when.to_uppercase()).text_size(px(10.5))),
                    )
                    .child(button(
                        p,
                        SharedString::from(format!("dots-routine-run-{}", routine.id)),
                        Some("Run".into()),
                        Some(ICON_PLAY),
                        Tone::Quiet,
                        true,
                        self.is_busy(&format!("routine-run:{}", routine.id)),
                        move |this: &mut Self, _, cx| {
                            this.call(
                                format!("routine-run:{run_id}"),
                                "runRoutineNow",
                                json!({ "routineId": run_id }),
                                |this, result, cx| match result {
                                    Ok(value) => this.focus_session_from(&value, cx),
                                    Err(error) => this.notice = Some(error),
                                },
                                cx,
                            )
                        },
                        cx,
                    ))
                    .child(button(
                        p,
                        SharedString::from(format!("dots-routine-toggle-{}", routine.id)),
                        Some(if enabled {
                            "Pause".into()
                        } else {
                            "Resume".into()
                        }),
                        None,
                        Tone::Quiet,
                        true,
                        false,
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("routine-toggle:{toggle_id}"),
                                "updateRoutine",
                                json!({ "routineId": toggle_id, "enabled": !enabled }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-routine-delete-{}", routine.id)),
                        "Delete routine",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("routine-delete:{delete_id}"),
                                "deleteRoutine",
                                json!({ "routineId": delete_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let presets = SCHEDULES
            .iter()
            .enumerate()
            .map(|(index, (label, cron))| {
                let cron = *cron;
                let state = schedule_input.clone();
                chip(
                    p,
                    ("dots-routine-preset", index),
                    *label,
                    None,
                    schedule_now == cron,
                    move |_this: &mut Self, window, cx| {
                        state.update(cx, |state, cx| state.set_value(cron, window, cx));
                    },
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let busy = self.is_busy(&format!("routine-add:{}", dot.id));
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(if routines.is_empty() {
                empty_note(p, "No routines yet. Try \u{201c}Every weekday at 8am, brief me on AI news\u{201d} in a chat.").into_any_element()
            } else {
                rows_card(p, routines).into_any_element()
            })
            .child(
                card(p)
                    .p(px(14.0))
                    .gap(px(10.0))
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap(px(10.0))
                            .child(div().w(px(180.0)).flex_shrink_0().child(text_input(p, &name_input, window, cx)))
                            .child(div().flex_1().min_w(px(220.0)).child(text_input(p, &instruction_input, window, cx))),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap(px(6.0))
                            .items_center()
                            .children(presets)
                            .child(div().w(px(150.0)).child(text_input(p, &schedule_input, window, cx)))
                            .child(div().flex_1())
                            .child(button(
                                p,
                                "dots-routine-add",
                                Some("Add routine".into()),
                                None,
                                Tone::Primary,
                                true,
                                !typed || busy,
                                |this: &mut Self, _, cx| this.add_routine(cx),
                                cx,
                            )),
                    )
                    .child(note_line(p, "Schedules are cron: minute hour day month weekday, in this computer's time."))
                    .when_some(error, |this, error| this.child(error_line(p, error))),
            )
            .into_any_element()
    }

    fn render_memory(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.forms.memory.is_none() {
            let state = input(window, cx, "Add something it should remember");
            let dot_id = dot.id.clone();
            self.submit_on_enter("memory", &state, window, cx, move |this, _, cx| {
                this.add_memory(dot_id.clone(), cx)
            });
            self.forms.memory = Some(state);
        }
        let state = self.forms.memory.clone().expect("made above");
        let typed = !state.read(cx).value().trim().is_empty();
        let rows: Vec<AnyElement> = snapshot
            .memories
            .iter()
            .filter(|memory| memory.dot_id == dot.id)
            .map(|memory| {
                let memory_id = memory.id.clone();
                h_flex()
                    .w_full()
                    .pl(px(14.0))
                    .pr(px(6.0))
                    .py(px(6.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.5))
                            .child(memory.text.clone()),
                    )
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-memory-delete-{}", memory.id)),
                        "Forget",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("memory-delete:{memory_id}"),
                                "deleteMemory",
                                json!({ "memoryId": memory_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let dot_id = dot.id.clone();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(if rows.is_empty() {
                empty_note(p, "Nothing yet.").into_any_element()
            } else {
                rows_card(p, rows).into_any_element()
            })
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(text_input(p, &state, window, cx)),
                    )
                    .child(button(
                        p,
                        "dots-memory-add",
                        Some("Add".into()),
                        None,
                        Tone::Secondary,
                        false,
                        !typed,
                        move |this: &mut Self, _, cx| this.add_memory(dot_id.clone(), cx),
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_skills(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = p.hover;
        let rows: Vec<AnyElement> = snapshot
            .skills
            .iter()
            .filter(|skill| skill.dot_id == dot.id)
            .map(|skill| {
                let open = self.skill_open.as_deref() == Some(skill.id.as_str());
                let toggle_id = skill.id.clone();
                let delete_id = skill.id.clone();
                v_flex()
                    .w_full()
                    .child(
                        h_flex()
                            .w_full()
                            .pl(px(14.0))
                            .pr(px(6.0))
                            .py(px(8.0))
                            .gap(px(10.0))
                            .items_center()
                            .child(
                                v_flex()
                                    .id(SharedString::from(format!("dots-skill-{}", skill.id)))
                                    .role(gpui::Role::Button)
                                    .aria_label(SharedString::from(skill.name.clone()))
                                    .aria_expanded(open)
                                    .flex_1()
                                    .min_w_0()
                                    .rounded(px(6.0))
                                    .cursor_pointer()
                                    .hover(move |this| this.bg(hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.skill_open = if this.skill_open.as_deref()
                                            == Some(toggle_id.as_str())
                                        {
                                            None
                                        } else {
                                            Some(toggle_id.clone())
                                        };
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(skill.name.clone()),
                                    )
                                    .child(
                                        truncated(skill.description.clone())
                                            .text_size(px(13.0))
                                            .text_color(p.muted),
                                    ),
                            )
                            .child(remove_button(
                                p,
                                SharedString::from(format!("dots-skill-delete-{}", skill.id)),
                                "Delete skill",
                                move |this: &mut Self, _, cx| {
                                    this.act(
                                        format!("skill-delete:{delete_id}"),
                                        "deleteSkill",
                                        json!({ "skillId": delete_id }),
                                        cx,
                                    )
                                },
                                cx,
                            )),
                    )
                    .when(open, |this| {
                        this.child(
                            div()
                                .mx(px(14.0))
                                .mb(px(10.0))
                                .p(px(12.0))
                                .rounded(px(8.0))
                                .bg(p.raised)
                                .font_family(crate::app::window::native_modal_kit::MODAL_MONO_FONT)
                                .text_size(px(12.0))
                                .line_height(px(18.0))
                                .text_color(p.foreground.opacity(0.8))
                                .child(skill.body.clone()),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            empty_note(p, "No skills yet.").into_any_element()
        } else {
            rows_card(p, rows).into_any_element()
        }
    }

    fn render_delete(&mut self, p: &DotsPalette, dot: &Dot, cx: &mut Context<Self>) -> AnyElement {
        let name = dot.name.clone();
        let dot_id = dot.id.clone();
        if self.confirm_delete {
            h_flex()
                .gap(px(8.0))
                .child(button(
                    p,
                    "dots-delete-confirm",
                    Some(format!("Yes, delete {name}").into()),
                    None,
                    Tone::DangerFilled,
                    false,
                    self.is_busy(&format!("delete:{dot_id}")),
                    move |this: &mut Self, _, cx| {
                        this.confirm_delete = false;
                        this.call(
                            format!("delete:{dot_id}"),
                            "deleteDot",
                            json!({ "dotId": dot_id }),
                            |this, result, cx| match result {
                                Ok(_) => this.navigate(OpenDotsTarget::Home, cx),
                                Err(error) => this.notice = Some(error),
                            },
                            cx,
                        );
                    },
                    cx,
                ))
                .child(button(
                    p,
                    "dots-delete-cancel",
                    Some("Cancel".into()),
                    None,
                    Tone::Quiet,
                    false,
                    false,
                    |this: &mut Self, _, cx| {
                        this.confirm_delete = false;
                        cx.notify();
                    },
                    cx,
                ))
                .into_any_element()
        } else {
            button(
                p,
                "dots-delete",
                Some(format!("Delete {name}…").into()),
                None,
                Tone::Danger,
                false,
                false,
                |this: &mut Self, _, cx| {
                    this.confirm_delete = true;
                    cx.notify();
                },
                cx,
            )
            .into_any_element()
        }
    }
}
