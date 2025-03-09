#[cfg(test)]
mod tests;

use crate::fs_entry::DiskEntry;
use crate::fsevent::{EventId, FsEvent};

use anyhow::{Context, Result};
use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufWriter;
use std::{io::BufReader, path::Path};
use tracing::info;

/// The overall database of Cardinal.
///
/// It's created or loaded on app starts, stored into disk on app closes.
#[derive(Decode, Encode, Serialize, Deserialize)]
pub struct Database {
    /// The snapshot time of this file system tree.
    time: EventId,
    /// Snapshot of the file system tree.
    fs_entry: DiskEntry,
}

const CONFIG: bincode::config::Configuration = bincode::config::standard();

impl Database {
    pub fn from_fs(path: &Path) -> Result<Self> {
        let file = File::open(path).context("load db from disk failed.")?;
        let mut file = BufReader::new(file);
        let database =
            bincode::decode_from_std_read(&mut file, CONFIG).context("Decode failed.")?;
        Ok(database)
    }

    pub fn into_fs(&self, path: &Path) -> Result<()> {
        let file = File::create(path).context("open db file from disk failed.")?;
        let mut file = BufWriter::new(file);
        bincode::encode_into_std_write(self, &mut file, CONFIG).context("Encode failed.")?;
        Ok(())
    }

    pub fn merge(&mut self, event: &FsEvent) {
        if event.id < self.time.raw_event_id {
            info!("Ignore old event");
            return;
        }
        self.time = EventId::now_with_id(event.id);
        self.fs_entry.merge(Path::new("/").to_path_buf(), event)
    }

    /// Return event_id of the last merged event.
    pub fn last_event_id(&self) -> u64 {
        self.time.raw_event_id
    }
}

/// The PartialDatabase contains the file system snapshot and the time starting
/// to take the snapshot.
///
/// To make it really useful, merge the filesystem change(from start time to
/// current time) into the file system.
pub struct PartialDatabase {
    /// The time starting to scan this file system tree.
    time: EventId,
    /// Snapshot of the file system tree.
    fs_entry: DiskEntry,
}

impl PartialDatabase {
    /// Scan the hierarchy from file system.
    pub fn scan_fs() -> Self {
        let start_time = EventId::now();
        info!(?start_time, "The start time of fs scanning");
        let fs_entry = DiskEntry::from_fs(Path::new("/"));
        Self {
            time: start_time,
            fs_entry,
        }
    }

    pub fn merge(&mut self, event: &FsEvent) {
        info!(?event, "Merge event into partial db");
        if event.id > self.time.raw_event_id {
            info!("Ignore old event");
            return;
        }
        self.time = EventId::now_with_id(event.id);
        self.fs_entry.merge(Path::new("/").to_path_buf(), event)
    }

    /// Complete modification merging. Convert self into a serializable database.
    /// `time` is the time of last_fs_event.
    pub fn complete_merge(self) -> Database {
        info!(
            merge_complete_time = ?self.time,
            "Merging fs events into scanned result completes"
        );
        Database {
            time: self.time,
            fs_entry: self.fs_entry,
        }
    }
}
