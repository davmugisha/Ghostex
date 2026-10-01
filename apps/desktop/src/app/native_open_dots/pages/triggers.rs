//! Triggers (Open Dot's `Triggers.tsx`): a dot's Composio triggers and the add flow (app, then
//! event, then the event's fields and what the dot should do), and Settings' Composio API key,
//! which turns triggers on.

use std::collections::HashMap;

use super::super::forms::{PendingClear, TriggerFlow, TriggersKeyForm, input_with, textarea};
use super::super::model::{Dot, DotsSnapshot, TriggerApp, TriggerField, TriggerType, weekday_time};
use super::super::style::*;
use super::super::target::OpenDotsTarget;
use super::super::view::NativeOpenDotsView;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::{Map, Value, json};

/// Open Dot's `toConfig`: the form's text as the types the event's schema asks for.
fn to_config(
    fields: &[TriggerField],
    text: &HashMap<String, String>,
    flags: &HashMap<String, bool>,
) -> Result<Value, String> {
    let mut config = Map::new();
    for field in fields {
        if field.field_type == "boolean" {
            config.insert(
                field.name.clone(),
                Value::Bool(flags.get(&field.name).copied().unwrap_or(false)),
            );
            continue;
        }
        let value = text
            .get(&field.name)
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if value.is_empty() {
            if field.required {
                return Err(format!("Fill in {}.", field.title));
            }
            continue;
        }
        let typed = match field.field_type.as_str() {
            "integer" | "number" => {
                let number: f64 = value
                    .parse()
                    .map_err(|_| format!("{} needs a number.", field.title))?;
                if field.field_type == "integer" {
                    json!(number.round() as i64)
                } else {
                    json!(number)
                }
            }
            kind if kind.starts_with("array:") => {
                let items: Vec<&str> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .collect();
                if kind == "array:string" {
                    json!(items)
                } else {
                    json!(
                        items
                            .iter()
                            .filter_map(|item| item.parse::<f64>().ok())
                            .collect::<Vec<_>>()
                    )
                }
            }
            _ => Value::String(value),
        };
        config.insert(field.name.clone(), typed);
    }
    Ok(Value::Object(config))
}

fn default_text(field: &TriggerField) -> String {
    match &field.default {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| item.to_string())
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None | Some(Value::Bool(_)) => String::new(),
        Some(other) => other.to_string(),
    }
}

impl NativeOpenDotsView {
    fn load_trigger_apps(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.forms.trigger_flow.as_mut() else {
            return;
        };
        flow.apps = None;
        self.call(
            "trigger-apps",
            "triggerApps",
            json!({}),
            |this, result, _| {
                if let Some(flow) = this.forms.trigger_flow.as_mut() {
                    flow.apps = Some(result.map(|value| {
                        value
                            .get("apps")
                            .and_then(Value::as_array)
                            .map(|rows| {
                                rows.iter()
                                    .filter_map(|row| serde_json::from_value(row.clone()).ok())
                                    .collect()
                            })
                            .unwrap_or_default()
                    }));
                }
            },
            cx,
        );
    }

    fn pick_trigger_app(&mut self, app: TriggerApp, cx: &mut Context<Self>) {
        let Some(flow) = self.forms.trigger_flow.as_mut() else {
            return;
        };
        let toolkit = app.slug.clone();
        flow.app = Some(app);
        flow.types = None;
        flow.trigger_type = None;
        flow.values.clear();
        flow.flags.clear();
        flow.error = None;
        self.call(
            "trigger-types",
            "triggerTypes",
            json!({ "toolkit": toolkit }),
            |this, result, _| {
                if let Some(flow) = this.forms.trigger_flow.as_mut() {
                    flow.types = Some(result.map(|value| {
                        value
                            .get("types")
                            .and_then(Value::as_array)
                            .map(|rows| {
                                rows.iter()
                                    .filter_map(|row| serde_json::from_value(row.clone()).ok())
                                    .collect()
                            })
                            .unwrap_or_default()
                    }));
                }
            },
            cx,
        );
    }

    fn pick_trigger_type(
        &mut self,
        trigger_type: TriggerType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut values = Vec::new();
        let mut flags = HashMap::new();
        for field in &trigger_type.fields {
            if field.field_type == "boolean" {
                flags.insert(field.name.clone(), field.default == Some(Value::Bool(true)));
            } else if field.choices.is_none() {
                let placeholder = if field.field_type.starts_with("array:") {
                    "Comma-separated".to_string()
                } else {
                    field.description.chars().take(60).collect()
                };
                values.push((
                    field.name.clone(),
                    input_with(window, cx, &placeholder, &default_text(field)),
                ));
            } else {
                values.push((
                    field.name.clone(),
                    input_with(window, cx, "", &default_text(field)),
                ));
            }
        }
        let instruction = textarea(
            window,
            cx,
            "e.g. If it's a bill, add it to my budget sheet and tell me the amount and due date.",
            "",
            3,
            8,
        );
        if let Some(flow) = self.forms.trigger_flow.as_mut() {
            flow.trigger_type = Some(trigger_type);
            flow.values = values;
            flow.flags = flags;
            flow.instruction = Some(instruction);
            flow.error = None;
        }
        cx.notify();
    }

    fn add_trigger(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.forms.trigger_flow.as_ref() else {
            return;
        };
        let (Some(app), Some(trigger_type), Some(instruction)) = (
            flow.app.clone(),
            flow.trigger_type.clone(),
            flow.instruction.clone(),
        ) else {
            return;
        };
        let text: HashMap<String, String> = flow
            .values
            .iter()
            .map(|(name, state)| (name.clone(), state.read(cx).value().to_string()))
            .collect();
        let config = match to_config(&trigger_type.fields, &text, &flow.flags) {
            Ok(config) => config,
            Err(error) => {
                if let Some(flow) = self.forms.trigger_flow.as_mut() {
                    flow.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        let instruction_text = instruction.read(cx).value().trim().to_string();
        let dot_id = flow.dot_id.clone();
        self.call(
            "trigger-add",
            "addTrigger",
            json!({
                "dotId": dot_id,
                "toolkit": app.slug,
                "slug": trigger_type.slug,
                "config": config,
                "instruction": instruction_text,
            }),
            |this, result, _| match result {
                Ok(_) => this.forms.trigger_flow = None,
                Err(error) => {
                    if let Some(flow) = this.forms.trigger_flow.as_mut() {
                        flow.error = Some(error);
                    }
                }
            },
            cx,
        );
    }

    pub(super) fn render_dot_triggers(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if snapshot.settings.triggers_key.is_none() {
            return v_flex()
                .w_full()
                .gap(px(10.0))
                .child(empty_note(
                    p,
                    "Triggers are off. Add a Composio API key in Settings to turn them on.",
                ))
                .child(h_flex().child(button(
                    p,
                    "dots-triggers-open-settings",
                    Some("Open Settings".into()),
                    Some(ICON_SETTINGS),
                    Tone::Secondary,
                    true,
                    false,
                    |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::Settings, cx),
                    cx,
                )))
                .into_any_element();
        }
        let rows: Vec<AnyElement> = snapshot
            .triggers
            .iter()
            .filter(|trigger| trigger.dot_id == dot.id)
            .map(|trigger| {
                let status = trigger.last_error.clone().unwrap_or_else(|| {
                    if !trigger.enabled {
                        "Paused".to_string()
                    } else if let Some(at) = trigger.last_fired_at {
                        format!("Last fired {}", weekday_time(at))
                    } else {
                        "Waiting for the first event".to_string()
                    }
                });
                let toggle_id = trigger.id.clone();
                let delete_id = trigger.id.clone();
                let enabled = trigger.enabled;
                h_flex()
                    .w_full()
                    .pl(px(14.0))
                    .pr(px(6.0))
                    .py(px(10.0))
                    .gap(px(10.0))
                    .items_start()
                    .child(
                        app_initial(p, &trigger.toolkit, 22.0)
                            .when(!trigger.enabled, |this| this.opacity(0.4)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.0))
                            .child(div().text_size(px(14.0)).child(trigger.name.clone()))
                            .child(body_text(p, trigger.instruction.clone()).line_clamp(2))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(if trigger.last_error.is_some() {
                                        p.danger
                                    } else {
                                        p.faint
                                    })
                                    .child(status.to_uppercase()),
                            ),
                    )
                    .child(button(
                        p,
                        SharedString::from(format!("dots-trigger-toggle-{}", trigger.id)),
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
                                format!("trigger-toggle:{toggle_id}"),
                                "setTriggerEnabled",
                                json!({ "triggerId": toggle_id, "enabled": !enabled }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-trigger-delete-{}", trigger.id)),
                        "Delete trigger",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("trigger-delete:{delete_id}"),
                                "deleteTrigger",
                                json!({ "triggerId": delete_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let adding = self
            .forms
            .trigger_flow
            .as_ref()
            .is_some_and(|flow| flow.dot_id == dot.id);
        let dot_id = dot.id.clone();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .when(!rows.is_empty(), |this| this.child(rows_card(p, rows)))
            .when(snapshot.triggers.iter().all(|trigger| trigger.dot_id != dot.id) && !adding, |this| {
                this.child(empty_note(
                    p,
                    format!(
                        "No triggers yet. A trigger wakes {} when something happens, like a new email from your bank or a Slack mention.",
                        dot.name
                    ),
                ))
            })
            .child(if adding {
                self.render_add_trigger(p, dot, window, cx)
            } else {
                h_flex()
                    .child(button(
                        p,
                        "dots-trigger-add-start",
                        Some("Add trigger".into()),
                        Some(ICON_PLUS),
                        Tone::Secondary,
                        false,
                        false,
                        move |this: &mut Self, _, cx| {
                            this.forms.trigger_flow = Some(TriggerFlow {
                                dot_id: dot_id.clone(),
                                apps: None,
                                app: None,
                                types: None,
                                trigger_type: None,
                                values: Vec::new(),
                                flags: HashMap::new(),
                                instruction: None,
                                error: None,
                            });
                            this.load_trigger_apps(cx);
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_add_trigger(
        &mut self,
        p: &DotsPalette,
        dot: &Dot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(flow) = self.forms.trigger_flow.as_ref() else {
            return div().into_any_element();
        };
        let apps = flow.apps.clone();
        let app = flow.app.clone();
        let types = flow.types.clone();
        let trigger_type = flow.trigger_type.clone();
        let values = flow.values.clone();
        let flags = flow.flags.clone();
        let instruction = flow.instruction.clone();
        let error = flow.error.clone();
        let loading_apps = self.is_busy("trigger-apps");
        let hover = p.selected;

        let apps_body: AnyElement = match apps {
            None => note_line(p, "Loading apps…").into_any_element(),
            Some(Err(error)) => error_line(p, error).into_any_element(),
            Some(Ok(apps)) => {
                let any_unconnected = apps.iter().any(|app| !app.connected);
                v_flex()
                    .gap(px(8.0))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .children(apps.into_iter().map(|candidate| {
                                let selected = app.as_ref().is_some_and(|app| app.slug == candidate.slug);
                                let label = if candidate.connected {
                                    candidate.name.clone()
                                } else {
                                    format!("{} · Connect", candidate.name)
                                };
                                let leading = app_initial(p, &candidate.name, 20.0).into_any_element();
                                let pick = candidate.clone();
                                chip(
                                    p,
                                    SharedString::from(format!("dots-trigger-app-{}", candidate.slug)),
                                    label,
                                    Some(leading),
                                    selected,
                                    move |this: &mut Self, _, cx| {
                                        if pick.connected {
                                            this.pick_trigger_app(pick.clone(), cx);
                                        } else {
                                            this.call_and_open_url(
                                                format!("trigger-connect:{}", pick.slug),
                                                "connectTriggerApp",
                                                json!({ "toolkit": pick.slug }),
                                                cx,
                                            );
                                        }
                                    },
                                    cx,
                                )
                            })),
                    )
                    .when(any_unconnected, |this| {
                        this.child(note_line(
                            p,
                            "Apps for triggers are connected in your Composio project, separate from your dots' apps. Connect one, then refresh.",
                        ))
                    })
                    .into_any_element()
            }
        };

        let types_body: Option<AnyElement> = app.as_ref().map(|app| match &types {
            None => note_line(p, "Loading events…").into_any_element(),
            Some(Err(error)) => error_line(p, error.clone()).into_any_element(),
            Some(Ok(types)) if types.is_empty() => {
                note_line(p, format!("{} has no trigger events.", app.name)).into_any_element()
            }
            Some(Ok(types)) => h_flex()
                .w_full()
                .flex_wrap()
                .gap(px(8.0))
                .children(types.iter().map(|candidate| {
                    let selected = trigger_type
                        .as_ref()
                        .is_some_and(|picked| picked.slug == candidate.slug);
                    let pick = candidate.clone();
                    v_flex()
                        .id(SharedString::from(format!(
                            "dots-trigger-type-{}",
                            candidate.slug
                        )))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from(candidate.name.clone()))
                        .aria_selected(selected)
                        .flex_grow_1()
                        .flex_basis(px(260.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded(px(CONTROL_RADIUS))
                        .border_1()
                        .border_color(if selected { p.foreground } else { p.border })
                        .when(selected, |this| this.bg(p.selected))
                        .cursor_pointer()
                        .hover(move |this| this.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_trigger_type(pick.clone(), window, cx)
                        }))
                        .child(div().text_size(px(13.0)).child(candidate.name.clone()))
                        .child(
                            body_text(p, candidate.description.clone())
                                .text_size(px(12.0))
                                .line_clamp(2),
                        )
                }))
                .into_any_element(),
        });

        let fields_body: Option<AnyElement> = trigger_type.as_ref().map(|trigger_type| {
            let field_controls: Vec<AnyElement> = trigger_type
                .fields
                .iter()
                .map(|field| {
                    let label = if field.required {
                        field.title.clone()
                    } else {
                        format!("{} · optional", field.title)
                    };
                    if field.field_type == "boolean" {
                        let name = field.name.clone();
                        let on = flags.get(&field.name).copied().unwrap_or(false);
                        return chip(
                            p,
                            SharedString::from(format!("dots-trigger-flag-{}", field.name)),
                            field.title.clone(),
                            Some(
                                icon(
                                    if on { ICON_CHECK } else { ICON_X },
                                    12.0,
                                    if on { p.primary_text } else { p.faint },
                                )
                                .ml(px(6.0))
                                .into_any_element(),
                            ),
                            on,
                            move |this: &mut Self, _, cx| {
                                if let Some(flow) = this.forms.trigger_flow.as_mut() {
                                    let value = flow.flags.entry(name.clone()).or_insert(false);
                                    *value = !*value;
                                }
                                cx.notify();
                            },
                            cx,
                        )
                        .into_any_element();
                    }
                    let state = values
                        .iter()
                        .find(|(name, _)| name == &field.name)
                        .map(|(_, state)| state.clone());
                    let control: AnyElement = match (&field.choices, state) {
                        (Some(choices), Some(state)) => {
                            let current = state.read(cx).value().to_string();
                            let mut options: Vec<String> = Vec::new();
                            if !field.required {
                                options.push(String::new());
                            }
                            options.extend(choices.iter().cloned());
                            h_flex()
                                .flex_wrap()
                                .gap(px(6.0))
                                .children(options.into_iter().enumerate().map(|(index, option)| {
                                    let state = state.clone();
                                    let selected = option == current;
                                    let label = if option.is_empty() {
                                        "Any".to_string()
                                    } else {
                                        option.clone()
                                    };
                                    chip(
                                        p,
                                        SharedString::from(format!(
                                            "dots-trigger-choice-{}-{index}",
                                            field.name
                                        )),
                                        label,
                                        None,
                                        selected,
                                        move |_this: &mut Self, window, cx| {
                                            let option = option.clone();
                                            state.update(cx, |state, cx| {
                                                state.set_value(option, window, cx)
                                            });
                                        },
                                        cx,
                                    )
                                }))
                                .into_any_element()
                        }
                        (_, Some(state)) => text_input(p, &state, window, cx),
                        (_, None) => div().into_any_element(),
                    };
                    labelled(p, label, control)
                        .flex_grow_1()
                        .flex_basis(px(260.0))
                        .into_any_element()
                })
                .collect();
            let has_interval = trigger_type
                .fields
                .iter()
                .any(|field| field.name == "interval");
            let instruction_typed = instruction
                .as_ref()
                .is_some_and(|state| !state.read(cx).value().trim().is_empty());
            v_flex()
                .w_full()
                .gap(px(12.0))
                .when(!field_controls.is_empty(), |this| {
                    this.child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap(px(12.0))
                            .children(field_controls),
                    )
                })
                .children(instruction.map(|state| {
                    labelled(
                        p,
                        format!("3 · Then {} should", dot.name),
                        text_area(p, &state, 80.0, window, cx),
                    )
                }))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .items_center()
                        .child(note_line(
                            p,
                            if has_interval {
                                "Can take up to the interval (in minutes) to fire."
                            } else {
                                "Runs as events come in."
                            },
                        ))
                        .child(div().flex_1())
                        .child(button(
                            p,
                            "dots-trigger-add",
                            Some(if self.is_busy("trigger-add") {
                                "Adding…".into()
                            } else {
                                "Add trigger".into()
                            }),
                            None,
                            Tone::Primary,
                            true,
                            !instruction_typed || self.is_busy("trigger-add"),
                            |this: &mut Self, _, cx| this.add_trigger(cx),
                            cx,
                        )),
                )
                .into_any_element()
        });

        card(p)
            .p(px(14.0))
            .gap(px(16.0))
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(eyebrow(p, "1 · App"))
                            .child(button(
                                p,
                                "dots-trigger-apps-refresh",
                                None,
                                Some(ICON_REFRESH),
                                Tone::Quiet,
                                true,
                                loading_apps,
                                |this: &mut Self, _, cx| this.load_trigger_apps(cx),
                                cx,
                            )),
                    )
                    .child(apps_body),
            )
            .children(types_body.map(|body| {
                v_flex()
                    .gap(px(8.0))
                    .child(eyebrow(p, "2 · When"))
                    .child(body)
            }))
            .children(fields_body)
            .when_some(error, |this, error| this.child(error_line(p, error)))
            .child(h_flex().justify_end().child(button(
                p,
                "dots-trigger-cancel",
                Some("Cancel".into()),
                None,
                Tone::Quiet,
                true,
                false,
                |this: &mut Self, _, cx| {
                    this.forms.trigger_flow = None;
                    cx.notify();
                },
                cx,
            )))
            .into_any_element()
    }

    fn save_triggers_key(&mut self, value: String, cx: &mut Context<Self>) {
        self.call(
            "triggers-key",
            "setTriggersKey",
            json!({ "key": value }),
            |this, result, _| {
                let error = match result {
                    Ok(value) => value
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    Err(error) => Some(error),
                };
                if let Some(form) = this.forms.triggers_key.as_mut() {
                    if error.is_none() {
                        form.editing = false;
                        this.forms
                            .pending_clears
                            .push(PendingClear::Input(form.key.clone()));
                    }
                    form.error = error;
                }
            },
            cx,
        );
    }

    /// Settings: the Composio API key that turns triggers on (Open Dot's `TriggersKey`).
    pub(super) fn render_triggers_key(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.forms.triggers_key.is_none() {
            let key = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx)
                    .placeholder("ak_...")
                    .masked(true)
            });
            self.forms.triggers_key = Some(TriggersKeyForm {
                key,
                editing: false,
                error: None,
            });
        }
        let source = snapshot.settings.triggers_key.clone();
        let form = self.forms.triggers_key.as_ref().expect("made above");
        let key = form.key.clone();
        let editing = form.editing;
        let error = form.error.clone();
        let typed = !key.read(cx).value().trim().is_empty();
        let busy = self.is_busy("triggers-key");
        let status = match source.as_deref() {
            Some("env") => "Connected from COMPOSIO_API_KEY. Add triggers from a dot's Setup tab.",
            Some(_) => "Connected. Add triggers from a dot's Setup tab.",
            None => {
                "Create a project at platform.composio.dev, copy its API key and paste it here."
            }
        };
        let show_form = editing || source.is_none();
        card(p)
            .p(px(14.0))
            .gap(px(10.0))
            .child(
                h_flex()
                    .gap(px(10.0))
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap(px(4.0))
                                    .child(div().text_size(px(14.0)).child("Composio API key"))
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .text_color(p.faint)
                                            .child("· optional"),
                                    ),
                            )
                            .child(body_text(p, status)),
                    )
                    .when(source.as_deref() == Some("settings") && !editing, |this| {
                        this.child(button(
                            p,
                            "dots-triggers-key-remove",
                            Some("Remove".into()),
                            None,
                            Tone::Quiet,
                            true,
                            busy,
                            |this: &mut Self, _, cx| this.save_triggers_key(String::new(), cx),
                            cx,
                        ))
                        .child(button(
                            p,
                            "dots-triggers-key-change",
                            Some("Change".into()),
                            None,
                            Tone::Secondary,
                            true,
                            false,
                            |this: &mut Self, _, cx| {
                                if let Some(form) = this.forms.triggers_key.as_mut() {
                                    form.editing = true;
                                }
                                cx.notify();
                            },
                            cx,
                        ))
                    }),
            )
            .when(show_form && source.as_deref() != Some("env"), |this| {
                this.child(
                    h_flex()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(text_input(p, &key, window, cx)),
                        )
                        .child(button(
                            p,
                            "dots-triggers-key-save",
                            Some(if busy {
                                "Checking…".into()
                            } else {
                                "Save".into()
                            }),
                            None,
                            Tone::Primary,
                            false,
                            busy || !typed,
                            move |this: &mut Self, _, cx| {
                                let value = key.read(cx).value().trim().to_string();
                                this.save_triggers_key(value, cx);
                            },
                            cx,
                        )),
                )
            })
            .when_some(error, |this, error| this.child(error_line(p, error)))
            .into_any_element()
    }
}
