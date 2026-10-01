//! OpenDots settings (Open Dot's `SettingsView.tsx`): rules for all dots, saved logins, your apps
//! via Composio, the triggers key, and where OpenDots keeps things. Open Dot's OpenAI, OpenRouter
//! and E2B keys have no counterpart: a dot runs on the agents Ghostex already runs, on this
//! computer.

use super::super::forms::{PasswordForm, PendingClear, input};
use super::super::model::DotsSnapshot;
use super::super::style::*;
use super::super::view::NativeOpenDotsView;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use gpui_component::input::InputState;
use gpui_component::{h_flex, v_flex};
use serde_json::json;

impl NativeOpenDotsView {
    fn save_password(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.forms.password.as_ref() else {
            return;
        };
        let site = form.site.read(cx).value().trim().to_string();
        let username = form.username.read(cx).value().trim().to_string();
        let password = form.password.read(cx).value().to_string();
        let fields = [
            form.site.clone(),
            form.username.clone(),
            form.password.clone(),
        ];
        if site.is_empty() || username.is_empty() || password.is_empty() {
            if let Some(form) = self.forms.password.as_mut() {
                form.error = Some("Fill in the site, the username and the password.".to_string());
            }
            cx.notify();
            return;
        }
        self.call(
            "password-add",
            "addPassword",
            json!({ "site": site, "username": username, "password": password }),
            move |this, result, _| {
                let error = result.err();
                if error.is_none() {
                    this.forms
                        .pending_clears
                        .extend(fields.into_iter().map(PendingClear::Input));
                }
                if let Some(form) = this.forms.password.as_mut() {
                    form.error = error;
                }
            },
            cx,
        );
    }

    pub(super) fn render_settings(
        &mut self,
        p: &DotsPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        v_flex()
            .w_full()
            .child(page_header(
                p,
                "Settings",
                "OpenDots settings",
                Some("Saved logins, rules that apply to every dot, and the apps your dots can use.".into()),
            ))
            .child(
                section(
                    p,
                    "Approvals",
                    "Rules for all dots",
                    Some("These apply to every dot, on top of each dot's own rules.".into()),
                    self.render_rule_editor(p, &snapshot, None, "a dot", window, cx),
                )
                .border_t_0()
                .pt(px(4.0)),
            )
            .child(section(
                p,
                "Passwords",
                "Saved logins",
                Some(SharedString::from(
                    "Your dots can use these to sign in to websites in their browser. The password is typed straight into the page and never shown to the dot's agent.",
                )),
                self.render_passwords(p, &snapshot, window, cx),
            ))
            .child(section(
                p,
                "Apps",
                "Your apps, via Composio",
                Some("Sign in with your Composio account to give your dots Gmail, Calendar, Slack, Notion, GitHub, and 500+ more apps. Dots read on their own and ask before sending, posting, or changing anything.".into()),
                self.render_apps(p, &snapshot, cx),
            ))
            .child(section(
                p,
                "Triggers",
                "Wake dots from your apps",
                Some("Let a dot act when something happens, like a new email or a GitHub issue. Triggers run through a Composio developer project, so they need its API key. Then add them from a dot's Setup tab.".into()),
                self.render_triggers_key(p, &snapshot, window, cx),
            ))
            .child(section(
                p,
                "Notifications",
                "When a dot needs you",
                Some("A dot that needs your approval or posts an update shows a notification, following Ghostex's attention notification setting.".into()),
                div(),
            ))
            .child(section(
                p,
                "Storage",
                "Where OpenDots keeps things",
                None,
                self.render_storage_notes(p, &snapshot),
            ))
            .into_any_element()
    }

    fn render_passwords(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.forms.password.is_none() {
            let password = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Password")
                    .masked(true)
            });
            self.forms.password = Some(PasswordForm {
                site: input(window, cx, "Site, e.g. github.com"),
                username: input(window, cx, "Username or email"),
                password,
                error: None,
            });
        }
        let form = self.forms.password.as_ref().expect("made above");
        let (site, username, password) = (
            form.site.clone(),
            form.username.clone(),
            form.password.clone(),
        );
        let error = form.error.clone();
        let rows: Vec<AnyElement> = snapshot
            .passwords
            .iter()
            .map(|entry| {
                let password_id = entry.id.clone();
                h_flex()
                    .w_full()
                    .pl(px(14.0))
                    .pr(px(6.0))
                    .py(px(6.0))
                    .gap(px(10.0))
                    .items_center()
                    .child(icon(ICON_KEY, 14.0, p.faint))
                    .child(
                        truncated(entry.site.clone())
                            .w(px(180.0))
                            .flex_shrink_0()
                            .text_size(px(14.0)),
                    )
                    .child(
                        truncated(entry.username.clone())
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(p.muted),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(p.faint)
                            .child("••••••••"),
                    )
                    .child(remove_button(
                        p,
                        SharedString::from(format!("dots-password-delete-{}", entry.id)),
                        "Delete saved login",
                        move |this: &mut Self, _, cx| {
                            this.act(
                                format!("password-delete:{password_id}"),
                                "deletePassword",
                                json!({ "passwordId": password_id }),
                                cx,
                            )
                        },
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let vault = match snapshot.settings.vault.as_str() {
            "keychain" => "AES-256-GCM, key in your Keychain",
            _ => "AES-256-GCM, key in a private file on this computer",
        };
        v_flex()
            .w_full()
            .gap(px(10.0))
            .child(if rows.is_empty() {
                empty_note(p, "No saved logins yet.").into_any_element()
            } else {
                rows_card(p, rows).into_any_element()
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
                            .child(
                                div()
                                    .flex_grow_1()
                                    .flex_basis(px(180.0))
                                    .child(text_input(p, &site, window, cx)),
                            )
                            .child(
                                div()
                                    .flex_grow_1()
                                    .flex_basis(px(180.0))
                                    .child(text_input(p, &username, window, cx)),
                            )
                            .child(
                                div()
                                    .flex_grow_1()
                                    .flex_basis(px(180.0))
                                    .child(text_input(p, &password, window, cx)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .items_center()
                            .child(icon(ICON_LOCK, 12.0, p.faint))
                            .child(note_line(p, vault))
                            .child(div().flex_1())
                            .when_some(error, |this, error| this.child(error_line(p, error)))
                            .child(button(
                                p,
                                "dots-password-save",
                                Some("Save login".into()),
                                None,
                                Tone::Primary,
                                true,
                                self.is_busy("password-add"),
                                |this: &mut Self, _, cx| this.save_password(cx),
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_apps(
        &mut self,
        p: &DotsPalette,
        snapshot: &DotsSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let composio = &snapshot.settings.composio;
        if !composio.signed_in {
            return card(p)
                .p(px(16.0))
                .gap(px(10.0))
                .child(
                    h_flex()
                        .gap(px(14.0))
                        .items_center()
                        .flex_wrap()
                        .child(app_stack(p, &["Gmail", "Calendar", "Slack", "Notion", "GitHub"], 30.0))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(200.0))
                                .child(div().text_size(px(15.0)).child("Composio For You"))
                                .child(body_text(p, "One sign-in connects your dots to the apps you already use. No API keys.")),
                        )
                        .child(button(
                            p,
                            "dots-composio-sign-in",
                            Some("Sign in with Composio".into()),
                            None,
                            Tone::Primary,
                            false,
                            self.is_busy("composio-sign-in"),
                            |this: &mut Self, _, cx| {
                                this.call_and_open_url("composio-sign-in", "composioSignIn", json!({}), cx)
                            },
                            cx,
                        )),
                )
                .child(note_line(p, "Sign-in opens in your browser. Come back here when it's done."))
                .into_any_element();
        }
        let connected: Vec<_> = composio
            .apps
            .iter()
            .filter(|app| app.connected)
            .cloned()
            .collect();
        let suggested: Vec<_> = composio
            .apps
            .iter()
            .filter(|app| !app.connected)
            .cloned()
            .collect();
        let busy = self.is_busy("composio");
        v_flex()
            .w_full()
            .gap(px(12.0))
            .child(
                card(p).px(px(14.0)).py(px(10.0)).child(
                    h_flex()
                        .gap(px(10.0))
                        .items_center()
                        .child(app_initial(p, "Composio", 24.0))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(px(14.0)).child("Composio For You"))
                                .child(note_line(
                                    p,
                                    format!(
                                        "Signed in · {} app{} connected",
                                        connected.len(),
                                        if connected.len() == 1 { "" } else { "s" }
                                    ),
                                )),
                        )
                        .child(badge("Connected", p.success))
                        .child(button(
                            p,
                            "dots-composio-refresh",
                            None,
                            Some(ICON_REFRESH),
                            Tone::Quiet,
                            true,
                            busy,
                            |this: &mut Self, _, cx| {
                                this.act("composio", "composioRefresh", json!({}), cx)
                            },
                            cx,
                        ))
                        .child(button(
                            p,
                            "dots-composio-sign-out",
                            Some("Sign out".into()),
                            None,
                            Tone::Quiet,
                            true,
                            busy,
                            |this: &mut Self, _, cx| {
                                this.act("composio", "composioSignOut", json!({}), cx)
                            },
                            cx,
                        )),
                ),
            )
            .when(!connected.is_empty(), |this| {
                this.child(h_flex().w_full().flex_wrap().gap(px(8.0)).children(
                    connected.iter().map(|app| {
                        h_flex()
                            .flex_grow_1()
                            .flex_basis(px(220.0))
                            .px(px(12.0))
                            .py(px(8.0))
                            .gap(px(10.0))
                            .items_center()
                            .rounded(px(CONTROL_RADIUS))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.card)
                            .child(app_initial(p, &app.name, 20.0))
                            .child(truncated(app.name.clone()).flex_1().text_size(px(14.0)))
                            .child(status_dot(p.success, 6.0))
                    }),
                ))
            })
            .when(!suggested.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap(px(8.0))
                        .child(eyebrow(p, "Connect more"))
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap(px(6.0))
                                .children(suggested.iter().map(|app| {
                                    let slug = app.slug.clone();
                                    chip(
                                        p,
                                        SharedString::from(format!(
                                            "dots-composio-connect-{}",
                                            app.slug
                                        )),
                                        format!("{} +", app.name),
                                        Some(app_initial(p, &app.name, 20.0).into_any_element()),
                                        false,
                                        move |this: &mut Self, _, cx| {
                                            // The dots' own app connection (Composio's manage-connections flow).
                                            this.call_and_open_url(
                                                format!("composio-connect:{slug}"),
                                                "composioConnectApp",
                                                json!({ "toolkit": slug }),
                                                cx,
                                            )
                                        },
                                        cx,
                                    )
                                })),
                        )
                        .child(note_line(
                            p,
                            "Connecting opens in your browser. Press refresh when it's done.",
                        )),
                )
            })
            .into_any_element()
    }

    fn render_storage_notes(&self, p: &DotsPalette, snapshot: &DotsSnapshot) -> AnyElement {
        let folder = snapshot
            .dots
            .first()
            .and_then(|dot| {
                dot.folder
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_string())
            })
            .unwrap_or_else(|| "~/.local/share/ghostex/dots".to_string());
        let chrome = snapshot.settings.chrome.clone().unwrap_or_else(|| {
            "Google Chrome is not installed, so dots cannot browse.".to_string()
        });
        let rows: Vec<AnyElement> = [
            ("Dots' folders", folder),
            ("Browser", chrome),
            ("Records", "Rules, memory, skills, routines and updates stay in Ghostex's state on this computer.".to_string()),
        ]
        .into_iter()
        .map(|(label, value)| {
            h_flex()
                .w_full()
                .px(px(14.0))
                .py(px(10.0))
                .gap(px(14.0))
                .items_start()
                .child(eyebrow(p, label).w(px(120.0)).flex_shrink_0().pt(px(2.0)))
                .child(div().flex_1().min_w_0().text_size(px(13.0)).text_color(p.muted).child(value))
                .into_any_element()
        })
        .collect();
        rows_card(p, rows).into_any_element()
    }
}
