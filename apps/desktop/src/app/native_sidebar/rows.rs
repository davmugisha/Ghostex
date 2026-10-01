use super::drag::SidebarDropTarget;
use super::{appearance::SidebarAppearance, model::NativeSidebarGroup};
use crate::GhostexGpuiApp;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, px,
};
use gpui_component::v_flex;
use serde_json::{Value, json};

impl GhostexGpuiApp {
    /// CDXC:Bots 2026-09-27 DECISION:
    /// User: New Session under a bot starts a session with that bot, the same launch as its "+", so it opens in chat or terminal per the agent-interface setting instead of a plain terminal.
    ///
    /// The "New Session" row of a group with no sessions: a project's opens a terminal. A dot's
    /// ("New chat") starts a chat with the dot, the same launch as its "+".
    fn start_native_sidebar_empty_group_session(
        &mut self,
        group_id: &str,
        launch_action: Option<&'static str>,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(action) = launch_action {
            self.dispatch_native_sidebar_ui(
                json!({"type": "projectAction", "action": action, "groupId": group_id}),
                cx,
            );
        } else {
            self.dispatch_native_sidebar_command(
                json!({"type": "createProjectTerminal", "groupId": group_id}),
                cx,
            );
        }
    }

    pub(crate) fn render_native_sidebar_group(
        &self,
        group: &NativeSidebarGroup,
        hud: &Value,
        appearance: &SidebarAppearance,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().clone();
        let group_id = group.group_id.clone();
        let key = format!("group:{}", group.group_id);
        let scale = appearance.scale;
        // CDXC:Sidebar 2026-09-24 DECISION:
        // User: don't show the Sessions label under a project or worktree when it's the only label there (no pinned, drafts or parked sessions). It comes back while a session is dragged, next to the Pinned and Parked drop headings.
        let hide_lone_sessions_heading = matches!(group.sections.as_slice(), [only] if only.id == "sessions")
            && self
                .native_sidebar_missing_drop_sections(group, None, None)
                .is_empty();
        let body = self
            .native_sidebar
            .disclosures
            .present(&key, group.collapsed)
            .then(|| {
                let content = v_flex()
                    .w_full()
                    .pb(px(3.0 * scale))
                    .when(
                        group.sessions.is_empty() && group.project_context.is_some(),
                        |column| {
                            let id = group.group_id.clone();
                            let is_dot = group.dot_status().is_some();
                            let launch_action = if is_dot {
                                Some("dot")
                            } else if group.bot_gateway_running().is_some() {
                                Some("bot")
                            } else {
                                None
                            };
                            column.child(
                                div()
                                    .id(format!("native-empty-project-{id}"))
                                    .h(px(34.0 * scale))
                                    .mx(px(3.0 * scale))
                                    .pl(px(26.0 * scale))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.0 * scale))
                                    .hover(|row| row.bg(appearance.hover))
                                    .relative()
                                    .child(if is_dot { "New chat" } else { "New Session" })
                                    .sidebar_drop_target("session-group", id.clone(), None, cx)
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        cx.stop_propagation();
                                        app.start_native_sidebar_empty_group_session(
                                            &id,
                                            launch_action,
                                            cx,
                                        );
                                    })),
                            )
                        },
                    )
                    .children(
                        group
                            .sections
                            .iter()
                            .enumerate()
                            .flat_map(|(index, section)| {
                                // A session drag shows the sections it can land in that are empty
                                // (section_move.rs), each in its own place among the drawn ones.
                                let placeholders = self
                                    .native_sidebar_missing_drop_sections(
                                        group,
                                        index
                                            .checked_sub(1)
                                            .map(|previous| group.sections[previous].id.as_str()),
                                        Some(section.id.as_str()),
                                    )
                                    .into_iter()
                                    .map(|id| {
                                        self.render_native_section_drop_placeholder(
                                            group, id, appearance, cx,
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                let key = format!("section:{}:{}", group.group_id, section.id);
                                let ids = self.native_sidebar.disclosures.section_ids(
                                    &key,
                                    &section.session_ids,
                                    section.collapsed,
                                );
                                let body = self
                                    .native_sidebar
                                    .disclosures
                                    .present(&key, section.collapsed)
                                    .then(|| {
                                        let sessions = group
                                            .sessions
                                            .iter()
                                            .map(|session| (session.session_id.as_str(), session))
                                            .collect::<std::collections::HashMap<_, _>>();
                                        let rows = self.render_native_session_list(
                                            group,
                                            ids.iter()
                                                .filter_map(|id| {
                                                    sessions
                                                        .get(id.as_str())
                                                        .map(|session| (*session).clone())
                                                })
                                                .collect(),
                                            appearance,
                                            cx,
                                        );
                                        self.render_native_disclosure(key, rows, cx)
                                    });
                                let section_column = v_flex()
                                    .w_full()
                                    .flex_shrink_0()
                                    .when(
                                        index > 0 && group.sections[index - 1].collapsed,
                                        |column| column.mt(px(8.0 * scale)),
                                    )
                                    .when(!hide_lone_sessions_heading, |column| {
                                        column.child(self.render_native_section_header(
                                            group, section, appearance, cx,
                                        ))
                                    })
                                    .children(body)
                                    .into_any_element();
                                placeholders
                                    .into_iter()
                                    .chain(std::iter::once(section_column))
                            }),
                    )
                    .children(
                        self.native_sidebar_missing_drop_sections(
                            group,
                            group.sections.last().map(|section| section.id.as_str()),
                            None,
                        )
                        .into_iter()
                        .map(|id| {
                            self.render_native_section_drop_placeholder(group, id, appearance, cx)
                        }),
                    )
                    .when(
                        group.hidden_session_count > 0 && !group.expanded,
                        |column| {
                            let storage_id = group.storage_id.clone();
                            column.child(
                                div()
                                    .id(format!("native-sidebar-list-toggle-{}", group.group_id))
                                    .mx(px(3.0 * scale))
                                    .h(px(34.0 * scale))
                                    .pl(px(26.0 * scale))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.0 * scale))
                                    .hover(|row| row.bg(appearance.session_hover))
                                    .child(format!(
                                        "Show all ({})",
                                        group
                                            .sections
                                            .iter()
                                            .filter(|section| !section.collapsed)
                                            .map(|section| section.count)
                                            .sum::<usize>()
                                    ))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        cx.stop_propagation();
                                        app.dispatch_native_sidebar_ui(
                                            json!({"type": "toggleList", "groupId": storage_id}),
                                            cx,
                                        );
                                    })),
                            )
                        },
                    );
                self.render_native_disclosure(key, content.into_any_element(), cx)
            });
        // The line of a project drop is drawn on the project's whole block, header and sessions,
        // because a project dropped after an expanded one lands after its sessions too.
        let drop_line = self.native_sidebar_project_drop_line("group", &group.group_id);
        let block = v_flex()
            .when(
                self.native_sidebar.is_dragging("group", &group.group_id),
                |row| row.opacity(0.18),
            )
            .on_children_prepainted(move |bounds, _, cx| {
                if let (Some(first), Some(last)) = (bounds.first(), bounds.last()) {
                    let bounds = gpui::Bounds {
                        origin: first.origin,
                        size: gpui::size(first.size.width, last.bottom() - first.top()),
                    };
                    view.update(cx, |app, cx| {
                        app.record_native_project_bounds(&group_id, bounds, cx)
                    });
                }
            })
            .w_full()
            .flex_shrink_0()
            .child(self.render_native_project_header(group, hud, appearance, cx))
            .children(body);
        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .child(block)
            .when_some(drop_line, |row, position| {
                row.child(super::drag::drop_line(position, scale))
            })
            .into_any_element()
    }
}
