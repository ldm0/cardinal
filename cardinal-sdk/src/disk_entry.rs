use crate::consts::CONFIG;
use crate::models::DiskEntryRaw;
use bincode::{Decode, Encode};
use std::fs;
use std::{path::PathBuf, time::SystemTime};

#[derive(Encode, Decode, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
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
#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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

pub struct DiskEntry {
    pub path: PathBuf,
    pub meta: Metadata,
}

impl DiskEntry {
    pub fn to_raw(&self) -> Result<DiskEntryRaw, bincode::error::EncodeError> {
        let the_meta = bincode::encode_to_vec(&self.meta, CONFIG)?;
        Ok(DiskEntryRaw {
            the_path: self.path.to_string_lossy().into_owned(),
            the_meta,
        })
    }
}

impl TryFrom<DiskEntryRaw> for DiskEntry {
    type Error = bincode::error::DecodeError;
    fn try_from(entry: DiskEntryRaw) -> Result<Self, Self::Error> {
        let (meta, _) = bincode::decode_from_slice(&entry.the_meta, CONFIG)?;
        Ok(Self {
            path: PathBuf::from(entry.the_path),
            meta,
        })
    }
}
