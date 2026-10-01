//! New team chat (Open Dot's `NewChannel.tsx`): a name, the dots on the team with one lead, and
//! the first message. The team chat is a conversation with the lead, which hands parts of the job
//! to the others and brings back the combined result.

use super::super::forms::{TeamForm, input, textarea};
use super::super::style::*;
use super::super::view::NativeOpenDotsView;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{h_flex, v_flex};
use serde_json::json;

/// Open Dot's channel names: lower case, spaces as dashes.
fn team_slug(name: &str) -> String {
    name.trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

impl NativeOpenDotsView {
    fn create_team(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.team.as_ref() else {
            return;
        };
        let lead = form
            .lead
            .clone()
            .filter(|lead| form.members.contains(lead))
            .or_else(|| form.members.first().cloned());
        let Some(lead) = lead else {
            return;
        };
        let name = team_slug(&form.name.read(cx).value());
        let name = if name.is_empty() {
            "team".to_string()
        } else {
            name
        };
        let message = form.message.read(cx).value().trim().to_string();
        let members = form.members.clone();
        self.call(
            "team-create",
            "startTeamChat",
            json!({ "name": name, "leadId": lead, "memberIds": members, "message": message }),
            |this, result, cx| match result {
                Ok(value) => {
                    this.forms.team = None;
                    this.focus_session_from(&value, cx);
                    this.navigate(super::super::target::OpenDotsTarget::Home, cx);
                }
                Err(error) => this.notice = Some(error),
            },
            cx,
        );
    }

    pub(super) fn render_new_team(
        &mut self,
        p: &DotsPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.forms.team.is_none() {
            self.forms.team = Some(TeamForm {
                name: input(window, cx, "launch-plan"),
                message: textarea(window, cx, "What should the team do?", "", 3, 10),
                members: Vec::new(),
                lead: None,
            });
        }
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let form = self.forms.team.as_ref().expect("made above");
        let name_state = form.name.clone();
        let message_state = form.message.clone();
        let members = form.members.clone();
        let lead = form
            .lead
            .clone()
            .filter(|lead| members.contains(lead))
            .or_else(|| members.first().cloned());
        let shown_name = {
            let slug = team_slug(&name_state.read(cx).value());
            if slug.is_empty() {
                "team".to_string()
            } else {
                slug
            }
        };
        let typed_message = !message_state.read(cx).value().trim().is_empty();
        let hover = p.hover;
        let rows: Vec<AnyElement> = snapshot
            .dots
            .iter()
            .map(|dot| {
                let on = members.contains(&dot.id);
                let is_lead = lead.as_deref() == Some(dot.id.as_str());
                let toggle_id = dot.id.clone();
                let lead_id = dot.id.clone();
                h_flex()
                    .w_full()
                    .px(px(14.0))
                    .py(px(8.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("dots-team-member-{}", dot.id)))
                            .role(gpui::Role::CheckBox)
                            .aria_label(SharedString::from(dot.name.clone()))
                            .aria_toggled(if on {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .flex_1()
                            .min_w_0()
                            .gap(px(10.0))
                            .items_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(move |this| this.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(form) = this.forms.team.as_mut() {
                                    if let Some(index) =
                                        form.members.iter().position(|id| id == &toggle_id)
                                    {
                                        form.members.remove(index);
                                    } else {
                                        form.members.push(toggle_id.clone());
                                    }
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .size(px(16.0))
                                    .flex_shrink_0()
                                    .rounded(px(4.0))
                                    .border_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(on, |this| {
                                        this.bg(p.primary).border_color(p.primary).child(icon(
                                            ICON_CHECK,
                                            11.0,
                                            p.primary_text,
                                        ))
                                    })
                                    .when(!on, |this| this.border_color(p.faint)),
                            )
                            .child(face(&dot.look, dot.face_status(), 28.0))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(div().text_size(px(14.0)).child(dot.name.clone()))
                                    .child(
                                        truncated(dot.purpose.clone())
                                            .text_size(px(12.0))
                                            .text_color(p.muted),
                                    ),
                            ),
                    )
                    .when(on, |this| {
                        this.child(button(
                            p,
                            SharedString::from(format!("dots-team-lead-{}", dot.id)),
                            Some(if is_lead {
                                "Lead".into()
                            } else {
                                "Make lead".into()
                            }),
                            Some(ICON_CROWN),
                            if is_lead {
                                Tone::Secondary
                            } else {
                                Tone::Quiet
                            },
                            true,
                            false,
                            move |this: &mut Self, _, cx| {
                                if let Some(form) = this.forms.team.as_mut() {
                                    form.lead = Some(lead_id.clone());
                                }
                                cx.notify();
                            },
                            cx,
                        ))
                    })
                    .into_any_element()
            })
            .collect();
        let ready = members.len() >= 2 && lead.is_some() && typed_message;
        let creating = self.is_busy("team-create");
        let label: SharedString = if members.len() < 2 {
            "Pick at least two dots".into()
        } else if !typed_message {
            "Write the first message".into()
        } else if creating {
            "Starting…".into()
        } else {
            format!("Start #{shown_name}").into()
        };
        v_flex()
            .w_full()
            .gap(px(18.0))
            .child(page_header(
                p,
                "New team chat",
                "Put a team on it",
                Some("A team chat is one conversation with several dots. The lead coordinates: it hands parts of the job to the others and brings you the combined result.".into()),
            ))
            .child(labelled(
                p,
                "Name",
                h_flex()
                    .gap(px(6.0))
                    .items_center()
                    .child(div().text_size(px(14.0)).text_color(p.faint).child("#"))
                    .child(div().flex_1().child(text_input(p, &name_state, window, cx)))
                    .into_any_element(),
            ))
            .child(labelled(
                p,
                "Members · pick the lead with the crown",
                if rows.is_empty() {
                    empty_note(p, "Make a few dots first.").into_any_element()
                } else {
                    rows_card(p, rows).into_any_element()
                },
            ))
            .child(labelled(p, "First message", text_area(p, &message_state, 90.0, window, cx)))
            .child(
                button(
                    p,
                    "dots-team-create",
                    Some(label),
                    None,
                    Tone::Primary,
                    false,
                    !ready || creating,
                    |this: &mut Self, _, cx| this.create_team(cx),
                    cx,
                )
                .w_full()
                .justify_center()
                .h(px(40.0)),
            )
            .into_any_element()
    }
}
