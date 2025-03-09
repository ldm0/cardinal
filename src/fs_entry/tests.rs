use super::*;
use cardinal_sdk::fsevent::MacEventFlag;
use pathbytes::b2p;
use std::fs::OpenOptions;
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::{
    fs::{self, File},
    os::unix::fs as unixfs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

/// Compare two entries without comparing the create, accessed, modified time.
/// Useful for manually testing.
fn compare_test_entry(a: &DiskEntry, b: &DiskEntry) {
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
        .for_each(|((aname, a), (bname, b))| {
            unsafe { assert_eq!(b2p(&aname), b2p(bname)) };
            compare_test_entry(a, b);
        })
}

fn metadata_dir() -> Metadata {
    Metadata {
        file_type: FileType::Dir,
        ..Default::default()
    }
}

fn entry_file_with_name(name: &[u8], len: u64) -> (Vec<u8>, DiskEntry) {
    (name.to_vec(), entry_file(len))
}

fn entry_file(len: u64) -> DiskEntry {
    DiskEntry {
        metadata: Some(Metadata {
            file_type: FileType::File,
            len,
            ..Default::default()
        }),
        entries: Default::default(),
    }
}

fn entry_symlink_with_name(name: &[u8]) -> (Vec<u8>, DiskEntry) {
    (
        name.to_vec(),
        DiskEntry {
            metadata: Some(Metadata {
                file_type: FileType::Symlink,
                ..Default::default()
            }),
            entries: Default::default(),
        },
    )
}

fn entry_folder_with_name<const N: usize>(
    name: &[u8],
    entries: [(Vec<u8>, DiskEntry); N],
) -> (Vec<u8>, DiskEntry) {
    (name.to_vec(), entry_folder(entries))
}

fn entry_folder<const N: usize>(entries: [(Vec<u8>, DiskEntry); N]) -> DiskEntry {
    DiskEntry {
        metadata: Some(metadata_dir()),
        entries: entries.into_iter().collect(),
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ComplexEntry {
    base_dir: PathBuf,
    entry: DiskEntry,
}

impl ComplexEntry {
    fn merge(&mut self, event: &FsEvent) {
        self.entry.merge(self.base_dir.clone(), event);
    }

    fn from_fs(path: &Path) -> Self {
        Self {
            base_dir: path.to_path_buf(),
            entry: DiskEntry::from_fs(path),
        }
    }
}

impl Deref for ComplexEntry {
    type Target = DiskEntry;

    fn deref(&self) -> &DiskEntry {
        &self.entry
    }
}

impl DerefMut for ComplexEntry {
    fn deref_mut(&mut self) -> &mut DiskEntry {
        &mut self.entry
    }
}

fn complex_entry<P: AsRef<Path>>(path: P) -> ComplexEntry {
    let path = path.as_ref();
    ComplexEntry {
        base_dir: path.to_path_buf(),
        entry: entry_folder([
            entry_folder_with_name(b"afolder", [entry_file_with_name(b"hello.txt", 666)]),
            entry_file_with_name(b"233.txt", 233),
            entry_file_with_name(b"445.txt", 445),
            entry_file_with_name(b"heck.txt", 0),
            entry_folder_with_name(
                b"src",
                [entry_folder_with_name(
                    b"template",
                    [entry_file_with_name(b"hello.java", 514)],
                )],
            ),
        ]),
    }
}

fn apply_complex_entry(path: &Path) {
    fs::create_dir_all(path.join("afolder")).unwrap();
    fs::write(path.join("afolder/hello.txt"), vec![42; 666]).unwrap();
    fs::write(path.join("233.txt"), vec![42; 233]).unwrap();
    fs::write(path.join("445.txt"), vec![42; 445]).unwrap();
    fs::write(path.join("heck.txt"), vec![0; 0]).unwrap();
    fs::create_dir_all(path.join("src/template")).unwrap();
    fs::write(path.join("src/template/hello.java"), vec![42; 514]).unwrap();
}

fn full_entry() -> DiskEntry {
    entry_folder([
        entry_folder_with_name(
            b"afolder",
            [
                entry_file_with_name(b"foo", 666),
                entry_file_with_name(b"bar", 89),
            ],
        ),
        entry_folder_with_name(
            b"bfolder",
            [
                entry_folder_with_name(b"cfolder", [entry_file_with_name(b"another", 0)]),
                entry_file_with_name(b"foo", 11),
                entry_file_with_name(b"bar", 0),
            ],
        ),
        entry_file_with_name(b"abc", 233),
        entry_file_with_name(b"ldm", 288),
        entry_file_with_name(b"vvv", 12),
    ])
}

fn apply_full_entry(path: &Path) {
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
    let entry = DiskEntry::from_fs(path);
    compare_test_entry(&entry_folder([]), &entry)
}

#[test]
fn entry_from_single_file() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    let path = path.join("emm.txt");
    fs::write(&path, vec![42; 1000]).unwrap();
    let entry = DiskEntry::from_fs(&path);
    compare_test_entry(&entry, &entry_file(1000));
}

#[test]
fn test_complex_entry_scanner() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    apply_complex_entry(path);
    let entry = DiskEntry::from_fs(path);
    compare_test_entry(&entry, &complex_entry(path).entry);
}

#[test]
fn entry_from_full_folder() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    apply_full_entry(path);
    let entry = DiskEntry::from_fs(path);
    compare_test_entry(&entry, &full_entry());
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

    fn complex_entry_with_symlink() -> DiskEntry {
        entry_folder([
            entry_folder_with_name(
                b"afolder",
                [
                    entry_file_with_name(b"foo", 71),
                    entry_file_with_name(b"bar", 0),
                    entry_file_with_name(b"kksk", 121),
                    entry_symlink_with_name(b"baz"),
                ],
            ),
            entry_symlink_with_name(b"dfolder"),
            entry_folder_with_name(
                b"bfolder",
                [
                    entry_folder_with_name(b"cfolder", [entry_file_with_name(b"another", 0)]),
                    entry_file_with_name(b"foo", 0),
                    entry_symlink_with_name(b"foz"),
                    entry_file_with_name(b"bar", 0),
                    entry_file_with_name(b"kksk", 188),
                ],
            ),
            entry_file_with_name(b"abc", 0),
            entry_file_with_name(b"ldm", 0),
            entry_file_with_name(b"vvv", 0),
        ])
    }

    #[test]
    fn test_symlink() {
        let tempdir = TempDir::new().unwrap();
        let path = tempdir.path();
        create_complex_directory_with_symlink(path);
        let entry = DiskEntry::from_fs(path);
        compare_test_entry(&entry, &complex_entry_with_symlink());
    }
}

#[test]
fn test_simple_entry_merging() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    // DiskEntry::new()
}

/* comment out due to fake fs path
#[test]
fn test_complex_entry_merging() {
    // Delete
    {
        let mut entry = complex_entry("");
        entry.merge(&FsEvent {
            path: "/445.txt".into(),
            flag: EventFlag::Delete,
            id: 0,
        });
        let mut expected = complex_entry("");
        expected.entries.remove(&b"445.txt".to_vec()).unwrap();
        compare_test_entry(&entry.entry, &expected.entry)
    }

    // Create uncreated file.
    {
        let mut entry = complex_entry("");
        entry.merge(&FsEvent {
            path: "/asdfasdfknasdf.txt".into(),
            flag: EventFlag::Create,
            id: 0,
        });
        let expected = complex_entry("");
        compare_test_entry(&entry.entry, &expected.entry)
    }

    // Modify uncreated file.
    {
        let mut entry = complex_entry("");
        entry.merge(&FsEvent {
            path: "/11451419190810.txt".into(),
            flag: EventFlag::Modify,
            id: 0,
        });
        let expected = complex_entry("");
        compare_test_entry(&entry.entry, &expected.entry)
    }
}
 */

#[test]
fn test_on_disk_entry_modifying() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    apply_complex_entry(path);

    // Write 6 extra bytes to 445.txt.
    {
        let mut file = OpenOptions::new()
            .append(true)
            .open(path.join("445.txt"))
            .unwrap();
        file.write_all(b"hello?").unwrap();
        drop(file);
    }
    // Write 8 extra bytes to hello.java.
    {
        let mut file = OpenOptions::new()
            .append(true)
            .open(path.join("src/template/hello.java"))
            .unwrap();
        file.write_all(b"asdfasdf").unwrap();
        drop(file);
    }

    let mut entry = ComplexEntry::from_fs(path);
    entry.merge(&FsEvent {
        path: path.join("445.txt"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemModified,
        id: 0,
    });
    entry.merge(&FsEvent {
        path: path.join("src/template/hello.java"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemModified,
        id: 0,
    });
    let x = entry.entries.get(&b"445.txt".to_vec()).unwrap();
    let metadata = x.metadata.as_ref().unwrap();
    assert_eq!(metadata.permissions_read_only, false);
    assert_ne!(metadata.created, SystemTime::UNIX_EPOCH);
    assert_ne!(metadata.modified, SystemTime::UNIX_EPOCH);
    assert_ne!(metadata.accessed, SystemTime::UNIX_EPOCH);
    assert_eq!(metadata.len, 451);
    assert_eq!(metadata.file_type, FileType::File);

    let src = entry.entries.get(&b"src".to_vec()).unwrap();
    let template = src.entries.get(&b"template".to_vec()).unwrap();
    let x = template.entries.get(&b"hello.java".to_vec()).unwrap();
    let metadata = x.metadata.as_ref().unwrap();
    assert_eq!(metadata.permissions_read_only, false);
    assert_ne!(metadata.created, SystemTime::UNIX_EPOCH);
    assert_ne!(metadata.modified, SystemTime::UNIX_EPOCH);
    assert_ne!(metadata.accessed, SystemTime::UNIX_EPOCH);
    assert_eq!(metadata.len, 522);
    assert_eq!(metadata.file_type, FileType::File);
}

#[test]
fn test_on_disk_entry_deleting() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    apply_complex_entry(path);

    // Remove `445.txt`.
    fs::remove_file(path.join("445.txt")).unwrap();
    // Remove `template` folder.
    fs::remove_dir_all(path.join("src/template")).unwrap();

    let mut entry = ComplexEntry::from_fs(path);
    entry.merge(&FsEvent {
        path: path.join("445.txt"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemRemoved,
        id: 0,
    });
    entry.merge(&FsEvent {
        path: path.join("src/template"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemRemoved,
        id: 0,
    });

    let mut expected = complex_entry(path);
    expected.entries.remove(&b"445.txt".to_vec()).unwrap();

    expected
        .entries
        .get_mut(&b"src".to_vec())
        .unwrap()
        .entries
        .remove(&b"template".to_vec())
        .unwrap();

    compare_test_entry(&entry.entry, &expected.entry);
}

#[test]
fn test_on_disk_entry_creating() {
    let tempdir = TempDir::new().unwrap();
    let path = tempdir.path();
    apply_complex_entry(path);

    // Create `foobar.txt`.
    fs::write(path.join("foobar.txt"), b"donoughliu123").unwrap();
    // Create `fook/barm/tmp`.
    fs::create_dir_all(path.join("fook/barm/")).unwrap();
    fs::write(path.join("fook/barm/tmp"), b"1234567890").unwrap();

    let mut entry = ComplexEntry::from_fs(path);
    entry.merge(&FsEvent {
        path: path.join("foobar.txt"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemCreated,
        id: 0,
    });
    entry.merge(&FsEvent {
        path: path.join("fook/barm/tmp"),
        flag: MacEventFlag::kFSEventStreamEventFlagItemCreated,
        id: 0,
    });

    let mut expected = complex_entry(path);
    expected
        .entries
        .insert(b"foobar.txt".to_vec(), entry_file(13));
    let tmp_entry = entry_folder([entry_folder_with_name(
        b"barm",
        [entry_file_with_name(b"tmp", 10)],
    )]);
    expected.entries.insert(b"fook".to_vec(), tmp_entry);

    compare_test_entry(&entry.entry, &expected.entry);
}
