// Copyright 2020 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use pollster::FutureExt as _;

use crate::backend;
use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::store::Store;
use crate::tree::Tree;

#[derive(Debug)]
enum Override {
    Tombstone,
    Replace(TreeValue),
}

#[derive(Debug)]
pub struct TreeBuilder {
    store: Arc<Store>,
    base_tree_id: TreeId,
    overrides: BTreeMap<RepoPathBuf, Override>,
}

impl TreeBuilder {
    pub fn new(store: Arc<Store>, base_tree_id: TreeId) -> Self {
        let overrides = BTreeMap::new();
        Self {
            store,
            base_tree_id,
            overrides,
        }
    }

    pub fn store(&self) -> &Store {
        self.store.as_ref()
    }

    pub fn set(&mut self, path: RepoPathBuf, value: TreeValue) {
        assert!(!path.is_root());
        self.overrides.insert(path, Override::Replace(value));
    }

    pub fn remove(&mut self, path: RepoPathBuf) {
        assert!(!path.is_root());
        self.overrides.insert(path, Override::Tombstone);
    }

    pub fn set_or_remove(&mut self, path: RepoPathBuf, value: Option<TreeValue>) {
        assert!(!path.is_root());
        if let Some(value) = value {
            self.overrides.insert(path, Override::Replace(value));
        } else {
            self.overrides.insert(path, Override::Tombstone);
        }
    }

    pub fn write_tree(self) -> BackendResult<TreeId> {
        if self.overrides.is_empty() {
            return Ok(self.base_tree_id);
        }

        let mut trees_to_write = self.get_base_trees()?;

        // Update entries in parent trees for file overrides
        for (path, file_override) in self.overrides {
            let (dir, basename) = path.split().unwrap();
            let tree_entries = trees_to_write.get_mut(dir).unwrap();
            match file_override {
                Override::Replace(value) => {
                    tree_entries.insert(basename.to_owned(), value);
                }
                Override::Tombstone => {
                    tree_entries.remove(basename);
                }
            }
        }

        // Write trees in reverse lexicographical order, starting with trees without
        // children.
        // TODO: Writing trees concurrently should help on high-latency backends
        let store = &self.store;
        while let Some((dir, cur_entries)) = trees_to_write.pop_last() {
            if let Some((parent, basename)) = dir.split() {
                let parent_entries = trees_to_write.get_mut(parent).unwrap();
                if cur_entries.is_empty() {
                    if let Some(TreeValue::Tree(_)) = parent_entries.get(basename) {
                        parent_entries.remove(basename);
                    } else {
                        // Entry would have been replaced with file (see above)
                    }
                } else {
                    let data =
                        backend::Tree::from_sorted_entries(cur_entries.into_iter().collect());
                    let tree = store.write_tree(&dir, data).block_on()?;
                    parent_entries.insert(basename.to_owned(), TreeValue::Tree(tree.id().clone()));
                }
            } else {
                // We're writing the root tree. Write it even if empty. Return its id.
                assert!(trees_to_write.is_empty());
                let data = backend::Tree::from_sorted_entries(cur_entries.into_iter().collect());
                let written_tree = store.write_tree(&dir, data).block_on()?;
                return Ok(written_tree.id().clone());
            }
        }

        unreachable!("trees_to_write must contain the root tree");
    }

    fn get_base_trees(
        &self,
    ) -> BackendResult<BTreeMap<RepoPathBuf, BTreeMap<RepoPathComponentBuf, TreeValue>>> {
        let store = &self.store;
        let mut tree_cache = {
            let dir = RepoPathBuf::root();
            let tree = store.get_tree(dir.clone(), &self.base_tree_id)?;
            BTreeMap::from([(dir, tree)])
        };

        fn populate_trees<'a>(
            tree_cache: &'a mut BTreeMap<RepoPathBuf, Tree>,
            store: &Arc<Store>,
            dir: &RepoPath,
        ) -> BackendResult<&'a Tree> {
            // `if let Some(tree) = ...` doesn't pass lifetime check as of Rust 1.84.0
            if tree_cache.contains_key(dir) {
                return Ok(tree_cache.get(dir).unwrap());
            }
            let (parent, basename) = dir.split().expect("root must be populated");
            let tree = populate_trees(tree_cache, store, parent)?
                .sub_tree(basename)?
                .unwrap_or_else(|| Tree::empty(store.clone(), dir.to_owned()));
            Ok(tree_cache.entry(dir.to_owned()).or_insert(tree))
        }

        for path in self.overrides.keys() {
            let parent = path.parent().unwrap();
            populate_trees(&mut tree_cache, store, parent)?;
        }

        Ok(tree_cache
            .into_iter()
            .map(|(dir, tree)| {
                let entries = tree
                    .data()
                    .entries()
                    .map(|entry| (entry.name().to_owned(), entry.value().clone()))
                    .collect();
                (dir, entries)
            })
            .collect())
    }
}
