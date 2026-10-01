//! OpenDots Home (Open Dot's `Home.tsx`): hand off a task to a dot you pick, what needs you, your
//! dots, the latest from them, team chats, and the apps your dots can use. With no dots yet it is
//! Open Dot's "Meet your dots" welcome.

use super::super::forms::textarea;
use super::super::model::{DotsSnapshot, file_size, now_ms, one_line, time_ago};
use super::super::style::*;
use super::super::target::{OpenDotsDotTab, OpenDotsTarget};
use super::super::view::NativeOpenDotsView;
use ghostex_gx_core::open_dots::DotLook;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::text::TextView;
use gpui_component::{h_flex, v_flex};
use serde_json::json;

const SUGGESTED_APPS: [&str; 5] = ["Gmail", "Calendar", "Slack", "Notion", "GitHub"];

impl NativeOpenDotsView {
    fn send_home_message(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let Some(dot_id) = self
            .forms
            .picked_dot
            .clone()
            .filter(|id| snapshot.dot(id).is_some())
            .or_else(|| snapshot.dots.first().map(|dot| dot.id.clone()))
        else {
            return;
        };
        let Some(state) = self.forms.home_message.clone() else {
            return;
        };
        let message = state.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        self.forms
            .pending_clears
            .push(super::super::forms::PendingClear::Textarea(state));
        self.start_chat(dot_id, Some(message), cx);
        cx.notify();
    }

    pub(super) fn render_home(
        &mut self,
        p: &DotsPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(snapshot) = self.snapshot.clone() else {
            return div().into_any_element();
        };
        if snapshot.dots.is_empty() {
            return self.render_welcome(p, cx);
        }
        let now = now_ms();
        let target = self
            .forms
            .picked_dot
            .as_ref()
            .and_then(|id| snapshot.dot(id))
            .or_else(|| snapshot.dots.first())
            .cloned()
            .expect("dots is not empty");

        // The composer.
        let composer = match self.forms.home_message.clone() {
            Some(state) => state,
            None => {
                let state = textarea(window, cx, "", "", 2, 8);
                state.update(cx, |state, cx| state.set_submit_on_enter(true, cx));
                self.submit_on_enter("home", &state, window, cx, |this, _, cx| {
                    this.send_home_message(cx)
                });
                self.forms.home_message = Some(state.clone());
                state
            }
        };
        if self.forms.home_placeholder_dot.as_deref() != Some(target.id.as_str()) {
            self.forms.home_placeholder_dot = Some(target.id.clone());
            let placeholder = format!(
                "Ask {} to research, browse, plan, or build something…",
                target.name
            );
            composer.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            });
        }
        let typed = !composer.read(cx).value().trim().is_empty();
        let sending = self.is_busy(&format!("startChat:{}", target.id));

        let chips = h_flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .gap(px(6.0))
            .children(snapshot.dots.iter().map(|dot| {
                let dot_id = dot.id.clone();
                chip(
                    p,
                    SharedString::from(format!("dots-home-pick-{}", dot.id)),
                    dot.name.clone(),
                    Some(face(&dot.look, dot.face_status(), 20.0).into_any_element()),
                    dot.id == target.id,
                    move |this: &mut Self, _, cx| {
                        this.forms.picked_dot = Some(dot_id.clone());
                        cx.notify();
                    },
                    cx,
                )
            }));
        let send = div()
            .id("dots-home-send")
            .role(gpui::Role::Button)
            .aria_label("Send")
            .flex_shrink_0()
            .size(px(36.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.primary)
            .when(!typed || sending, |this| this.opacity(0.3))
            .when(typed && !sending, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.send_home_message(cx)))
            })
            .child(icon(ICON_ARROW_UP, 16.0, p.primary_text));

        let hero = v_flex()
            .w_full()
            .items_center()
            .pt(px(40.0))
            .pb(px(24.0))
            .child(face(&target.look, target.face_status(), 104.0))
            .child(
                eyebrow(p, "Hand off a task")
                    .mt(px(14.0))
                    .text_color(p.accent),
            )
            .child(
                heading(p, format!("What should {} do?", target.name), 28.0)
                    .mt(px(6.0))
                    .text_center(),
            )
            .child(
                body_text(p, "It works on its own and messages you when it's done.")
                    .mt(px(6.0))
                    .text_center(),
            )
            .child(
                card(p)
                    .mt(px(22.0))
                    .max_w(px(640.0))
                    .p(px(12.0))
                    .gap(px(8.0))
                    .child(text_area(p, &composer, 72.0, window, cx))
                    .child(h_flex().gap(px(8.0)).items_end().child(chips).child(send)),
            );

        v_flex()
            .w_full()
            .gap(px(28.0))
            .child(hero)
            .child(self.render_apps_strip(p, &snapshot, cx))
            .children(self.render_needs_you(p, &snapshot, now, window, cx))
            .child(self.render_dot_grid(p, &snapshot, cx))
            .children(self.render_latest(p, &snapshot, now, cx))
            .child(self.render_teams(p, &snapshot, cx))
            .into_any_element()
    }

    fn render_welcome(&mut self, p: &DotsPalette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .items_center()
            .pt(px(72.0))
            .gap(px(8.0))
            .child(face_for(&DotLook::default(), 180.0))
            .child(eyebrow(p, "Personal agents").mt(px(16.0)).text_color(p.accent))
            .child(heading(p, "Meet your dots", 30.0))
            .child(
                body_text(
                    p,
                    "Dots work on their own. Each one has its own browser and folder, remembers what matters, runs routines on a schedule, and knows when to ask for your approval.",
                )
                .text_center()
                .max_w(px(520.0)),
            )
            .child(h_flex().mt(px(18.0)).child(button(
                p,
                "dots-welcome-create",
                Some("Create your first dot".into()),
                None,
                Tone::Primary,
                false,
                false,
                |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::NewDot, cx),
                cx,
            )))
            .into_any_element()
    }

    /// Composio For You: which apps the dots can use, or a nudge to sign in.
    fn render_apps_strip(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let composio = &snapshot.settings.composio;
        let connected: Vec<String> = composio
            .apps
            .iter()
            .filter(|app| app.connected)
            .map(|app| app.name.clone())
            .collect();
        let logos: Vec<String> = if composio.signed_in && !connected.is_empty() {
            connected.iter().take(8).cloned().collect()
        } else {
            SUGGESTED_APPS.iter().map(|name| name.to_string()).collect()
        };
        let (lead, rest, action) = if composio.signed_in {
            let count = connected.len();
            (
                if count == 0 {
                    "Your dots can use your apps".to_string()
                } else {
                    format!(
                        "Your dots can use {count} app{}",
                        if count == 1 { "" } else { "s" }
                    )
                },
                " · via Composio".to_string(),
                "Manage",
            )
        } else {
            (
                "Give your dots your apps".to_string(),
                " · Gmail, Calendar, Slack, Notion and 500+ more".to_string(),
                "Sign in with Composio",
            )
        };
        let signed_in = composio.signed_in;
        let hover = p.selected;
        h_flex()
            .id("dots-home-apps")
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(lead.clone()))
            .w_full()
            .max_w(px(640.0))
            .mx_auto()
            .px(px(14.0))
            .py(px(10.0))
            .gap(px(12.0))
            .items_center()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .cursor_pointer()
            .hover(move |this| this.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                if signed_in {
                    this.navigate(OpenDotsTarget::Settings, cx);
                } else {
                    this.call_and_open_url("composio-sign-in", "composioSignIn", json!({}), cx);
                }
            }))
            .child(app_stack(
                p,
                &logos.iter().map(String::as_str).collect::<Vec<_>>(),
                26.0,
            ))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .child(div().text_color(p.foreground).child(lead))
                    .child(truncated(rest).text_color(p.faint)),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.accent)
                    .child(action),
            )
            .into_any_element()
    }

    fn render_needs_you(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        now: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pending: Vec<_> = snapshot.pending_approvals().cloned().collect();
        if pending.is_empty() {
            return None;
        }
        let count = pending.len();
        let cards: Vec<AnyElement> = pending
            .iter()
            .map(|approval| {
                let dot = snapshot.dot(&approval.dot_id);
                self.render_approval_card(p, approval, dot, now, window, cx)
            })
            .collect();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(
                h_flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(status_dot(p.warning, 6.0))
                    .child(eyebrow(p, format!("Needs you · {count}"))),
            )
            .children(cards)
            .into_any_element()
            .into()
    }

    fn render_dot_grid(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = p.selected;
        let cards = snapshot
            .dots
            .iter()
            .map(|dot| {
                let dot_id = dot.id.clone();
                let status = dot.face_status();
                h_flex()
                    .id(SharedString::from(format!("dots-home-dot-{}", dot.id)))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(dot.name.clone()))
                    .w_full()
                    .min_w_0()
                    .p(px(14.0))
                    .gap(px(12.0))
                    .items_start()
                    .rounded(px(CARD_RADIUS))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(
                            OpenDotsTarget::Dot {
                                dot_id: dot_id.clone(),
                                tab: OpenDotsDotTab::Chats,
                            },
                            cx,
                        )
                    }))
                    .child(face(&dot.look, status, 40.0))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(3.0))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(px(15.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(dot.name.clone()),
                                    )
                                    .child(icon(ICON_ARROW_UP_RIGHT, 15.0, p.faint)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .line_height(px(18.0))
                                    .text_color(p.muted)
                                    .line_clamp(2)
                                    .child(if dot.purpose.trim().is_empty() {
                                        "General helper".to_string()
                                    } else {
                                        dot.purpose.clone()
                                    }),
                            )
                            .child(
                                h_flex()
                                    .mt(px(6.0))
                                    .gap(px(6.0))
                                    .items_center()
                                    .child(status_dot(p.status_color(status), 6.0))
                                    .child(
                                        truncated(dot.status_word().to_uppercase())
                                            .text_size(px(10.5))
                                            .text_color(p.faint),
                                    ),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .pt(px(20.0))
            .border_t_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(eyebrow(p, format!("Your dots · {}", snapshot.dots.len())))
                    .child(button(
                        p,
                        "dots-home-new-dot",
                        Some("New dot".into()),
                        Some(ICON_PLUS),
                        Tone::Quiet,
                        true,
                        false,
                        |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::NewDot, cx),
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .grid()
                    .grid_cols(self.grid_columns())
                    .gap(px(12.0))
                    .children(cards),
            )
            .into_any_element()
    }

    fn render_latest(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        now: i64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let updates: Vec<_> = snapshot.updates.iter().take(12).cloned().collect();
        if updates.is_empty() {
            return None;
        }
        let unread = snapshot
            .updates
            .iter()
            .filter(|update| !update.read)
            .count();
        let hover = p.hover;
        let rows: Vec<AnyElement> = updates
            .iter()
            .map(|update| {
                let dot = snapshot.dot(&update.dot_id);
                let expanded = self.expanded_update.as_deref() == Some(update.id.as_str());
                let update_id = update.id.clone();
                let read = update.read;
                let row = h_flex()
                    .id(SharedString::from(format!("dots-update-{}", update.id)))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from(
                        update
                            .title
                            .clone()
                            .unwrap_or_else(|| one_line(&update.text, 80)),
                    ))
                    .aria_expanded(expanded)
                    .w_full()
                    .min_w_0()
                    .px(px(14.0))
                    .py(px(10.0))
                    .gap(px(10.0))
                    .items_center()
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.expanded_update.as_deref() == Some(update_id.as_str()) {
                            this.expanded_update = None;
                        } else {
                            this.expanded_update = Some(update_id.clone());
                            if !read {
                                this.act(
                                    format!("read:{update_id}"),
                                    "markUpdatesRead",
                                    json!({ "updateIds": [update_id.clone()] }),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    }))
                    .child(status_dot(
                        if update.read {
                            gpui::transparent_black()
                        } else {
                            p.accent
                        },
                        6.0,
                    ))
                    .when_some(dot, |this, dot| {
                        this.child(face(&dot.look, dot.face_status(), 22.0)).child(
                            truncated(dot.name.clone())
                                .w(px(72.0))
                                .flex_shrink_0()
                                .text_size(px(13.5)),
                        )
                    })
                    .when_some(
                        update
                            .title
                            .clone()
                            .filter(|title| !title.trim().is_empty()),
                        |this, title| this.child(badge(title, p.accent)),
                    )
                    .child(
                        truncated(one_line(&update.text, 200))
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(p.muted),
                    )
                    .when(!update.files.is_empty(), |this| {
                        this.child(icon(ICON_FILE, 13.0, p.faint)).child(
                            div()
                                .text_size(px(11.0))
                                .text_color(p.faint)
                                .child(update.files.len().to_string()),
                        )
                    })
                    .child(note_line(p, time_ago(update.created_at, now)).flex_shrink_0());
                let body = expanded.then(|| self.render_update_body(p, update, cx));
                v_flex()
                    .w_full()
                    .child(row)
                    .children(body)
                    .into_any_element()
            })
            .collect();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(eyebrow(p, "Latest from your dots"))
                    .when(unread > 0, |this| {
                        this.child(button(
                            p,
                            "dots-mark-all-read",
                            Some("Mark all read".into()),
                            Some(ICON_CHECK),
                            Tone::Quiet,
                            true,
                            self.is_busy("read:all"),
                            |this: &mut Self, _, cx| {
                                this.act("read:all", "markUpdatesRead", json!({}), cx)
                            },
                            cx,
                        ))
                    }),
            )
            .child(rows_card(p, rows))
            .into_any_element()
            .into()
    }

    fn render_update_body(
        &mut self,
        p: &DotsPalette,
        update: &super::super::model::DotUpdate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files = update.files.iter().enumerate().map(|(index, file)| {
            let open = file.path.clone();
            let reveal = file.path.clone();
            h_flex()
                .gap(px(8.0))
                .items_center()
                .child(icon(ICON_FILE, 14.0, p.faint))
                .child(truncated(file.name.clone()).flex_1().text_size(px(13.0)))
                .child(note_line(p, file_size(file.size)))
                .child(button(
                    p,
                    SharedString::from(format!("dots-update-file-open-{}-{index}", update.id)),
                    Some("Open".into()),
                    None,
                    Tone::Secondary,
                    true,
                    false,
                    move |this: &mut Self, _, cx| this.open_path(open.clone(), false, cx),
                    cx,
                ))
                .child(button(
                    p,
                    SharedString::from(format!("dots-update-file-reveal-{}-{index}", update.id)),
                    Some("Show in folder".into()),
                    None,
                    Tone::Quiet,
                    true,
                    false,
                    move |this: &mut Self, _, cx| this.open_path(reveal.clone(), true, cx),
                    cx,
                ))
        });
        let session = update
            .session_id
            .clone()
            .map(|session| (update.project_id.clone(), session));
        v_flex()
            .w_full()
            .px(px(14.0))
            .pb(px(12.0))
            .pl(px(30.0))
            .gap(px(10.0))
            .child(
                TextView::markdown(
                    SharedString::from(format!("dots-update-text-{}", update.id)),
                    update.text.clone(),
                )
                .min_w_0()
                .selectable(true)
                .style(p.markdown.clone())
                .text_size(px(13.5))
                .text_color(p.foreground.opacity(0.9)),
            )
            .children(files)
            .when_some(session, |this, (project_id, session_id)| {
                this.child(h_flex().child(button(
                    p,
                    SharedString::from(format!("dots-update-open-chat-{}", update.id)),
                    Some("Open chat".into()),
                    Some(ICON_MESSAGES),
                    Tone::Quiet,
                    true,
                    false,
                    move |this: &mut Self, _, cx| {
                        this.focus_session(project_id.clone(), session_id.clone(), cx)
                    },
                    cx,
                )))
            })
            .into_any_element()
    }

    fn render_teams(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows: Vec<AnyElement> = snapshot
            .teams
            .iter()
            .map(|team| {
                let members = team.member_ids.iter().filter_map(|id| snapshot.dot(id));
                let lead = snapshot
                    .dot(&team.lead_id)
                    .map(|dot| dot.name.clone())
                    .unwrap_or_default();
                let project_id = team.project_id.clone();
                let session_id = team.session_id.clone();
                let team_id = team.id.clone();
                h_flex()
                    .w_full()
                    .px(px(14.0))
                    .py(px(10.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(icon(ICON_TEAM, 16.0, p.muted))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(truncated(format!("#{}", team.name)).text_size(px(14.0)))
                            .child(note_line(
                                p,
                                format!("{} dots · led by {lead}", team.member_ids.len()),
                            )),
                    )
                    .child(h_flex().children(members.enumerate().map(|(index, dot)| {
                        face(&dot.look, dot.face_status(), 24.0)
                            .when(index > 0, |this| this.ml(px(-4.0)))
                    })))
                    .child(button(
                        p,
                        SharedString::from(format!("dots-team-open-{}", team.id)),
                        Some("Open".into()),
                        Some(ICON_MESSAGES),
                        Tone::Secondary,
                        true,
                        session_id.is_empty(),
                        move |this: &mut Self, _, cx| {
                            this.focus_session(project_id.clone(), session_id.clone(), cx)
                        },
                        cx,
                    ))
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-team-delete-{}", team.id)),
                        "Delete team chat",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("team:{team_id}"),
                                "deleteTeam",
                                json!({ "teamId": team_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(eyebrow(p, "Team chats"))
                    .child(button(
                        p,
                        "dots-home-new-team",
                        Some("New team chat".into()),
                        Some(ICON_PLUS),
                        Tone::Quiet,
                        true,
                        snapshot.dots.len() < 2,
                        |this: &mut Self, _, cx| this.navigate(OpenDotsTarget::NewTeam, cx),
                        cx,
                    )),
            )
            .child(if rows.is_empty() {
                empty_note(
                    p,
                    if snapshot.dots.len() < 2 {
                        "A team chat puts several dots on one job. Make a second dot to start one."
                    } else {
                        "A team chat puts several dots on one job: the lead hands out the work and brings you the result."
                    },
                )
                .into_any_element()
            } else {
                rows_card(p, rows).into_any_element()
            })
            .into_any_element()
    }
}
