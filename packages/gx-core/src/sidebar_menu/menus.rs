//! The facade a host builds a drawn list's menus through.
//!
//! Holds no state of its own: the tag catalog, the Spaces and the collections are derived from the
//! store when it is made, and every menu is built from the view it was made for, so a menu can
//! never be older than the row it belongs to.

use std::collections::BTreeSet;

use crate::core::Core;
use crate::keys::{MachineId, CHATS_GROUP_ID};
use crate::sidebar_view::collections::CollectionsState;
use crate::sidebar_view::spaces::SpacesState;
use crate::sidebar_view::tags::TagCatalog;
use crate::sidebar_view::{
    CollectionView, GroupView, SessionRow, SessionView, SidebarInputs, SidebarView,
    LOCAL_MACHINE_ID,
};

use super::group::MenuGroup;
use super::host::MenuHost;
use super::hover::HoverAction;
use super::item::MenuItem;
use super::session::SessionActions;
use super::{bulk, collection, header, navigation, project, session};

/// Builds the menus of one drawn list. Holds no state of its own: everything is derived from the
/// store and the list it was made for, so a menu can never be older than the row it belongs to.
pub struct SidebarMenus<'a> {
    view: &'a SidebarView,
    inputs: &'a SidebarInputs,
    host: &'a MenuHost,
    /// The catalog of the machine whose rows are drawn: what a ROW's own tag submenu offers, which
    /// is the `customTags` the TypeScript hands `createNativeSessionActions`.
    catalog: TagCatalog,
    /// Every machine's catalog, this computer's first: what a tag id RESOLVES to for a label or an
    /// icon (`getSessionTagCatalogs`).
    label_catalog: TagCatalog,
    /// This computer's alone: which tag filters the Sort & Filter menu offers at all
    /// (`normalizeSidebarSessionTagListItems(settings, state.customSessionTags)`).
    filter_catalog: TagCatalog,
    spaces: Option<SpacesState>,
    collections: CollectionsState,
    /// The drawn machine's daemon id (`S…`), the first part of a session's global ref; absent
    /// until a stream frame has named it.
    server_id: Option<String>,
    now_ms: u64,
}

impl<'a> SidebarMenus<'a> {
    pub fn new(
        core: &Core,
        view: &'a SidebarView,
        inputs: &'a SidebarInputs,
        host: &'a MenuHost,
        now_ms: u64,
    ) -> Self {
        let machine = if view.selected_machine_id == LOCAL_MACHINE_ID {
            MachineId::Local
        } else {
            MachineId::Remote(view.selected_machine_id.clone())
        };
        let store = core.presentation();
        let side_state = store.machine(&machine).map(|entry| entry.side_state());
        Self {
            view,
            inputs,
            host,
            catalog: TagCatalog::from_state(
                side_state.and_then(|side| side.custom_session_tags.as_ref()),
            ),
            label_catalog: TagCatalog::merged(
                std::iter::once(MachineId::Local)
                    .chain(
                        store
                            .machines()
                            .map(|(machine, _)| machine.clone())
                            .filter(|machine| !machine.is_local()),
                    )
                    .map(|machine| {
                        store
                            .machine(&machine)
                            .and_then(|entry| entry.side_state().custom_session_tags.as_ref())
                    })
                    .collect::<Vec<_>>(),
            ),
            filter_catalog: TagCatalog::from_state(
                store
                    .machine(&MachineId::Local)
                    .and_then(|entry| entry.side_state().custom_session_tags.as_ref()),
            ),
            // The Spaces submenu follows the same switch the drawn list does: with Spaces off the
            // sidebar has none to offer.
            spaces: inputs
                .settings
                .sidebar_spaces_enabled
                .then(|| side_state.and_then(|side| side.spaces.as_ref()))
                .flatten()
                .map(SpacesState::from_wire),
            collections: match side_state.and_then(|side| side.project_collections.as_ref()) {
                Some(state) => CollectionsState::from_wire(state),
                None => inputs
                    .host
                    .stored_project_collections
                    .as_ref()
                    .map(CollectionsState::from_local_json)
                    .unwrap_or_default(),
            },
            server_id: store
                .loaded(&machine)
                .map(|loaded| loaded.server_id.clone())
                .filter(|server_id| !server_id.is_empty()),
            now_ms,
        }
    }

    /// One number over everything a menu reads from the store besides the row or group it is
    /// built for: the custom tag catalog, the Spaces and the collections. A host that caches a
    /// built menu compares this to know when the cached one went stale, because none of the three
    /// touches the `GroupCore` the menu belongs to.
    pub fn context_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.catalog.hash(&mut hasher);
        self.label_catalog.hash(&mut hasher);
        self.filter_catalog.hash(&mut hasher);
        self.spaces.hash(&mut hasher);
        self.collections.hash(&mut hasher);
        self.server_id.hash(&mut hasher);
        hasher.finish()
    }

    /// The group facts every menu of a group and of its rows reads.
    pub fn menu_group(&self, group: &'a GroupView) -> MenuGroup<'a> {
        let core = &group.core;
        MenuGroup {
            group_id: core.group_id.as_str(),
            storage_id: core.storage_id.as_str(),
            title: core.title.as_str(),
            is_remote: core.remote_machine.is_some(),
            remote_machine_name: core
                .remote_machine
                .as_ref()
                .map(|remote| remote.machine_name.as_str()),
            is_stale: core.is_stale,
            // A project group and a user-made group can take a session into a new group; a
            // machine's Chats collection cannot.
            //
            // CDXC:ContextMenus 2026-09-20 WHY:
            // Asked of the project row rather than of the group id, which is what
            // `createGpuiRemotePresentationSidebarGroups` does (`project !== undefined`). The id
            // test alone was written for this computer, where a project group always has a row;
            // a remote group whose project the machine did not publish is the case it was never
            // asked about, and `!is_chats_group` would offer the item on a group with nothing to
            // put in it.
            //
            // A bot's sessions stay under the bot: Bots lists them flat, with no groups (the bot
            // row's own menu has no New Group, see sidebar_menu/project.rs), so a group made from
            // one would leave Bots for Projects > Other. A dot's chats stay under the dot the same
            // way.
            can_create_session_group: core
                .project_context
                .as_ref()
                .is_some_and(|project| project.bot_profile.is_none() && project.dot_id.is_none())
                || is_user_made_group(core.group_id.as_str()),
            // The projection never sets it, so the Focus item never appears.
            can_focus_mode: false,
            project: core.project_context.as_ref(),
            // The projection marks every project group removable; only a remote machine's are not.
            can_remove_project: true,
            git_remote_origin_url: core
                .project_context
                .as_ref()
                .and_then(|project| project.git_remote_origin_url.as_deref()),
            server_id: self.server_id.clone(),
        }
    }

    /// The hover buttons and the placeholder menu a row publishes.
    pub fn row_actions(&self, group: &GroupView, session: &SessionView) -> SessionActions {
        let menu_group = self.menu_group(group);
        session::session_hover_actions(&self.session_input(&menu_group, &session.row, &[]))
    }

    /// The whole context menu of one row: the bulk menu when the row is part of a multi-selection,
    /// its own menu otherwise. `None` when the list no longer draws the row.
    pub fn row_menu(&self, sidebar_session_id: &str) -> Option<Vec<MenuItem>> {
        let (group, session) = self.find_row(sidebar_session_id)?;
        if self
            .inputs
            .ui
            .selected_session_ids
            .iter()
            .any(|selected| selected == sidebar_session_id)
        {
            if let Some(menu) = self.bulk_menu() {
                return Some(menu);
            }
        }
        let below = self.rows_below(group, &session.row);
        let below: Vec<&SessionRow> = below.iter().map(|session| &*session.row).collect();
        let menu_group = self.menu_group(group);
        Some(session::session_menu(&self.session_input(
            &menu_group,
            &session.row,
            &below,
        )))
    }

    /// The submenu behind one hover button of one row.
    pub fn row_hover_submenu(
        &self,
        sidebar_session_id: &str,
        action: HoverAction,
    ) -> Option<Vec<MenuItem>> {
        let (group, session) = self.find_row(sidebar_session_id)?;
        let menu_group = self.menu_group(group);
        Some(session::session_hover_submenu(
            &self.session_input(&menu_group, &session.row, &[]),
            action,
        ))
    }

    /// The menu a multi-selection carries, or `None` below two selected rows.
    pub fn bulk_menu(&self) -> Option<Vec<MenuItem>> {
        let selected: Vec<&SessionRow> = self
            .inputs
            .ui
            .selected_session_ids
            .iter()
            .filter_map(|session_id| self.find_row(session_id).map(|(_, session)| &*session.row))
            .collect();
        bulk::bulk_menu(&bulk::BulkMenuInput {
            selected: &selected,
            settings: &self.inputs.settings,
            catalog: &self.catalog,
        })
    }

    /// A project header's context menu.
    pub fn project_menu(&self, group: &GroupView) -> Vec<MenuItem> {
        let menu_group = self.menu_group(group);
        project::project_menu(&project::ProjectMenuInput {
            group: &menu_group,
            sessions: &group.core.sessions,
            collection_id: group.collection_id.as_deref(),
            collections: &self.collections,
            spaces: self.spaces.as_ref(),
            hidden_group: self
                .inputs
                .ui
                .hidden_items
                .group_ids
                .iter()
                .any(|group_id| *group_id == group.core.group_id),
            open_targets: &self.host.open_targets,
        })
    }

    /// A project header's buttons.
    pub fn header_actions(&self, group: &GroupView) -> Vec<MenuItem> {
        let menu_group = self.menu_group(group);
        header::project_header_actions(&menu_group, &self.inputs.settings, self.host)
    }

    /// A collection's context menu.
    pub fn collection_menu(&self, collection: &CollectionView) -> Vec<MenuItem> {
        let sessions: Vec<&SessionView> = collection
            .group_ids
            .iter()
            .filter_map(|group_id| self.view.group(group_id))
            .flat_map(|group| group.core.sessions.iter())
            .collect();
        collection::collection_menu(&collection::CollectionMenuInput {
            collection_id: collection.collection_id.as_str(),
            color: collection.color.as_str(),
            sessions: &sessions,
            hidden: self
                .inputs
                .ui
                .hidden_items
                .collection_keys
                .iter()
                .any(|key| *key == collection.storage_id),
            tag_list_items: &self.inputs.settings.tag_list_items,
            catalog: &self.catalog,
            spaces: self.spaces.as_ref(),
        })
    }

    /// The sidebar's own "more" menu.
    pub fn more_menu(&self) -> Vec<MenuItem> {
        let drawn: Vec<String> = self
            .view
            .groups
            .iter()
            .filter(|group| !is_chats_group(group.core.group_id.as_str()))
            .map(|group| group.core.group_id.clone())
            .collect();
        navigation::more_menu(&navigation::MoreMenuInput {
            ui: &self.inputs.ui,
            settings: &self.inputs.settings,
            catalog: &self.label_catalog,
            filter_catalog: &self.filter_catalog,
            host: self.host,
            drawn_project_group_ids: &drawn,
        })
    }

    fn session_input<'b>(
        &'b self,
        group: &'b MenuGroup<'b>,
        row: &'b SessionRow,
        below: &'b [&'b SessionRow],
    ) -> session::SessionMenuInput<'b> {
        session::SessionMenuInput {
            row,
            group,
            settings: &self.inputs.settings,
            catalog: &self.catalog,
            below,
            now_ms: self.now_ms,
        }
    }

    fn find_row(&self, sidebar_session_id: &str) -> Option<(&'a GroupView, &'a SessionView)> {
        self.view.groups.iter().find_map(|group| {
            group
                .core
                .sessions
                .iter()
                .find(|session| session.row.sidebar_session_id == sidebar_session_id)
                .map(|session| (group, session))
        })
    }

    /// The rows drawn under this one in its group, which is what Sleep Below and Close Below
    /// target. A row inside a collapsed section is not drawn and is not below anything.
    fn rows_below(&self, group: &'a GroupView, row: &SessionRow) -> Vec<&'a SessionView> {
        let visible: BTreeSet<&str> = group
            .core
            .sections
            .iter()
            .filter(|section| !section.collapsed)
            .flat_map(|section| section.session_ids.iter().map(String::as_str))
            .collect();
        let drawn: Vec<&SessionView> = group
            .core
            .sessions
            .iter()
            .filter(|session| visible.contains(session.row.sidebar_session_id.as_str()))
            .collect();
        match drawn
            .iter()
            .position(|session| session.row.sidebar_session_id == row.sidebar_session_id)
        {
            Some(index) => drawn[index + 1..].to_vec(),
            None => Vec::new(),
        }
    }
}

/// Whether a sidebar group id is a machine's Chats collection. A remote machine's is
/// `remote:<machine>:group:combined-chats`, so the id is not comparable as one string.
///
/// CDXC:ContextMenus 2026-09-20 WHY:
/// Written as two string tests rather than through `ProjectKey::parse_sidebar_group_id`, which is
/// the obvious way and is what this was first. `menu_group` runs once per ROW, not once per group,
/// so parsing meant percent-decoding a project path out of every local group id for every row of
/// every install: one allocation and one decode per row for a question that is answered by a
/// prefix and a suffix. The two forms are the only ones the encoder produces.
/// Whether a sidebar group id is a user-made session group, which has no project row of its own
/// and can still take a session into a new group.
fn is_user_made_group(group_id: &str) -> bool {
    group_id.starts_with("gpui-wsg:")
}

fn is_chats_group(group_id: &str) -> bool {
    const REMOTE_PREFIX: &str = "remote:";
    const REMOTE_CHATS_SUFFIX: &str = ":group:combined-chats";
    group_id == CHATS_GROUP_ID
        || (group_id.starts_with(REMOTE_PREFIX) && group_id.ends_with(REMOTE_CHATS_SUFFIX))
}
