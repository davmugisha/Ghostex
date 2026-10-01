//! A dot's "+" (a new chat with the dot), its Setup and Computer buttons, and its Pause or Resume.
//!
//! The launch is the ordinary agent launch into the dot's project with the dot's own agent:
//! gxserver adds the dot's MCP server, instructions and model to every session made in a dot
//! project (`apply_dot_launch_flags` in server/src/agents/launch_plan.rs). Like a bot's "+", it
//! skips only what belongs to the agent launcher: the launcher's highlighted agent is left alone.
//!
//! SEE-ALSO: packages/gx-core/src/sidebar_menu/header.rs (the buttons),
//! apps/desktop/src/app/native_sidebar/dots_host.rs (where the OpenDots view opens).

use ghostex_gx_core::{MachineId, ProjectKey};
use serde_json::{Value, json};

use super::super::gx_rpc;
use crate::GhostexGpuiApp;

/// The agent a dot starts with when its row names none (OpenDots' default, CONTRACT.md).
const DEFAULT_DOT_AGENT_ID: &str = "claude";

impl GhostexGpuiApp {
    /// The dot messages a menu row or a header button sends through either sidebar door. Returns
    /// whether it was one.
    pub(super) fn gx_store_answer_dot_message(
        &mut self,
        message: &Value,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        match message.get("type").and_then(Value::as_str) {
            Some("openDotsView") => self.gx_store_open_dots_view_message(message, cx),
            Some("setDotPaused") => self.gx_store_set_dot_paused_message(message, cx),
            _ => return false,
        }
        true
    }

    /// A `runSidebarDot` host message: a new chat with the dot whose project the group is.
    pub(super) fn gx_store_run_sidebar_dot_message(
        &mut self,
        message: &Value,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(group_id) = message.get("groupId").and_then(Value::as_str) else {
            return;
        };
        let agent_id = self
            .gx_store_dot_agent_id(group_id)
            .unwrap_or_else(|| DEFAULT_DOT_AGENT_ID.to_string());
        self.gx_store_request_agent_launch(&agent_id, Some(group_id), None, cx);
    }

    /// The dot's own agent, off the project row the list draws (`dotAgentId`).
    fn gx_store_dot_agent_id(&self, group_id: &str) -> Option<String> {
        let key = ProjectKey::parse_sidebar_group_id(group_id)?;
        let project = self
            .gx_store
            .core
            .presentation()
            .loaded(&key.machine)?
            .project(&key.project_id)?;
        project.dot_id.as_ref()?;
        project
            .dot_agent_id
            .as_deref()
            .map(str::trim)
            .filter(|agent_id| !agent_id.is_empty())
            .map(str::to_string)
    }

    /// The dot's name when this is a launch into a local dot project, so its new chat is named
    /// after the dot rather than after its agent; `None` for every other project.
    pub(super) fn gx_store_dot_name(&self, project_id: &str) -> Option<String> {
        let project = self
            .gx_store
            .core
            .presentation()
            .loaded(&MachineId::Local)?
            .project(project_id)?;
        project
            .dot_id
            .is_some()
            .then(|| project.title.trim().to_string())
            .filter(|name| !name.is_empty())
    }

    /// An `openDotsView` sidebar message (`{"page","dotId"?,"tab"?}`): the OpenDots view on that
    /// page. The sidebar's doors carry no window, so the open waits for the main window's turn.
    pub(super) fn gx_store_open_dots_view_message(
        &mut self,
        message: &Value,
        cx: &mut gpui::Context<Self>,
    ) {
        let message = message.clone();
        self.defer_in_main_window(cx, move |app, window, cx| {
            app.open_dots_view_from_sidebar(&message, window, cx);
        });
    }

    /// A `setDotPaused` sidebar message (a dot row's Pause or Resume). The row follows on its own:
    /// gxserver publishes the dot's project with `dotPaused` changed.
    pub(super) fn gx_store_set_dot_paused_message(
        &mut self,
        message: &Value,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(dot_id) = message
            .get("dotId")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let paused = message.get("paused").and_then(Value::as_bool) == Some(true);
        let method = if paused { "pauseDot" } else { "resumeDot" };
        cx.spawn(async move |this, cx| {
            let result = gx_rpc(
                None,
                "/api/openDotsRpc",
                json!({ "method": method, "params": { "dotId": dot_id } }),
            )
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |this, cx| {
                    this.gx_store_create_toast(
                        "warning",
                        if paused {
                            "Could not pause the dot"
                        } else {
                            "Could not resume the dot"
                        },
                        Some(&error.message),
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}
