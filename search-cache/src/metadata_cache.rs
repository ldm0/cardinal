use crate::SlabNodeMetadataCompact;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MetadataCache {
    ctime_index: BTreeMap<NonZeroU32, Vec<usize>>,
    mtime_index: BTreeMap<NonZeroU32, Vec<usize>>,
    size_index: BTreeMap<u64, Vec<usize>>,
    /// For slab nodes without metadata
    no_ctime_index: BTreeSet<usize>,
    no_mtime_index: BTreeSet<usize>,
    no_size_index: BTreeSet<usize>,
}

impl MetadataCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, index: usize, metadata: SlabNodeMetadataCompact) {
        if let Some(ctime) = metadata.as_ref().and_then(|x| x.ctime()) {
            if let Some(indexes) = self.ctime_index.get_mut(&ctime) {
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            } else {
                self.ctime_index.insert(ctime, vec![index]);
            }
        } else {
            self.no_ctime_index.insert(index);
        }
        if let Some(mtime) = metadata.as_ref().and_then(|x| x.mtime()) {
            if let Some(indexes) = self.mtime_index.get_mut(&mtime) {
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            } else {
                self.mtime_index.insert(mtime, vec![index]);
            }
        } else {
            self.no_mtime_index.insert(index);
        }
        if let Some(size) = metadata.as_ref().map(|x| x.size()) {
            if let Some(indexes) = self.size_index.get_mut(&size) {
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            } else {
                self.size_index.insert(size, vec![index]);
            }
        } else {
            self.no_size_index.insert(index);
        }
    }

    pub fn remove(&mut self, index: usize, metadata: SlabNodeMetadataCompact) {
        if let Some(ctime) = metadata.as_ref().and_then(|x| x.ctime()) {
            if let Some(indexes) = self.ctime_index.get_mut(&ctime) {
                indexes.retain(|&x| x != index);
                if indexes.is_empty() {
                    self.ctime_index.remove(&ctime);
                }
            }
        } else {
            self.no_ctime_index.remove(&index);
        }
        if let Some(mtime) = metadata.as_ref().and_then(|x| x.mtime()) {
            if let Some(indexes) = self.mtime_index.get_mut(&mtime) {
                indexes.retain(|&x| x != index);
                if indexes.is_empty() {
                    self.mtime_index.remove(&mtime);
                }
            }
        } else {
            self.no_mtime_index.remove(&index);
        }
        if let Some(size) = metadata.as_ref().map(|x| x.size()) {
            if let Some(indexes) = self.size_index.get_mut(&size) {
                indexes.retain(|&x| x != index);
                if indexes.is_empty() {
                    self.size_index.remove(&size);
                }
            }
        } else {
            self.no_size_index.remove(&index);
        }
    }
}
