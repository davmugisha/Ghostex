//! The buttons on a project header row, and the agent launcher behind the last one.
//!
//! Ported from the TypeScript sidebar page's project actions and agent launcher (frozen in the
//! deleted `tooling/gx-core/sidebar-page-frozen/project-actions.ts` and `agent-launcher.ts`; see
//! git history).

use crate::sidebar_accounts::AccountsState;
use crate::sidebar_view::SidebarSettings;

use super::agent_logos::colored_agent_logo;
use super::commands::{message, MenuCommand};
use super::group::MenuGroup;
use super::host::MenuHost;
use super::item::{MenuItem, MenuSplit};
use super::text::transcript_agent;

/// `createNativeProjectHeaderActions`.
///
/// CDXC:AgentLauncher 2026-09-18 DECISION:
/// User: remove the gap between the last-used agent button and the Select agent button in the
/// project header. Both halves render as the one split button the React header shows.
///
/// CDXC:Bots 2026-09-27 DECISION:
/// User: a bot row shows its pinned Actions, Edit SOUL, Edit config and one "+", because a bot is not a repo: no worktree, PR, history, browser or terminal button, and no agent split with its picker.
/// Edit SOUL and Edit config always open that bot's own `SOUL.md` and `config.yaml`, in Ghostex's built-in Code view under the bot.
/// They are fixed buttons on every bot row on this computer, not editable or deletable Actions; a remote computer's bot has none, because its file would open on this computer.
/// Supersedes the 2026-09-26 decision (pinned Actions and "+" only, with seeded `code <file>` Actions).
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// A dot row is not a repo either, so it has exactly the dot's own buttons: Setup and Computer (the OpenDots view on that dot's tab) and one "+" that starts a new chat with the dot's own agent. No git, history, browser, terminal or pinned Actions, and no agent split. A remote computer's dot has only the "+", because the OpenDots view shows this computer's dots.
pub fn project_header_actions(
    group: &MenuGroup<'_>,
    settings: &SidebarSettings,
    host: &MenuHost,
) -> Vec<MenuItem> {
    let group_id = group.group_id;
    let Some(project) = group.project else {
        return vec![MenuItem::row(
            "Create a Terminal",
            "plus",
            MenuCommand::command(message::create_session_in_group(group_id)),
        )];
    };
    if let Some(dot_id) = project.dot_id.as_deref() {
        let mut actions = Vec::new();
        if !group.is_remote {
            for (label, icon, tab) in [
                ("Setup", "settings", "setup"),
                ("Computer", "device-desktop", "computer"),
            ] {
                actions.push(MenuItem::row(
                    label,
                    icon,
                    MenuCommand::command(message::open_dots_view_on_dot(dot_id, tab)),
                ));
            }
        }
        actions.push(MenuItem::row(
            &format!("New chat with {}", group.title),
            "plus",
            MenuCommand::project_action(group_id, "dot", None),
        ));
        return actions;
    }
    let mut pinned = Vec::new();
    let project_commands = host
        .project_commands
        .get(project.project_id.as_str())
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (scope, commands) in [
        ("global", host.global_commands.as_slice()),
        ("project", project_commands),
    ] {
        for command in commands
            .iter()
            .filter(|command| command.show_on_project_row)
        {
            let label = command.name.trim();
            pinned.push(MenuItem::row(
                if label.is_empty() {
                    "Run Action"
                } else {
                    label
                },
                command.icon.as_deref().unwrap_or("bolt"),
                MenuCommand::command(message::run_sidebar_command(
                    &command.command_id,
                    scope,
                    group_id,
                )),
            ));
        }
    }
    if project.bot_profile.is_some() {
        let edit_files: &[_] = if group.is_remote {
            &[]
        } else {
            &[
                ("Edit SOUL", "brain", "SOUL.md"),
                ("Edit config", "settings", "config.yaml"),
            ]
        };
        for &(label, icon, file) in edit_files {
            let file_path = std::path::Path::new(&project.path).join(file);
            pinned.push(MenuItem::row(
                label,
                icon,
                MenuCommand::command(message::open_bot_file(
                    group_id,
                    &project.path,
                    &file_path.to_string_lossy(),
                )),
            ));
        }
        pinned.push(MenuItem::row(
            &format!("New {} session", group.title),
            "plus",
            MenuCommand::project_action(group_id, "bot", None),
        ));
        return pinned;
    }
    let mut actions = vec![
        if project.worktree.is_some() {
            MenuItem::row(
                "Create PR",
                "git-pull-request",
                MenuCommand::command(message::run_sidebar_git_action(group_id, "pr")),
            )
        } else {
            MenuItem::row(
                "Add Worktree",
                "git-branch",
                MenuCommand::project_action(group_id, "worktree", None),
            )
        },
        MenuItem::row(
            "History",
            "history",
            MenuCommand::project_action(group_id, "history", None),
        ),
    ];
    if !settings.browser_view_tab_hidden {
        actions.push(
            MenuItem::row(
                "New Browser Tab",
                "world",
                MenuCommand::command(message::open_browser_pane_in_group(group_id)),
            )
            .with_hotkey("openBrowserPane"),
        );
    }
    actions.push(
        MenuItem::row(
            "Create Terminal",
            "terminal-2",
            MenuCommand::command(message::create_project_terminal(group_id)),
        )
        .with_hotkey("createSession"),
    );
    actions.append(&mut pinned);
    let primary = host.primary_agent();
    let primary_icon = primary.and_then(|agent| agent.icon.as_deref());
    actions.push(MenuItem {
        label: Some(format!(
            "Create {}",
            primary.map_or("Agent", |agent| agent.name.as_str())
        )),
        icon: Some("sparkles".to_string()),
        agent_icon: primary_icon.map(str::to_string),
        image_data_url: primary_icon
            .and_then(colored_agent_logo)
            .map(str::to_string),
        command: Some(MenuCommand::project_action(
            group_id,
            "agent",
            primary.map(|agent| agent.agent_id.as_str()),
        )),
        split: Some(MenuSplit::Start),
        hotkey: Some("createAgentSession".to_string()),
        ..MenuItem::default()
    });
    actions.push(MenuItem {
        label: Some("Select Agent".to_string()),
        icon: Some("chevron-down".to_string()),
        children: Some(agent_launcher_items(group_id, host)),
        split: Some(MenuSplit::End),
        ..MenuItem::default()
    });
    actions
}

/// `nativeAgentLauncherItems(groupId)`, the state before any account list has been read.
pub fn agent_launcher_items(group_id: &str, host: &MenuHost) -> Vec<MenuItem> {
    agent_launcher_items_with_accounts(group_id, host, None)
}

/// `nativeAgentLauncherItems(groupId, data)`: with an account list read, each account button
/// carries how many registered accounts its provider has. The pages behind the buttons are
/// `sidebar_accounts/`.
pub fn agent_launcher_items_with_accounts(
    group_id: &str,
    host: &MenuHost,
    accounts: Option<&AccountsState>,
) -> Vec<MenuItem> {
    let primary_id = host.primary_agent().map(|agent| agent.agent_id.as_str());
    let mut items: Vec<MenuItem> = host
        .agents
        .iter()
        .map(|agent| {
            let icon = agent.icon.as_deref();
            MenuItem {
                primary: Some(agent.agent_id.as_str()) == primary_id,
                supports_chat: transcript_agent(Some(&agent.agent_id), icon).is_some(),
                label: Some(agent.name.clone()),
                agent_icon: icon.map(str::to_string),
                image_data_url: icon.and_then(colored_agent_logo).map(str::to_string),
                icon: icon.is_none().then(|| "code".to_string()),
                keep_open: true,
                command: Some(MenuCommand::agent_accounts(
                    group_id,
                    "launch",
                    Some(&agent.agent_id),
                )),
                secondary: account_provider(agent.agent_id.as_str(), icon).map(|provider| {
                    super::item::MenuSecondary {
                        icon: "user".to_string(),
                        // The count arrives with the account list; until then the pill is blank.
                        label: accounts
                            .map(|accounts| {
                                accounts.registered_for(Some(provider)).count().to_string()
                            })
                            .unwrap_or_default(),
                        command: MenuCommand::agent_accounts(
                            group_id,
                            "accounts",
                            Some(&agent.agent_id),
                        ),
                    }
                }),
                ..MenuItem::default()
            }
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.extend(super::run_in_box::run_in_box_row(group_id, host));
    // CDXC:Coordinators 2026-09-30 WHY: a coordinator is started where agents are started, so the launcher that lists them offers it too; the dialog behind it names the coordinator and picks its Claude or Codex agent.
    items.push(MenuItem::row(
        "New Coordinator…",
        "users-group",
        MenuCommand::project_action(group_id, "coordinator", None),
    ));
    items.push(MenuItem::row(
        "Configure",
        "settings",
        MenuCommand::project_action(group_id, "agent", None),
    ));
    if let Some(first) = items.first_mut() {
        first.agent_launcher = true;
        first.menu_owner = Some(format!("group:{group_id}"));
        first.on_open = Some(MenuCommand::agent_accounts(group_id, "load", None));
    }
    items
}

/// `providerFor`: only Claude and Codex have an account switcher.
pub(crate) fn account_provider(agent_id: &str, icon: Option<&str>) -> Option<&'static str> {
    match icon.unwrap_or(agent_id) {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}
