//! Where a dragged project or collection lands: the drawn row sequence, built once more by the
//! list's own collection rule with the move's writes applied.
//!
//! CDXC:Sidebar 2026-10-01 WHY:
//! The drop line of a project drag was drawn on the hovered header, so it disagreed with the move
//! wherever the rules put the row somewhere else: below an expanded project it sat between the
//! header and that project's sessions although the row lands after the whole block, after a parent
//! it ignored the worktrees the row is re-nested past, and a drop the worktree rules refuse still
//! drew a line. The line now comes from the same writes the drop performs.
//!
//! SEE-ALSO: packages/gx-core/src/sidebar_drag/project_move.rs (the writes),
//! apps/desktop/src/app/native_sidebar/drag.rs (the line).

use std::collections::BTreeMap;

use super::collections::{project_sidebar_collections, CollectionItem, CollectionsState};
use super::view::SidebarView;

/// A drawn row of the project list: a project or user-made group, or a collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDropRow {
    /// `group` or `collection`.
    pub kind: &'static str,
    pub id: String,
}

/// Where the moved row is drawn after the drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDropLanding {
    /// The collection the row lands in; `None` at the top level.
    pub collection_id: Option<String>,
    /// The drawn row right above the landing slot among the row's new siblings, and the one right
    /// below it.
    pub after: Option<ProjectDropRow>,
    pub before: Option<ProjectDropRow>,
    /// The drop leaves every drawn row where it is.
    pub unchanged: bool,
}

impl SidebarView {
    /// The landing of the dragged `moved_kind` (`group` or `collection`) row once the move has
    /// written `group_order` (the new project order, when the move writes one) and `collections`
    /// (the collections document after the move). `collections_before` is the document the list
    /// was drawn from. `None` when the row is not drawn.
    pub fn preview_project_drop(
        &self,
        collections_before: &CollectionsState,
        group_order: Option<&[String]>,
        collections: Option<&CollectionsState>,
        moved_kind: &str,
        moved_id: &str,
    ) -> Option<ProjectDropLanding> {
        let drawn: Vec<String> = self
            .groups
            .iter()
            .map(|group| group.core.group_id.clone())
            .collect();
        let mut next = drawn.clone();
        if let Some(order) = group_order {
            let rank: BTreeMap<&str, usize> = order
                .iter()
                .enumerate()
                .map(|(index, id)| (id.as_str(), index))
                .collect();
            // Stable, so a group the order does not name keeps its place among the others.
            next.sort_by_key(|id| rank.get(id.as_str()).copied().unwrap_or(usize::MAX));
        }
        let before = self.project_rows(&drawn, collections_before);
        let after = self.project_rows(&next, collections.unwrap_or(collections_before));
        let unchanged = before == after;
        let row_of = |item: &CollectionItem| match item {
            CollectionItem::Project { group_id } => ProjectDropRow {
                kind: "group",
                id: group_id.clone(),
            },
            CollectionItem::Collection { collection, .. } => ProjectDropRow {
                kind: "collection",
                id: collection.collection_id.clone(),
            },
        };
        let is_moved = |row: &ProjectDropRow| row.kind == moved_kind && row.id == moved_id;
        let drawn_before = flatten(&before, &row_of);
        let neighbours = |rows: Vec<ProjectDropRow>, collection_id: Option<String>| {
            let index = rows.iter().position(is_moved)?;
            // A project's worktrees move with it, so the row below the landing slot is the first
            // one after the rows that travel along.
            let carried = carried_rows(&drawn_before, &rows[index..], &is_moved);
            Some(ProjectDropLanding {
                collection_id,
                after: index.checked_sub(1).map(|previous| rows[previous].clone()),
                before: rows.get(index + 1 + carried).cloned(),
                unchanged,
            })
        };
        if let Some(landing) = neighbours(after.iter().map(row_of).collect(), None) {
            return Some(landing);
        }
        after.iter().find_map(|item| match item {
            CollectionItem::Collection {
                collection,
                group_ids,
            } if moved_kind == "group" => neighbours(
                group_ids
                    .iter()
                    .map(|id| ProjectDropRow {
                        kind: "group",
                        id: id.clone(),
                    })
                    .collect(),
                Some(collection.collection_id.clone()),
            ),
            _ => None,
        })
    }

    /// The top-level rows `assemble` draws for this group order and collections document.
    fn project_rows(
        &self,
        group_ids: &[String],
        collections: &CollectionsState,
    ) -> Vec<CollectionItem> {
        if self.bots_mode || self.dots_mode {
            return group_ids
                .iter()
                .map(|group_id| CollectionItem::Project {
                    group_id: group_id.clone(),
                })
                .collect();
        }
        let project = |group_id: &str| {
            self.group(group_id)
                .and_then(|group| group.core.project_context.as_ref())
        };
        let project_of_group =
            |group_id: &str| project(group_id).map(|project| project.project_id.clone());
        let parent_project_of_group = |group_id: &str| {
            project(group_id)
                .and_then(|project| project.worktree.as_ref())
                .map(|worktree| worktree.parent_project_id.clone())
        };
        project_sidebar_collections(
            group_ids,
            collections,
            &project_of_group,
            &parent_project_of_group,
        )
    }
}

/// Every drawn row in order: each top-level row, a collection followed by its projects.
fn flatten(
    items: &[CollectionItem],
    row_of: &dyn Fn(&CollectionItem) -> ProjectDropRow,
) -> Vec<ProjectDropRow> {
    let mut rows = Vec::new();
    for item in items {
        rows.push(row_of(item));
        if let CollectionItem::Collection { group_ids, .. } = item {
            rows.extend(group_ids.iter().map(|id| ProjectDropRow {
                kind: "group",
                id: id.clone(),
            }));
        }
    }
    rows
}

/// How many rows right after the moved one (`from_moved` starts at it) are the rows that already
/// followed it before the drop: the rows that travel with it.
pub(super) fn carried_rows<T: PartialEq>(
    drawn_before: &[T],
    from_moved: &[T],
    is_moved: &dyn Fn(&T) -> bool,
) -> usize {
    let Some(start) = drawn_before.iter().position(is_moved) else {
        return 0;
    };
    drawn_before[start + 1..]
        .iter()
        .zip(from_moved.iter().skip(1))
        .take_while(|(before, after)| before == after)
        .count()
}
