use super::*;
use pathbytes::b2p;
use std::borrow::Borrow;
use std::collections::BTreeSet;
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
        panic!("a: {:?} b: {:?}", b2p(&a.name), b2p(&b.name))
    }
    if a.metadata.clone().map(|Metadata { file_type, len, .. }| {
        (file_type, if file_type == FileType::File { len } else { 0 })
    }) != b.metadata.clone().map(|Metadata { file_type, len, .. }| {
        (file_type, if file_type == FileType::File { len } else { 0 })
    }) {
        panic!("a: {:?} b: {:?}", b2p(&a.name), b2p(&b.name))
    }
    let a_set: BTreeSet<_> = a.entries.clone().into_iter().collect();
    let b_set: BTreeSet<_> = b.entries.clone().into_iter().collect();
    a_set
        .into_iter()
        .zip(b_set.into_iter())
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

#[test]
fn test_complex_entry_scanner() {
    fn complex_entry(path: &Path) -> DiskEntry {
        DiskEntry {
            name: p2b(path).to_vec(),
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

    let tempdir = TempDir::new().unwrap();
    let path = &tempdir.path();
    fs::create_dir_all(path.join("afolder")).unwrap();
    fs::write(path.join("afolder/hello.txt"), vec![42; 666]).unwrap();
    fs::write(path.join("233.txt"), vec![42; 233]).unwrap();
    fs::write(path.join("445.txt"), vec![42; 445]).unwrap();
    fs::write(path.join("heck.txt"), vec![0; 0]).unwrap();
    fs::create_dir_all(path.join("src/template")).unwrap();
    fs::write(path.join("src/template/hello.java"), vec![42; 514]).unwrap();

    let entry = scan(path);
    compare_test_entry(entry, complex_entry(path));
}

#[test]
fn entry_from_full_folder() {
    fn full_entry(path: &Path) -> DiskEntry {
        DiskEntry {
            name: p2b(path).to_vec(),
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
                    entries: vec![
                        DiskEntry {
                            name: b"foo".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 666,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"bar".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 89,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                    ],
                },
                DiskEntry {
                    name: b"bfolder".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::Dir,
                        ..Default::default()
                    }),
                    entries: vec![
                        DiskEntry {
                            name: b"cfolder".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::Dir,
                                ..Default::default()
                            }),
                            entries: vec![DiskEntry {
                                name: b"another".to_vec(),
                                metadata: Some(Metadata {
                                    file_type: FileType::File,
                                    len: 0,
                                    ..Default::default()
                                }),
                                entries: Vec::new(),
                            }],
                        },
                        DiskEntry {
                            name: b"foo".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 11,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"bar".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 0,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                    ],
                },
                DiskEntry {
                    name: b"abc".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 233,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "ldm".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 288,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "vvv".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 12,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
            ],
        }
    }

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
    let entry = scan(path);
    compare_test_entry(entry, full_entry(path));
}

#[cfg(target_family = "unix")]
mod symlink_tests {
    use super::*;

    fn create_complex_directory_with_symlink(path: &Path) {
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

    fn complex_entry_with_symlink(path: &Path) -> DiskEntry {
        DiskEntry {
            name: p2b(path).to_vec(),
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
                    entries: vec![
                        DiskEntry {
                            name: b"foo".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 71,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"bar".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 0,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"kksk".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 121,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"baz".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::Symlink,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                    ],
                },
                DiskEntry {
                    name: b"dfolder".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::Symlink,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: b"bfolder".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::Dir,
                        ..Default::default()
                    }),
                    entries: vec![
                        DiskEntry {
                            name: b"cfolder".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::Dir,
                                ..Default::default()
                            }),
                            entries: vec![DiskEntry {
                                name: b"another".to_vec(),
                                metadata: Some(Metadata {
                                    file_type: FileType::File,
                                    len: 0,
                                    ..Default::default()
                                }),
                                entries: Vec::new(),
                            }],
                        },
                        DiskEntry {
                            name: b"foo".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 0,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"foz".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::Symlink,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"bar".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 0,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                        DiskEntry {
                            name: b"kksk".to_vec(),
                            metadata: Some(Metadata {
                                file_type: FileType::File,
                                len: 188,
                                ..Default::default()
                            }),
                            entries: Vec::new(),
                        },
                    ],
                },
                DiskEntry {
                    name: b"abc".to_vec(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 0,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "ldm".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 0,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
                DiskEntry {
                    name: "vvv".into(),
                    metadata: Some(Metadata {
                        file_type: FileType::File,
                        len: 0,
                        ..Default::default()
                    }),
                    entries: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn test_symlink() {
        let tempdir = TempDir::new().unwrap();
        let path = &tempdir.path();
        create_complex_directory_with_symlink(path);
        let entry = scan(path);
        compare_test_entry(entry, complex_entry_with_symlink(path));
    }
}
