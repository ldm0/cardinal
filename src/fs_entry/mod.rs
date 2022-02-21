#[cfg(test)]
mod tests;

use crate::fsevent::{EventFlag, FsEvent};

use std::ffi::OsString;
use std::io::{self, BufWriter};
use std::iter::Peekable;
use std::{
    fs::{self, File},
    io::prelude::*,
    io::BufReader,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};
use bincode::{config::Configuration, Decode, Encode};
use pathbytes::{b2p, o2b, p2b};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};
use walkdir::{IntoIter, WalkDir};

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
#[derive(Serialize, Deserialize, Decode, Encode, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Metadata {
    file_type: FileType,
    len: u64,
    created: SystemTime,
    modified: SystemTime,
    accessed: SystemTime,
    permissions_read_only: bool,
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
    /// WTF-8
    name: Vec<u8>,
    /// Is None when no permission.
    metadata: Option<Metadata>,
    /// Is set to Some when entry is a folder.
    entries: Vec<DiskEntry>,
}

impl DiskEntry {
    fn new(name: &[u8], metadata: Option<Metadata>) -> Self {
        Self {
            name: name.to_vec(),
            metadata,
            entries: Vec::new(),
        }
    }

    /// This requires a mutable reference
    pub fn merge(&mut self, event: &FsEvent) {
        debug!("merge {:?} into database", event);
        let event_path = p2b(&event.path)
            .strip_prefix(&*self.name)
            .expect("event path doesn't share a common prefix with the root.");
        let event_path = b2p(&event_path);
        let mut path_segs = event_path.into_iter().peekable();
        // Ensure we are not modifying the root. (When this feature is really
        // needed, add a match branch here for special-case this).
        debug_assert!(path_segs.peek().is_some());
        self.merge_inner(event, path_segs);
    }

    /// The `path_segs.next()` is ensured to be `Some`, it's the path segment we
    /// need to match in the `self.entries`.
    fn merge_inner(&mut self, event: &FsEvent, mut path_segs: Peekable<std::path::Iter>) {
        let seg = path_segs.next().map(o2b).unwrap();
        let entry = self
            .entries
            .iter_mut()
            .enumerate()
            .find_map(|(index, entry)| (entry.name == seg).then(|| (index, entry)));
        if let Some((index, entry)) = entry {
            match event.flag {
                EventFlag::Create | EventFlag::Modify => {
                    if path_segs.peek().is_none() {
                        // If we are on the node being processed directly.
                        if matches!(event.flag, EventFlag::Create) {
                            // This should happen on racing. We are creating a present file.
                            warn!(?event.path, "Creating an present file!");
                        }
                        if let Some((name, metadata)) = fs_metadata(&event.path) {
                            *entry = DiskEntry::new(name, metadata);
                        }
                    } else {
                        entry.merge_inner(event, path_segs);
                    }
                }
                EventFlag::Delete => {
                    self.entries.remove(index);
                }
            }
        } else {
            // `kFSEventStreamEventFlagItemRenamed` doesn't provide information about whether it's currently present, so modified unpresent file is acceptable.
            match event.flag {
                EventFlag::Create | EventFlag::Modify => {
                    if matches!(event.flag, EventFlag::Modify) {
                        info!(?event.path, "Modifying an unpresent file");
                    }

                    // Fetching the metadata from fs, create the entry.

                    // TODO(ldm0) There is a possibility of racing, e.g.
                    //
                    // Processing events:
                    //
                    // event1: modify /foo/bar
                    // event2: modify /foo/bar
                    //
                    // The file state is newer than event2 occurs, but we are processing the event1.
                    //
                    // Since we are assuming the file system event as a
                    // modification trigger rather than a data provider now,
                    // This doesn't hurt much.
                    if let Some((name, metadata)) = fs_metadata(&event.path) {
                        self.entries.push(DiskEntry::new(name, metadata))
                    }
                }
                EventFlag::Delete => {
                    // This should never happen. We are deleting an unpresent file.
                    error!(?event.path, "Deleting an unpresent file!");
                }
            }
        }
    }

    /// This function takes 400s+ on my mbp2019 512G. So don't use this function unless first boot.
    pub fn from_fs(path: &Path) -> DiskEntry {
        fn scan_folder(
            walker: &mut Peekable<DiskWalker>,
            parent_path: &Path,
            entry: &mut DiskEntry,
        ) {
            let DiskEntry { entries, .. } = entry;
            loop {
                // if a node under parent node.
                let under_parent = walker
                    .peek()
                    .map(|(path, _)| path.starts_with(parent_path))
                    .unwrap_or_default();
                if !under_parent {
                    break;
                }
                let (path, metadata) = match walker.next() {
                    Some(x) => x,
                    None => break,
                };
                // Should never panic since walkdir shouldn't emit same path twice.
                assert_ne!(path, parent_path);
                // Should never panic since root we are scanning after root.
                let mut entry = DiskEntry::new(o2b(path.file_name().expect("root path")), metadata);
                scan_folder(walker, &path, &mut entry);
                entries.push(entry);
            }
        }

        let mut walker = DiskWalker::new(path).peekable();
        let (root_path, metadata) = walker.next().unwrap();
        assert_eq!(root_path, path);
        let mut entry = DiskEntry::new(p2b(path), metadata);
        scan_folder(&mut walker, path, &mut entry);
        entry
    }
}

pub struct DiskWalker {
    walk_dir: Peekable<IntoIter>,
}

impl Iterator for DiskWalker {
    /// Metadata is none when permission denied.
    type Item = (PathBuf, Option<Metadata>);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.walk_dir.next()? {
                Ok(entry) => {
                    let meta = entry.metadata().ok().map(|x| x.into());
                    let path = entry.into_path();
                    break Some((path, meta));
                }
                Err(e) => {
                    if let Some(path) = e.path() {
                        // If we are trying to scan the inner elements in permission
                        // denied folder, walkdir will return a error with the path
                        // of the folder, here we filter out the dir path emitted in
                        // this situation.
                        if let Ok(x) = fs::symlink_metadata(path) {
                            if !x.is_dir() {
                                break Some((path.to_owned(), None));
                            }
                        }
                    }
                }
            }
        }
    }
}

impl DiskWalker {
    pub fn new(path: &Path) -> Self {
        Self {
            walk_dir: WalkDir::new(path).into_iter().peekable(),
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
        Ok(metadata) => Some((
            o2b(path.file_name().expect("root path")),
            Some(metadata.into()),
        )),
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
                Some((o2b(path.file_name().expect("root path")), None))
            } else {
                info!(?path, ?error, "Fetching metadata of event path failed.");
                None
            }
        }
    }
}
