#[cfg(test)]
mod tests;

use crate::fsevent::{EventFlag, FsEvent};

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::OsStr;
use std::io::{self};
use std::iter::Peekable;
use std::{
    fs::{self},
    path::{Path, PathBuf},
    time::SystemTime,
};

use bincode::{Decode, Encode};
use pathbytes::o2b;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use walkdir::{DirEntry, IntoIter, WalkDir};

#[derive(
    Serialize, Deserialize, Decode, Encode, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug,
)]
pub enum FileType {
    Dir,
    File,
    Symlink,
    Unknown,
}

impl From<fs::FileType> for FileType {
    fn from(file_type: fs::FileType) -> Self {
        if file_type.is_dir() {
            FileType::Dir
        } else if file_type.is_file() {
            FileType::File
        } else if file_type.is_symlink() {
            FileType::Symlink
        } else {
            FileType::Unknown
        }
    }
}

/// Most of the useful information for a disk node.
#[derive(Serialize, Deserialize, Decode, Encode, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Metadata {
    pub file_type: FileType,
    pub len: u64,
    pub created: SystemTime,
    pub modified: SystemTime,
    pub accessed: SystemTime,
    pub permissions_read_only: bool,
}

impl From<fs::Metadata> for Metadata {
    fn from(meta: fs::Metadata) -> Self {
        // unwrap is legal here since these things are always available on PC platforms.
        Self {
            file_type: meta.file_type().into(),
            len: meta.len(),
            created: meta.created().unwrap(),
            modified: meta.modified().unwrap(),
            accessed: meta.accessed().unwrap(),
            permissions_read_only: meta.permissions().readonly(),
        }
    }
}

#[derive(
    Serialize, Deserialize, Decode, Encode, Default, Clone, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct DiskEntry {
    /// Is None when no permission.
    pub metadata: Option<Metadata>,
    /// Is set to Some(path_seg(), entry) when entry is a folder,
    pub entries: BTreeMap<Vec<u8>, DiskEntry>,
}

impl DiskEntry {
    fn new(metadata: Option<Metadata>) -> Self {
        Self {
            metadata,
            entries: BTreeMap::default(),
        }
    }

    /// This requires a mutable reference
    pub fn merge(&mut self, base_dir: PathBuf, event: &FsEvent) {
        debug!("merge {:?} into database", event);
        let event_path = event
            .path
            .strip_prefix(&base_dir)
            .expect("event path doesn't share a common prefix with the root.");
        let mut path_segs = event_path.into_iter().peekable();
        // Ensure we are not modifying the root.
        if let Some(seg) = path_segs.next() {
            let mut current_dir = base_dir;
            self.merge_inner(event, &mut current_dir, seg, path_segs);
        } else {
            info!("event path on root: {:?}", event);
        }
    }

    /// The `path_segs.next()` is ensured to be `Some`, it's the path segment we
    /// need to match in the `self.entries`.
    fn merge_inner(
        &mut self,
        event: &FsEvent,
        current_dir: &mut PathBuf,
        seg: &OsStr,
        mut path_segs: Peekable<std::path::Iter>,
    ) {
        let next_seg = path_segs.next();
        let entry = self.entries.entry(o2b(seg).to_vec());
        let flag = match event.flag.try_into() {
            Ok(flag) => flag,
            Err(e) => {
                warn!("unreckonized event flag: {:?}", e);
                return;
            }
        };
        match entry {
            Entry::Occupied(mut entry) => {
                if let Some(next_seg) = next_seg {
                    // middle node
                    current_dir.push(seg);
                    entry
                        .get_mut()
                        .merge_inner(event, current_dir, next_seg, path_segs);
                    current_dir.pop();
                } else {
                    // leaf node
                    match flag {
                        EventFlag::Create | EventFlag::Modify | EventFlag::Delete => {
                            if let Some((name, metadata)) = fs_metadata(&event.path) {
                                debug_assert_eq!(name, o2b(seg));
                                entry.insert(DiskEntry::new(metadata));
                            } else {
                                entry.remove_entry();
                            }
                        }
                    }
                }
            }
            Entry::Vacant(entry) => {
                if let Some(next_seg) = next_seg {
                    // middle node
                    // Now we filling the middle node.
                    current_dir.push(seg);
                    if let Some((name, metadata)) = fs_metadata(current_dir) {
                        debug_assert_eq!(name, o2b(seg));
                        let entry = entry.insert(DiskEntry::new(metadata));
                        entry.merge_inner(event, current_dir, next_seg, path_segs);
                    } else {
                        debug!("But folder is not present: {:?}", current_dir);
                    }
                    current_dir.pop();
                } else {
                    // leaf node, just insert it to the entry if it's present
                    match flag {
                        EventFlag::Create | EventFlag::Modify | EventFlag::Delete => {
                            if let Some((name, metadata)) = fs_metadata(&event.path) {
                                debug_assert_eq!(name, o2b(seg));
                                entry.insert(DiskEntry::new(metadata));
                            }
                        }
                    }
                }
            }
        }
    }

    /// This function takes 400s+ on my mbp2019 512G. So don't use this function unless first boot.
    pub fn from_fs(path: &Path) -> DiskEntry {
        fn scan_folder(walker: &mut Peekable<DiskWalker>, parent_entry: &DirEntry) -> DiskEntry {
            fn get_meta(entry: &DirEntry) -> Option<Metadata> {
                entry.metadata().ok().map(|x| x.into())
            }
            let mut current_entry = DiskEntry::new(get_meta(parent_entry));
            let parent_depth = parent_entry.depth();
            loop {
                // if a node under current folder.
                let under = walker
                    .peek()
                    .map(|x| x.depth() > parent_depth)
                    .unwrap_or_default();
                if !under {
                    break;
                }
                let Some(entry) = walker.next() else { break };
                let sub_entry = if entry.file_type().is_dir() {
                    scan_folder(walker, &entry)
                } else {
                    DiskEntry::new(get_meta(&entry))
                };
                current_entry
                    .entries
                    .insert(o2b(entry.file_name()).to_vec(), sub_entry);
            }
            current_entry
        }

        let mut walker = DiskWalker::new(path).peekable();
        let entry = walker.next().unwrap();
        debug_assert_eq!(entry.path(), path);
        scan_folder(&mut walker, &entry)
    }
}

pub struct DiskWalker {
    walk_dir: IntoIter,
}

impl Iterator for DiskWalker {
    /// Metadata is none when permission denied.
    type Item = DirEntry;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.walk_dir.next()? {
                Ok(entry) => {
                    /*
                    let meta = entry.metadata().ok().map(|x| x.into());
                    let depth = entry.depth();
                    let path = entry.into_path();
                     */
                    break Some(entry);
                }
                Err(_e) => {
                    // When will this error occur: https://docs.rs/walkdir/latest/walkdir/struct.IntoIter.html#errors
                    //
                    // Info this error provides: https://docs.rs/walkdir/latest/walkdir/struct.Error.html#method.path
                    //
                    // If we are trying to scan the inner elements in permission
                    // denied folder, walkdir will return a error with the path
                    // of the folder. So this path in error is useless. continue
                }
            }
        }
    }
}

impl DiskWalker {
    pub fn new(path: &Path) -> Self {
        Self {
            walk_dir: WalkDir::new(path).same_file_system(true).into_iter(),
        }
    }
}

/// Get metadata of a path (without following the symlink). Return `Option<(filename, Option<filemeta>)>`
///
/// - When the file does not exist, return None.
/// - When the file is permission denied, return `Some(filename, None)`.
/// - When the file is reachable, return `Some(filename, Some(filemeta))`.
fn fs_metadata(path: &Path) -> Option<(&[u8], Option<Metadata>)> {
    match path.symlink_metadata() {
        Ok(metadata) => Some((o2b(path.file_name().unwrap()), Some(metadata.into()))),
        Err(error) => {
            // If the error is just a permission denied, we check
            // the ErrorKind, When a permission denied occurred,
            // create the entry with no metadata.
            let file_exists = match error.kind() {
                io::ErrorKind::PermissionDenied => true,
                io::ErrorKind::NotFound => false,
                _ => {
                    // Shouldn't emit error more than this.
                    debug_assert!(false);
                    false
                }
            };
            if file_exists {
                Some((o2b(path.file_name().unwrap()), None))
            } else {
                info!(?path, ?error, "Fetching metadata of event path failed.");
                None
            }
        }
    }
}
