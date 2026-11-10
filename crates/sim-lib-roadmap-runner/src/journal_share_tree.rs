// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use sim_kernel::ContentId;

use super::{
    ExecutionJournalError, Node, ShareIo, Store, empty_id, is_empty, node_datum, node_from,
};

impl Store<'_> {
    pub(super) fn build(
        &mut self,
        entries: &[(String, ContentId)],
    ) -> Result<ContentId, ExecutionJournalError> {
        fn rec(
            store: &mut Store<'_>,
            entries: &[(String, ContentId)],
        ) -> Result<Option<ContentId>, ExecutionJournalError> {
            if entries.is_empty() {
                return Ok(None);
            }
            let mid = entries.len() / 2;
            let left = rec(store, &entries[..mid])?;
            let right = rec(store, &entries[mid + 1..])?;
            Ok(Some(store.finish(
                entries[mid].0.clone(),
                entries[mid].1.clone(),
                left,
                right,
            )?))
        }
        Ok(rec(self, entries)?.unwrap_or_else(empty_id))
    }

    pub(super) fn insert(
        &mut self,
        root: Option<ContentId>,
        key: &str,
        value: &ContentId,
    ) -> Result<ContentId, ExecutionJournalError> {
        let Some(id) = root else {
            return self.finish(key.to_owned(), value.clone(), None, None);
        };
        let node = self.load(&id)?;
        match key.cmp(node.key.as_str()) {
            std::cmp::Ordering::Equal if &node.value == value => Ok(id),
            std::cmp::Ordering::Equal => {
                self.finish(node.key, value.clone(), node.left, node.right)
            }
            std::cmp::Ordering::Less => {
                let left = Some(self.insert(node.left, key, value)?);
                self.rebalance(node.key, node.value, left, node.right)
            }
            std::cmp::Ordering::Greater => {
                let right = Some(self.insert(node.right, key, value)?);
                self.rebalance(node.key, node.value, node.left, right)
            }
        }
    }

    pub(super) fn delete(
        &mut self,
        root: Option<ContentId>,
        key: &str,
    ) -> Result<Option<ContentId>, ExecutionJournalError> {
        let Some(id) = root else {
            return Ok(None);
        };
        let node = self.load(&id)?;
        match key.cmp(node.key.as_str()) {
            std::cmp::Ordering::Less => {
                let left = self.delete(node.left.clone(), key)?;
                if left == node.left {
                    return Ok(Some(id));
                }
                Ok(Some(
                    self.rebalance(node.key, node.value, left, node.right)?,
                ))
            }
            std::cmp::Ordering::Greater => {
                let right = self.delete(node.right.clone(), key)?;
                if right == node.right {
                    return Ok(Some(id));
                }
                Ok(Some(
                    self.rebalance(node.key, node.value, node.left, right)?,
                ))
            }
            std::cmp::Ordering::Equal => match (node.left, node.right) {
                (None, None) => Ok(None),
                (Some(child), None) | (None, Some(child)) => Ok(Some(child)),
                (Some(left), Some(right)) => {
                    let (succ_key, succ_value, new_right) = self.take_min(&right)?;
                    Ok(Some(self.rebalance(
                        succ_key,
                        succ_value,
                        Some(left),
                        new_right,
                    )?))
                }
            },
        }
    }

    fn take_min(
        &mut self,
        id: &ContentId,
    ) -> Result<(String, ContentId, Option<ContentId>), ExecutionJournalError> {
        let node = self.load(id)?;
        if let Some(left) = node.left {
            let (key, value, new_left) = self.take_min(&left)?;
            let merged = self.rebalance(node.key, node.value, new_left, node.right)?;
            Ok((key, value, Some(merged)))
        } else {
            Ok((node.key, node.value, node.right))
        }
    }

    fn rebalance(
        &mut self,
        key: String,
        value: ContentId,
        left: Option<ContentId>,
        right: Option<ContentId>,
    ) -> Result<ContentId, ExecutionJournalError> {
        let lh = self.height(left.as_ref());
        let rh = self.height(right.as_ref());
        if lh > rh.saturating_add(1) {
            let left_id = left
                .clone()
                .ok_or(ExecutionJournalError::Semantic("shared state"))?;
            let child = self.load(&left_id)?;
            let left = if self.height(child.right.as_ref()) > self.height(child.left.as_ref()) {
                Some(self.rotate_left(child)?)
            } else {
                Some(left_id)
            };
            let child = self.load(
                left.as_ref()
                    .ok_or(ExecutionJournalError::Semantic("shared state"))?,
            )?;
            return self.rotate_right(key, value, child, right);
        }
        if rh > lh.saturating_add(1) {
            let right_id = right
                .clone()
                .ok_or(ExecutionJournalError::Semantic("shared state"))?;
            let child = self.load(&right_id)?;
            let right = if self.height(child.left.as_ref()) > self.height(child.right.as_ref()) {
                Some(self.rotate_right_child(child)?)
            } else {
                Some(right_id)
            };
            let child = self.load(
                right
                    .as_ref()
                    .ok_or(ExecutionJournalError::Semantic("shared state"))?,
            )?;
            return self.rotate_left_root(key, value, left, child);
        }
        self.finish(key, value, left, right)
    }

    fn rotate_left(&mut self, node: Node) -> Result<ContentId, ExecutionJournalError> {
        let right = self.load(
            node.right
                .as_ref()
                .ok_or(ExecutionJournalError::Semantic("shared state"))?,
        )?;
        let new_left = self.finish(node.key, node.value, node.left, right.left.clone())?;
        self.finish(right.key, right.value, Some(new_left), right.right)
    }

    fn rotate_right(
        &mut self,
        key: String,
        value: ContentId,
        left: Node,
        right: Option<ContentId>,
    ) -> Result<ContentId, ExecutionJournalError> {
        let new_right = self.finish(key, value, left.right.clone(), right)?;
        self.finish(left.key, left.value, left.left, Some(new_right))
    }

    fn rotate_right_child(&mut self, node: Node) -> Result<ContentId, ExecutionJournalError> {
        let left = self.load(
            node.left
                .as_ref()
                .ok_or(ExecutionJournalError::Semantic("shared state"))?,
        )?;
        let new_right = self.finish(node.key, node.value, left.right.clone(), node.right)?;
        self.finish(left.key, left.value, left.left, Some(new_right))
    }

    fn rotate_left_root(
        &mut self,
        key: String,
        value: ContentId,
        left: Option<ContentId>,
        right: Node,
    ) -> Result<ContentId, ExecutionJournalError> {
        let new_left = self.finish(key, value, left, right.left.clone())?;
        self.finish(right.key, right.value, Some(new_left), right.right)
    }

    fn finish(
        &mut self,
        key: String,
        value: ContentId,
        left: Option<ContentId>,
        right: Option<ContentId>,
    ) -> Result<ContentId, ExecutionJournalError> {
        let height = 1 + self.height(left.as_ref()).max(self.height(right.as_ref()));
        let datum = node_datum(&key, &value, height, left.as_ref(), right.as_ref());
        let id = self.io.persist_share(datum.clone())?;
        self.fresh.insert(id.clone(), datum);
        Ok(id)
    }

    fn height(&mut self, id: Option<&ContentId>) -> u64 {
        id.and_then(|id| self.load(id).ok())
            .map(|node| node.height)
            .unwrap_or(0)
    }

    fn load(&mut self, id: &ContentId) -> Result<Node, ExecutionJournalError> {
        if !self.fresh.contains_key(id) {
            let datum = self.io.load_share(id)?;
            self.fresh.insert(id.clone(), datum);
        }
        node_from(
            self.fresh
                .get(id)
                .ok_or(ExecutionJournalError::MissingObject)?,
        )
    }
}

const DIFF_DEPTH: u32 = 64;

enum View {
    Empty,
    Shared(ContentId),
    Cons {
        key: String,
        value: ContentId,
        left: Box<View>,
        right: Box<View>,
    },
}

pub(super) fn evidence_additions(
    previous: &ContentId,
    next: &ContentId,
    io: &dyn ShareIo,
) -> Result<Vec<ContentId>, ExecutionJournalError> {
    let mut added = Vec::new();
    let old = view_of(previous);
    let new = view_of(next);
    additions(io, old, new, &mut added, 0, None, None)?;
    Ok(added)
}

/// Net fact edits between two roots. `None` is a deleted key. Equal roots load nothing.
pub(super) fn fact_changes(
    previous: &ContentId,
    next: &ContentId,
    io: &dyn ShareIo,
) -> Result<BTreeMap<String, Option<ContentId>>, ExecutionJournalError> {
    let mut changed = BTreeMap::new();
    let old = view_of(previous);
    let new = view_of(next);
    fact_diff(io, old, new, &mut changed, 0, None, None)?;
    Ok(changed)
}

fn fact_diff(
    io: &dyn ShareIo,
    old: View,
    new: View,
    changed: &mut BTreeMap<String, Option<ContentId>>,
    depth: u32,
    low: Option<&str>,
    high: Option<&str>,
) -> Result<(), ExecutionJournalError> {
    if depth > DIFF_DEPTH {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    if same(&old, &new) {
        return Ok(());
    }
    if blank(&new) {
        return fact_removed(io, old, changed, depth);
    }
    let (key, value, left, right) = explode(io, new)?;
    let key = key.as_str();
    if low.is_some_and(|bound| key <= bound) || high.is_some_and(|bound| key >= bound) {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    let (old_left, found, old_right) = split(io, key, old, depth)?;
    fact_diff(io, old_left, left, changed, depth + 1, low, Some(key))?;
    if found.as_ref() != Some(&value) {
        changed.insert(key.to_string(), Some(value));
    }
    fact_diff(io, old_right, right, changed, depth + 1, Some(key), high)
}

fn fact_removed(
    io: &dyn ShareIo,
    view: View,
    changed: &mut BTreeMap<String, Option<ContentId>>,
    depth: u32,
) -> Result<(), ExecutionJournalError> {
    if depth > DIFF_DEPTH {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    if blank(&view) {
        return Ok(());
    }
    let (key, _, left, right) = explode(io, view)?;
    fact_removed(io, left, changed, depth + 1)?;
    changed.insert(key, None);
    fact_removed(io, right, changed, depth + 1)
}

fn view_of(id: &ContentId) -> View {
    if is_empty(id) {
        View::Empty
    } else {
        View::Shared(id.clone())
    }
}

fn child_view(id: Option<ContentId>) -> View {
    match id {
        Some(id) => view_of(&id),
        None => View::Empty,
    }
}

fn same(left: &View, right: &View) -> bool {
    match (left, right) {
        (View::Empty, View::Empty) => true,
        (View::Shared(left), View::Shared(right)) => left == right,
        (View::Empty, View::Shared(id)) | (View::Shared(id), View::Empty) => is_empty(id),
        _ => false,
    }
}

fn blank(view: &View) -> bool {
    match view {
        View::Empty => true,
        View::Shared(id) => is_empty(id),
        View::Cons { .. } => false,
    }
}

fn additions(
    io: &dyn ShareIo,
    old: View,
    new: View,
    added: &mut Vec<ContentId>,
    depth: u32,
    low: Option<&str>,
    high: Option<&str>,
) -> Result<(), ExecutionJournalError> {
    if depth > DIFF_DEPTH {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    if same(&old, &new) {
        return Ok(());
    }
    if matches!(new, View::Empty) {
        return if blank(&old) {
            Ok(())
        } else {
            Err(ExecutionJournalError::Semantic(
                "evidence set dropped a retained member",
            ))
        };
    }
    let (key, value, left, right) = explode(io, new)?;
    let key = key.as_str();
    if key != super::id_key(&value)
        || low.is_some_and(|bound| key <= bound)
        || high.is_some_and(|bound| key >= bound)
    {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    let next = depth + 1;
    let (old_left, found, prior_r) = split(io, key, old, depth)?;
    additions(io, old_left, left, added, next, low, Some(key))?;
    match found {
        Some(existing) if existing != value => {
            return Err(ExecutionJournalError::Semantic("shared state"));
        }
        Some(_) => {}
        None => added.push(value),
    }
    additions(io, prior_r, right, added, next, Some(key), high)
}

fn explode(
    io: &dyn ShareIo,
    view: View,
) -> Result<(String, ContentId, View, View), ExecutionJournalError> {
    match view {
        View::Cons {
            key,
            value,
            left,
            right,
        } => Ok((key, value, *left, *right)),
        View::Shared(id) => {
            let node = node_from(&io.load_share(&id)?)?;
            Ok((
                node.key,
                node.value,
                child_view(node.left),
                child_view(node.right),
            ))
        }
        View::Empty => Err(ExecutionJournalError::Semantic("shared state")),
    }
}

fn split(
    io: &dyn ShareIo,
    key: &str,
    view: View,
    depth: u32,
) -> Result<(View, Option<ContentId>, View), ExecutionJournalError> {
    if depth > DIFF_DEPTH {
        return Err(ExecutionJournalError::Semantic("shared state"));
    }
    match view {
        View::Empty => Ok((View::Empty, None, View::Empty)),
        View::Shared(id) if is_empty(&id) => Ok((View::Empty, None, View::Empty)),
        View::Shared(id) => {
            let node = node_from(&io.load_share(&id)?)?;
            match key.cmp(node.key.as_str()) {
                std::cmp::Ordering::Equal => Ok((
                    child_view(node.left),
                    Some(node.value),
                    child_view(node.right),
                )),
                std::cmp::Ordering::Less => {
                    let (left, found, mid) = split(io, key, child_view(node.left), depth + 1)?;
                    Ok((
                        left,
                        found,
                        join(node.key, node.value, mid, child_view(node.right)),
                    ))
                }
                std::cmp::Ordering::Greater => {
                    let (mid, found, right) = split(io, key, child_view(node.right), depth + 1)?;
                    Ok((
                        join(node.key, node.value, child_view(node.left), mid),
                        found,
                        right,
                    ))
                }
            }
        }
        View::Cons {
            key: node_key,
            value,
            left,
            right,
        } => match key.cmp(node_key.as_str()) {
            std::cmp::Ordering::Equal => Ok((*left, Some(value), *right)),
            std::cmp::Ordering::Less => {
                let (left_tree, found, mid) = split(io, key, *left, depth + 1)?;
                Ok((left_tree, found, join(node_key, value, mid, *right)))
            }
            std::cmp::Ordering::Greater => {
                let (mid, found, right_tree) = split(io, key, *right, depth + 1)?;
                Ok((join(node_key, value, *left, mid), found, right_tree))
            }
        },
    }
}

fn join(key: String, value: ContentId, left: View, right: View) -> View {
    View::Cons {
        key,
        value,
        left: Box::new(left),
        right: Box::new(right),
    }
}
