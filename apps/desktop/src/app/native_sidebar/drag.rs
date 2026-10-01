use crate::GhostexGpuiApp;
use gpui::{
    AnyElement, Bounds, Context, DragMoveEvent, InteractiveElement, IntoElement, ParentElement,
    Pixels, Point, Render, Styled, Window, div, px,
};
use serde_json::{Value, json};

#[derive(Clone)]
pub(crate) struct SidebarDrag {
    pub(crate) kind: &'static str,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) scale: f32,
    pub(crate) preview: SidebarDragPreview,
}

#[derive(Clone)]
pub(crate) enum SidebarDragPreview {
    Space(super::space_drag::SpaceDragPreview),
    Row(super::row_drag::RowDragPreview),
}

impl Render for SidebarDrag {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.preview {
            SidebarDragPreview::Space(space) => space.render(self.scale, window),
            SidebarDragPreview::Row(row) => row.render(&self.title, window),
        }
    }
}

impl GhostexGpuiApp {
    pub(crate) fn update_native_sidebar_drop(
        &mut self,
        event: &DragMoveEvent<SidebarDrag>,
        target_kind: &str,
        target_id: &str,
        group_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let source = event.drag(cx).clone();
        self.resolve_native_sidebar_drop(
            &source,
            event.event.position,
            event.bounds,
            target_kind,
            target_id,
            group_id,
            cx,
        );
    }

    fn resolve_native_sidebar_drop(
        &mut self,
        source: &SidebarDrag,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        target_kind: &str,
        target_id: &str,
        group_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        // CDXC:Sidebar 2026-09-17 WHY:
        // GPUI broadcasts drag moves even outside a target. Only the row under the pointer may choose the drop command; otherwise later rows overwrite it.
        if !bounds.contains(&position)
            || (!matches!(target_kind, "space" | "space-row")
                && !self.native_sidebar.scroll.bounds().contains(&position))
        {
            return;
        }
        if target_kind == "space-row" && source.kind != "space" {
            return;
        }
        if source.kind == "space" && matches!(target_kind, "space" | "space-row") {
            let SidebarDragPreview::Space(space) = &source.preview else {
                return;
            };
            let command = space.drop_command(
                &source.id,
                target_kind,
                target_id,
                position,
                bounds,
                source.scale,
            );
            if self.native_sidebar.drop_command != command {
                self.native_sidebar.drop_command = command;
                cx.notify();
            }
            return;
        }
        if source.kind == target_kind && source.id == target_id {
            self.native_sidebar.drop_command = None;
            cx.notify();
            return;
        }
        let after = if target_kind == "space" {
            position.x > bounds.center().x
        } else {
            position.y > bounds.center().y
        };
        let position = if after { "after" } else { "before" };
        // A session aimed at another section of its project moves there (section_move.rs); a
        // section heading takes nothing else.
        if source.kind == "session" && matches!(target_kind, "section" | "session") {
            if let Some(command) = self.native_sidebar_section_move_command(
                &source.id,
                target_kind,
                target_id,
                group_id,
                position,
            ) {
                self.set_native_sidebar_session_drop(Some(command), cx);
                return;
            }
        }
        if target_kind == "section" {
            if self.native_sidebar.drop_command.take().is_some() {
                cx.notify();
            }
            return;
        }
        if source.kind == "session" {
            let command = match target_kind {
                "session" => Some(
                    json!({"type": "moveSession", "sessionId": source.id, "groupId": group_id, "targetSessionId": target_id, "position": position}),
                ),
                "group" | "session-group" => Some(
                    json!({"type": "moveSession", "sessionId": source.id, "groupId": target_id, "position": "before"}),
                ),
                _ => None,
            };
            self.set_native_sidebar_session_drop(command, cx);
            return;
        }
        let command = if target_kind == "space"
            && matches!(source.kind, "group" | "collection")
            // Bots and dots belong to no Space (gx-core `assemble`), so a Space tile takes no bot
            // and no dot.
            && !self
                .native_sidebar
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.bots_mode || snapshot.dots_mode)
        {
            json!({"type": "moveToSpace", "sourceKind": source.kind, "sourceId": source.id, "spaceId": target_id})
        } else if source.kind == "group" && matches!(target_kind, "collection" | "ungroup") {
            json!({"type": "moveToCollection", "sourceKind": source.kind, "sourceId": source.id, "collectionId": if target_kind == "collection" { Some(target_id) } else { None }})
        } else if source.kind == "collection" && matches!(target_kind, "group" | "collection") {
            json!({"type": "moveCollection", "sourceId": source.id, "targetKind": target_kind, "targetId": target_id, "position": position})
        } else if source.kind == target_kind {
            match target_kind {
                "group" => {
                    json!({"type": "moveGroup", "groupId": source.id, "targetGroupId": target_id, "position": position})
                }
                _ => return,
            }
        } else {
            self.native_sidebar.drop_command = None;
            cx.notify();
            return;
        };
        let command = if matches!(
            command["type"].as_str(),
            Some("moveGroup" | "moveCollection" | "moveToCollection")
        ) {
            self.resolve_native_sidebar_drop_landing(command, |app, command| {
                app.with_native_sidebar_project_drop_landing(command)
            })
        } else {
            Some(command)
        };
        if self.native_sidebar.drop_command != command {
            self.native_sidebar.drop_command = command;
            cx.notify();
        }
    }

    /// `land` for this target, or what it answered the last time the pointer was over it.
    fn resolve_native_sidebar_drop_landing(
        &mut self,
        command: Value,
        land: impl FnOnce(&mut Self, Value) -> Option<Value>,
    ) -> Option<Value> {
        if let Some((target, resolved)) = &self.native_sidebar.drop_memo {
            if *target == command {
                return resolved.clone();
            }
        }
        let resolved = land(self, command.clone());
        self.native_sidebar.drop_memo = Some((command, resolved.clone()));
        resolved
    }

    /// A project or collection drop with the place its row lands, from the same plan the drop
    /// performs (gx-core `sidebar_drag/project_drop.rs`); `None` for a drop that does nothing.
    fn with_native_sidebar_project_drop_landing(&mut self, command: Value) -> Option<Value> {
        let (mut command, landing) = self.gx_store_preview_project_drop(&command)?;
        if landing.unchanged {
            return None;
        }
        let row = |row: Option<ghostex_gx_core::ProjectDropRow>| {
            row.map(|row| json!({"kind": row.kind, "id": row.id}))
        };
        command["landing"] = json!({
            "collectionId": landing.collection_id,
            "after": row(landing.after),
            "before": row(landing.before),
        });
        Some(command)
    }

    /// Where a project or collection drop's line is drawn on this row's block: above the row the
    /// dropped one lands right before, or below the one it lands right after when it becomes the
    /// last among its siblings.
    pub(crate) fn native_sidebar_project_drop_line(&self, kind: &str, id: &str) -> Option<&'static str> {
        let landing = self.native_sidebar.drop_command.as_ref()?.get("landing")?;
        let names = |row: &Value| row["kind"] == kind && row["id"] == id;
        if names(&landing["before"]) {
            return Some("before");
        }
        (landing["before"].is_null() && names(&landing["after"])).then_some("after")
    }

    /// A session drop and the line it draws, both from the store's one plan (gx-core
    /// `sidebar_drag/session_drop.rs`): the line goes where the row will land, and a drop that is
    /// refused or moves nothing keeps no command, so it draws nothing and releasing does nothing.
    fn set_native_sidebar_session_drop(&mut self, command: Option<Value>, cx: &mut Context<Self>) {
        let command = command.and_then(|command| {
            self.resolve_native_sidebar_drop_landing(command, |app, command| {
                app.with_native_sidebar_session_drop_landing(command)
            })
        });
        if self.native_sidebar.drop_command != command {
            self.native_sidebar.drop_command = command;
            cx.notify();
        }
    }

    fn with_native_sidebar_session_drop_landing(&mut self, mut command: Value) -> Option<Value> {
        let stale = |session_id: &str| {
            self.native_sidebar.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.groups.iter().any(|group| {
                    group.is_stale
                        && group
                            .sessions
                            .iter()
                            .any(|session| session.session_id == session_id)
                })
            })
        };
        {
            let session_id = command["sessionId"].as_str()?;
            let target = command["targetSessionId"].as_str();
            if stale(session_id) || target.is_some_and(stale) {
                return None;
            }
            // The planned command, which may aim at a coordinator instead of the hovered thread,
            // is the one the drop performs.
            let drop = self.gx_store_plan_sidebar_session_drop(&command)?;
            let landing = drop.landing?;
            if landing.unchanged {
                return None;
            }
            command = drop.command;
            command["landing"] = json!({
                "groupId": landing.group_id,
                "section": landing.section,
                "afterSessionId": landing.after_session_id,
                "beforeSessionId": landing.before_session_id,
            });
            Some(command)
        }
    }

    /// Where a session drop's line is drawn on this row: above the row the dropped one lands right
    /// before, or below the one it lands right after when it becomes the last of its section.
    pub(crate) fn native_sidebar_session_drop_line(&self, session_id: &str) -> Option<&'static str> {
        let landing = self.native_sidebar.drop_command.as_ref()?.get("landing")?;
        let before = landing["beforeSessionId"].as_str();
        if before == Some(session_id) {
            return Some("before");
        }
        (before.is_none() && landing["afterSessionId"].as_str() == Some(session_id))
            .then_some("after")
    }

    /// CDXC:Sidebar 2026-09-22 WHY:
    /// The drop line is drawn in the gap between two rows, which is where a user aiming "between" them releases, and a release there is over no row, so no row's drop ran and the drag moved nothing while the line was still showing. The sidebar itself takes those releases and performs the drop the line shows; a row under the pointer still answers first. The line goes away once the pointer leaves the sidebar, so a release elsewhere drops nothing.
    pub(crate) fn clear_native_sidebar_drop_outside(
        &mut self,
        event: &DragMoveEvent<SidebarDrag>,
        cx: &mut Context<Self>,
    ) {
        if !event.bounds.contains(&event.event.position)
            && self.native_sidebar.drop_command.take().is_some()
        {
            cx.notify();
        }
    }

    pub(crate) fn finish_native_sidebar_drop(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self.native_sidebar.drop_command.take() {
            if command["type"] == "moveSessionToSection" {
                self.move_native_sidebar_session_to_section(&command, cx);
            } else {
                self.dispatch_native_sidebar_ui(command, cx);
            }
        }
        // A row drag that crossed an Agents pane hid the pane surfaces for its drop zones; a drop
        // back in the sidebar is the release the window root never sees.
        self.finish_workspace_tab_drag(cx);
        cx.notify();
    }

    pub(crate) fn native_sidebar_drop_position(&self, key: &str, id: &str) -> Option<&str> {
        let command = self.native_sidebar.drop_command.as_ref()?;
        (command.get(key)?.as_str()? == id)
            .then(|| command.get("position").and_then(Value::as_str))
            .flatten()
    }
}

pub(super) fn drop_line(position: &str, scale: f32) -> AnyElement {
    let mut line = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.0 * scale))
        .bg(gpui::rgb(0x60a5fa));
    if position == "after" {
        line = line.bottom(px(-3.0 * scale));
    } else {
        line = line.top(px(-3.0 * scale));
    }
    line.into_any_element()
}

pub(super) trait SidebarDropTarget:
    ParentElement + InteractiveElement + Styled + Sized
{
    fn sidebar_drop_target(
        self,
        kind: &'static str,
        id: String,
        group: Option<String>,
        cx: &mut Context<GhostexGpuiApp>,
    ) -> Self {
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
        let painted_bounds = bounds.clone();
        let move_id = id.clone();
        let move_group = group.clone();
        self.relative()
            .child(
                gpui::canvas(
                    move |bounds, _, _| painted_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_drag_move::<SidebarDrag>(cx.listener(move |app, event, _, cx| {
                app.update_native_sidebar_drop(event, kind, &move_id, move_group.as_deref(), cx);
            }))
            .on_drop::<SidebarDrag>(cx.listener(move |app, source, window, cx| {
                // Resolve again at release: a fast drag can cross its start threshold on the final move before mouse-up.
                app.native_sidebar.drop_command = None;
                app.native_sidebar.drop_memo = None;
                app.resolve_native_sidebar_drop(
                    source,
                    window.mouse_position(),
                    bounds.get(),
                    kind,
                    &id,
                    group.as_deref(),
                    cx,
                );
                app.finish_native_sidebar_drop(cx);
            }))
    }
}
impl<T: ParentElement + InteractiveElement + Styled> SidebarDropTarget for T {}
