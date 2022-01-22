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
use walkdir::{IntoIter, WalkDir};

#[derive(Decode, Encode, PartialEq, Eq, Clone, Copy)]
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
#[derive(Decode, Encode, Clone)]
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

#[derive(Decode, Encode, Default, Clone)]
pub struct DiskEntry {
    /// WTF-8
    name: Vec<u8>,
    /// Is None when no permission.
    metadata: Option<Metadata>,
    /// Is set to Some when entry is a folder.
    entries: Vec<DiskEntry>,
}

pub struct DiskWalker {
    walk_dir: Peekable<IntoIter>,
}

impl Iterator for DiskWalker {
    /// Metadata is none when permission denied.
    type Item = (PathBuf, Option<Metadata>);
    fn next(&mut self) -> Option<Self::Item> {
        match self.walk_dir.next()? {
            Ok(entry) => {
                let meta = entry.metadata().ok().map(|x| x.into());
                let path = entry.into_path();
                Some((path, meta))
            }
            Err(e) => match e.path() {
                Some(path) => Some((path.to_owned(), None)),
                None => None,
            },
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

pub fn scan(path: &Path) -> DiskEntry {
    let mut walker = DiskWalker::new(path).peekable();
    let (root_path, metadata) = walker.next().unwrap();
    assert_eq!(root_path, path);
    let mut entry = DiskEntry {
        name: p2b(path).to_vec(),
        metadata,
        entries: Vec::new(),
    };
    scan_folder(&mut walker, path, &mut entry);
    entry
}

fn scan_folder(walker: &mut Peekable<DiskWalker>, parent_path: &Path, entry: &mut DiskEntry) {
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
        assert!(path != parent_path);
        // Should never panic since root we are scanning after root.
        let mut entry = DiskEntry {
            name: o2b(path.file_name().expect("a root path")).to_vec(),
            metadata,
            entries: Vec::new(),
        };
        scan_folder(walker, &path, &mut entry);
        entries.push(entry);
    }
}

/*
pub struct Volumes {
    pub entries: Vec<Entry>,
}

impl Volumes {
    pub fn from_fs() -> Result<Volumes> {
        let entries = if let Ok(file) = File::open("target/hierarchy.db") {
            let mut reader = BufReader::new(file);
            //let mut reader = flate2::read::ZlibDecoder::new(reader);
            bincode::decode_from_std_read(&mut reader, Configuration::standard())
                .context("decode from db failed.")?
        } else {
            scan_hierarchy()
        };
        Ok(Volumes { entries })
    }

    pub fn to_fs(&self) -> Result<()> {
        let file = File::create("target/hierarchy.db").context("create db failed")?;
        let mut file = BufWriter::new(file);
        // let mut file = flate2::write::ZlibEncoder::new(file, flate2::Compression::fast());
        bincode::encode_into_std_write(&self.entries, &mut file, Configuration::standard())
            .context("Write to db failed.")?;
        Ok(())
    }
}
*/

#[allow(clippy::all)]
#[cfg(test)]
mod test {
    use super::*;
    use std::borrow::Borrow;
    use std::{
        fs::{self, File},
        os::unix::fs as unixfs,
        path::{Path, PathBuf},
    };
    use tempfile::TempDir;

    fn compare_test_entry(a: impl Borrow<DiskEntry>, b: impl Borrow<DiskEntry>) {
        let a = a.borrow();
        let b = b.borrow();
        if a.name != b.name {
            panic!()
        }
        if a.metadata.clone().map(|Metadata { file_type, len, .. }| {
            (file_type, if file_type == FileType::File { len } else { 0 })
        }) != b.metadata.clone().map(|Metadata { file_type, len, .. }| {
            (file_type, if file_type == FileType::File { len } else { 0 })
        }) {
            panic!()
        }
        a.entries
            .iter()
            .zip(b.entries.iter())
            .for_each(|(a, b)| compare_test_entry(a, b))
    }

    impl Default for Metadata {
        fn default() -> Self {
            Self {
                file_type: FileType::Unknown,
                len: 0,
                created: SystemTime::UNIX_EPOCH,
                accessed: SystemTime::UNIX_EPOCH,
                modified: SystemTime::UNIX_EPOCH,
                permissions_read_only: false,
            }
        }
    }

    fn complex_entry() -> DiskEntry {
        DiskEntry {
            name: b"tmp_folder".to_vec(),
            metadata: Some(Metadata {
                file_type: FileType::Dir,
                ..Default::default()
            }),
            entries: vec![
                DiskEntry {
                    name: b"afolder".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::Dir,
                        ..Default::default()
                    }),
                    entries: vec![DiskEntry {
                        name: b"hello.txt".to_vec(),
                        metadata: Some(Metadata {
                            file_type: FileType::File,
                            len: 666,
                            ..Default::default()
                        }),
                        entries: Vec::new(),
                    }],
                },
                DiskEntry {
                    name: b"233.txt".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 233,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "445.txt".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 445,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "heck.txt".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 0,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "src".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::Dir,
                        ..Default::default()
                    }),
                    entries: vec![DiskEntry {
                        name: "template".into(),
                        metadata: Some(Metadata {
                            file_type: FileType::Dir,
                            ..Default::default()
                        }),
                        entries: vec![DiskEntry {
                            name: "hello.java".into(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 514,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        }],
                    }],
                },
            ],
        }
    }

    #[test]
    fn entry_from_empty_folder() {
        let tempdir = TempDir::new().unwrap();
        let path = tempdir.path();
        let entry = scan(path);
        compare_test_entry(
            DiskEntry {
                name: p2b(path).to_vec(),
                metadata: Some(Metadata {
                    file_type: FileType::Dir,
                    ..Default::default()
                }),
                entries: Vec::new(),
            },
            entry,
        )
    }

    #[test]
    fn entry_from_single_file() {
        let tempdir = TempDir::new().unwrap();
        let path = tempdir.path();
        let path = path.join("emm.txt");
        fs::write(&path, vec![42; 1000]).unwrap();
        let entry = scan(&path);
        compare_test_entry(
            entry,
            DiskEntry {
                name: p2b(&path).to_vec(),
                metadata: Some(Metadata {
                    file_type: FileType::File,
                    len: 1000,
                    ..Default::default()
                }),
                entries: Vec::new(),
            },
        );
    }

    /*
    #[test]
    fn entry_from_full_folder() {
        let tempdir = TempDir::new().unwrap();
        let path = &tempdir.path();
        fs::create_dir_all(path.join("afolder")).unwrap();
        fs::create_dir_all(path.join("bfolder")).unwrap();
        fs::create_dir_all(path.join("bfolder/cfolder")).unwrap();
        fs::write(path.join("abc"), vec![42; 233]).unwrap();
        fs::write(path.join("ldm"), vec![42; 288]).unwrap();
        fs::write(path.join("vvv"), vec![42; 12]).unwrap();
        fs::write(path.join("afolder/foo"), vec![42; 666]).unwrap();
        fs::write(path.join("afolder/bar"), vec![42; 89]).unwrap();
        fs::write(path.join("bfolder/foo"), vec![42; 11]).unwrap();
        File::create(path.join("bfolder/bar")).unwrap();
        File::create(path.join("bfolder/cfolder/another")).unwrap();
        let entry =
            scan(path);
        let structure = serde_json::to_string_pretty(&entry).unwrap();

        assert_eq!(
            structure,
            r#"{
      "Folder": {
        "name": "",
        "entries": [
          {
            "File": {
              "name": "abc",
              "size": 233
            }
          },
          {
            "File": {
              "name": "vvv",
              "size": 12
            }
          },
          {
            "Folder": {
              "name": "bfolder",
              "entries": [
                {
                  "File": {
                    "name": "foo",
                    "size": 11
                  }
                },
                {
                  "File": {
                    "name": "bar",
                    "size": 0
                  }
                },
                {
                  "Folder": {
                    "name": "cfolder",
                    "entries": [
                      {
                        "File": {
                          "name": "another",
                          "size": 0
                        }
                      }
                    ]
                  }
                }
              ]
            }
          },
          {
            "File": {
              "name": "ldm",
              "size": 288
            }
          },
          {
            "Folder": {
              "name": "afolder",
              "entries": [
                {
                  "File": {
                    "name": "foo",
                    "size": 666
                  }
                },
                {
                  "File": {
                    "name": "bar",
                    "size": 89
                  }
                }
              ]
            }
          }
        ]
      }
    }"#
        );
    }

    #[cfg(target_family = "unix")]
    mod symlink_tests {
        use super::*;

        fn complex_directory_with_symlink(path: &Path) {
            fs::create_dir(path.join("afolder")).unwrap();
            fs::create_dir(path.join("bfolder")).unwrap();
            fs::create_dir(path.join("bfolder/cfolder")).unwrap();
            unixfs::symlink(path.join("bfolder/cfolder"), path.join("dfolder")).unwrap();
            File::create(path.join("abc")).unwrap();
            File::create(path.join("ldm")).unwrap();
            File::create(path.join("vvv")).unwrap();
            fs::write(path.join("afolder/foo"), vec![42; 71]).unwrap();
            fs::write(path.join("afolder/kksk"), vec![42; 121]).unwrap();
            File::create(path.join("afolder/bar")).unwrap();
            File::create(path.join("bfolder/foo")).unwrap();
            File::create(path.join("bfolder/bar")).unwrap();
            fs::write(path.join("bfolder/kksk"), vec![42; 188]).unwrap();
            File::create(path.join("bfolder/cfolder/another")).unwrap();
            unixfs::symlink(path.join("afolder/bar"), path.join("afolder/baz")).unwrap();
            unixfs::symlink(path.join("afolder/foo"), path.join("bfolder/foz")).unwrap();
        }

        #[test]
        fn test_symlink() {
            let tempdir = TempDir::new().unwrap();
            let path = &tempdir.path();
            complex_directory_with_symlink(&path);
            let (entry, entry_info) =
                Entry::from_fs(&path, std::usize::MAX, std::usize::MAX, std::u64::MAX).unwrap();
            assert_eq!(
                entry_info,
                EntryInfo {
                    max_depth: 3,
                    file_num: 10,
                    file_size: 380,
                }
            );
            let structure = serde_json::to_string_pretty(&entry).unwrap();
            assert_eq!(
                structure,
                r#"{
      "Folder": {
        "name": "",
        "entries": [
          {
            "File": {
              "name": "abc",
              "size": 0
            }
          },
          {
            "File": {
              "name": "vvv",
              "size": 0
            }
          },
          {
            "Symlink": {
              "name": "dfolder"
            }
          },
          {
            "Folder": {
              "name": "bfolder",
              "entries": [
                {
                  "File": {
                    "name": "kksk",
                    "size": 188
                  }
                },
                {
                  "Symlink": {
                    "name": "foz"
                  }
                },
                {
                  "File": {
                    "name": "foo",
                    "size": 0
                  }
                },
                {
                  "File": {
                    "name": "bar",
                    "size": 0
                  }
                },
                {
                  "Folder": {
                    "name": "cfolder",
                    "entries": [
                      {
                        "File": {
                          "name": "another",
                          "size": 0
                        }
                      }
                    ]
                  }
                }
              ]
            }
          },
          {
            "File": {
              "name": "ldm",
              "size": 0
            }
          },
          {
            "Folder": {
              "name": "afolder",
              "entries": [
                {
                  "File": {
                    "name": "kksk",
                    "size": 121
                  }
                },
                {
                  "File": {
                    "name": "foo",
                    "size": 71
                  }
                },
                {
                  "Symlink": {
                    "name": "baz"
                  }
                },
                {
                  "File": {
                    "name": "bar",
                    "size": 0
                  }
                }
              ]
            }
          }
        ]
      }
    }"#
            )
        }

        #[test]
        fn test_symlink_iterators() {
            let tempdir = TempDir::new().unwrap();
            let path = &tempdir.path();
            complex_directory_with_symlink(&path);
            let (entry, entry_info) =
                Entry::from_fs(&path, std::usize::MAX, std::usize::MAX, std::u64::MAX).unwrap();
            assert_eq!(
                entry_info,
                EntryInfo {
                    max_depth: 3,
                    file_num: 10,
                    file_size: 380,
                }
            );
            let symlinks: Vec<_> = entry.symlinks().collect();
            assert_eq!(
                symlinks,
                vec![
                    PathBuf::from("dfolder"),
                    PathBuf::from("bfolder/foz"),
                    PathBuf::from("afolder/baz"),
                ]
            )
        }
    }
    */
}
