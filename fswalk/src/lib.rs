use rayon::{iter::ParallelBridge, prelude::ParallelIterator};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::{
    fs::{self, Metadata},
    io::{Error, ErrorKind},
    num::NonZeroU64,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::UNIX_EPOCH,
};

#[derive(Serialize, Debug)]
pub struct Node {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    pub name: Box<str>,
    pub metadata: Option<NodeMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct NodeMetadata {
    pub r#type: NodeFileType,
    pub size: u64,
    pub ctime: Option<NonZeroU64>,
    pub mtime: Option<NonZeroU64>,
}

impl From<Metadata> for NodeMetadata {
    fn from(metadata: Metadata) -> Self {
        Self::new(&metadata)
    }
}

impl NodeMetadata {
    fn new(metadata: &Metadata) -> Self {
        let r#type = metadata.file_type().into();
        let size = metadata.size();
        let ctime = metadata
            .created()
            .ok()
            .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
            .and_then(|x| NonZeroU64::new(x.as_secs()));
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
            .and_then(|x| NonZeroU64::new(x.as_secs()));
        Self {
            r#type,
            size,
            ctime,
            mtime,
        }
    }
}

#[derive(Debug, Serialize_repr, Deserialize_repr, Clone, Copy, enumn::N, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeFileType {
    // File occurs a lot, assign it to 0 for better compression ratio(I guess... maybe useful).
    File = 0,
    Dir = 1,
    Symlink = 2,
    Unknown = 3,
}

impl From<fs::FileType> for NodeFileType {
    fn from(file_type: fs::FileType) -> Self {
        if file_type.is_file() {
            NodeFileType::File
        } else if file_type.is_dir() {
            NodeFileType::Dir
        } else if file_type.is_symlink() {
            NodeFileType::Symlink
        } else {
            NodeFileType::Unknown
        }
    }
}

#[derive(Debug)]
pub struct WalkData<'w> {
    pub num_files: AtomicUsize,
    pub num_dirs: AtomicUsize,
    /// Cancellation will be checked periodically.
    cancel: Option<&'w AtomicBool>,
    ignore_directory: Option<&'w Path>,
    /// If set, metadata will be collected for each file node(folder node will get free metadata).
    need_metadata: bool,
}

impl<'w> WalkData<'w> {
    pub const fn simple(need_metadata: bool) -> Self {
        Self {
            num_files: AtomicUsize::new(0),
            num_dirs: AtomicUsize::new(0),
            cancel: None,
            ignore_directory: None,
            need_metadata,
        }
    }

    pub const fn new(
        ignore_directory: Option<&'w Path>,
        need_metadata: bool,
        cancel: Option<&'w AtomicBool>,
    ) -> Self {
        Self {
            num_files: AtomicUsize::new(0),
            num_dirs: AtomicUsize::new(0),
            cancel,
            ignore_directory,
            need_metadata,
        }
    }
}

pub fn walk_it(dir: &Path, walk_data: &WalkData) -> Option<Node> {
    walk(dir, walk_data)
}

static RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap());

fn walk(path: &Path, walk_data: &WalkData) -> Option<Node> {
    if walk_data.ignore_directory == Some(path) {
        return None;
    }
    let metadata = {
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = Arc::clone(&done);
        let path_clone = path.to_path_buf();
        RUNTIME.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5000)).await;
            if !done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::warn!("walking file slow: {:?}", path_clone);
            }
        });
        // doesn't traverse symlink
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => Some(metadata),
            // If it's not found, we definitely don't want it.
            Err(e) if e.kind() == ErrorKind::NotFound => return None,
            // If it's permission denied or something, we still want to insert it into the tree.
            Err(e) => {
                if handle_error_and_retry(&e) {
                    // doesn't traverse symlink
                    path.symlink_metadata().ok()
                } else {
                    None
                }
            }
        };
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        metadata
    };
    let children = if metadata.as_ref().map(|x| x.is_dir()).unwrap_or_default() {
        walk_data.num_dirs.fetch_add(1, Ordering::Relaxed);
        let read_dir = {
            let done = Arc::new(AtomicBool::new(false));
            let done_clone = Arc::clone(&done);
            let path_clone = path.to_path_buf();
            RUNTIME.spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(5000)).await;
                if !done_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!("reading dir slow: {:?}", path_clone);
                }
            });
            let rd = fs::read_dir(path);
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            rd
        };
        match read_dir {
            Ok(entries) => entries
                .into_iter()
                .par_bridge()
                .filter_map(|entry| {
                    match &entry {
                        Ok(entry) => {
                            if walk_data
                                .cancel
                                .map(|x| x.load(Ordering::Relaxed))
                                .unwrap_or_default()
                            {
                                return None;
                            }
                            if walk_data.ignore_directory == Some(path) {
                                return None;
                            }
                            // doesn't traverse symlink
                            if let Ok(data) = entry.file_type() {
                                if data.is_dir() {
                                    return walk(&entry.path(), walk_data);
                                } else {
                                    walk_data.num_files.fetch_add(1, Ordering::Relaxed);
                                    let name = entry
                                        .file_name()
                                        .to_string_lossy()
                                        .into_owned()
                                        .into_boxed_str();
                                    return Some(Node {
                                        children: vec![],
                                        name,
                                        metadata: walk_data
                                            .need_metadata
                                            .then_some(entry)
                                            .and_then(|entry| {
                                                // doesn't traverse symlink
                                                entry.metadata().ok().map(NodeMetadata::from)
                                            }),
                                    });
                                }
                            }
                        }
                        Err(failed) => {
                            if handle_error_and_retry(failed) {
                                return walk(path, walk_data);
                            }
                        }
                    }
                    None
                })
                .collect(),
            Err(failed) => {
                if handle_error_and_retry(&failed) {
                    return walk(path, walk_data);
                } else {
                    vec![]
                }
            }
        }
    } else {
        walk_data.num_files.fetch_add(1, Ordering::Relaxed);
        vec![]
    };
    if walk_data
        .cancel
        .map(|x| x.load(Ordering::Relaxed))
        .unwrap_or_default()
    {
        return None;
    }
    let name = path
        .file_name()
        .map(|x| x.to_string_lossy().into_owned().into_boxed_str())
        .unwrap_or_default();
    Some(Node {
        children,
        name,
        metadata: metadata.map(NodeMetadata::from),
    })
}

fn handle_error_and_retry(failed: &Error) -> bool {
    failed.kind() == std::io::ErrorKind::Interrupted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        time::{Duration, Instant},
    };
    use tempdir::TempDir;

    #[test]
    fn test_walk_simple_tree_without_metadata() {
        let tmp = TempDir::new("fswalk_simple").unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("dir_a")).unwrap();
        fs::File::create(root.join("file_a.txt")).unwrap();
        fs::File::create(root.join("dir_a/file_b.log")).unwrap();
        let walk_data = WalkData::simple(false);
        let node = walk_it(root, &walk_data).unwrap();
        assert_eq!(&*node.name, root.file_name().unwrap().to_str().unwrap());
        // Root + dir + 2 files
        let mut counts = (0, 0);
        fn traverse(n: &Node, counts: &mut (usize, usize)) {
            if n.children.is_empty() {
                counts.0 += 1;
            } else {
                counts.1 += 1;
            }
            for c in &n.children {
                traverse(c, counts);
            }
        }
        traverse(&node, &mut counts);
        assert_eq!(counts.0 + counts.1, 4);
        // Metadata for files should be None (walk_data.need_metadata = false)
        fn assert_no_file_metadata(n: &Node) {
            if n.children.is_empty() {
                assert!(
                    n.metadata.is_none(),
                    "file node metadata should be None when not requested: {:?}",
                    n.name
                );
            } else {
                // directory metadata may be Some (free metadata) but it's optional; ensure type correctness when present
                if let Some(m) = n.metadata {
                    assert!(matches!(m.r#type, NodeFileType::Dir));
                }
                for c in &n.children {
                    assert_no_file_metadata(c);
                }
            }
        }
        assert_no_file_metadata(&node);
    }

    #[test]
    fn test_walk_with_metadata_enabled() {
        let tmp = TempDir::new("fswalk_meta").unwrap();
        let root = tmp.path();
        fs::File::create(root.join("meta_file.txt")).unwrap();
        let walk_data = WalkData::simple(true);
        let node = walk_it(root, &walk_data).unwrap();
        fn find<'a>(node: &'a Node, name: &str) -> Option<&'a Node> {
            if &*node.name == name {
                return Some(node);
            }
            for c in &node.children {
                if let Some(n) = find(c, name) {
                    return Some(n);
                }
            }
            None
        }
        let file_node = find(&node, "meta_file.txt").unwrap();
        assert!(matches!(
            file_node.metadata.map(|m| m.r#type),
            Some(NodeFileType::File)
        ));
    }

    #[test]
    fn test_symlink_not_traversed() {
        let tmp = TempDir::new("fswalk_symlink").unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("real_dir")).unwrap();
        fs::File::create(root.join("real_dir/file.txt")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("real_dir"), root.join("link_dir")).unwrap();
        let walk_data = WalkData::simple(true);
        let node = walk_it(root, &walk_data).unwrap();
        // Ensure link_dir exists as a file system entry but not traversed (should be a file node with no children)
        fn get_child<'a>(n: &'a Node, name: &str) -> Option<&'a Node> {
            n.children.iter().find(|c| &*c.name == name)
        }
        let link = get_child(&node, "link_dir").unwrap();
        assert!(
            link.children.is_empty(),
            "symlink directory should not be traversed"
        );
    }

    #[test]
    fn test_handle_error_and_retry_only_interrupted() {
        let interrupted = Error::from(ErrorKind::Interrupted);
        assert!(handle_error_and_retry(&interrupted));
        let not_found = Error::from(ErrorKind::NotFound);
        assert!(!handle_error_and_retry(&not_found));
    }

    #[test]
    fn test_large_number_of_files_counts() {
        let tmp = TempDir::new("fswalk_many").unwrap();
        let root = tmp.path();
        for i in 0..50u32 {
            let mut f = fs::File::create(root.join(format!("f{i}.txt"))).unwrap();
            writeln!(f, "hello {i}").unwrap();
        }
        let walk_data = WalkData::simple(false);
        let node = walk_it(root, &walk_data).unwrap();
        // Expect 1 (root) + 50 file children
        assert_eq!(
            node.children.len(),
            50,
            "expected 50 files directly under root"
        );
    }

    #[test]
    #[ignore]
    fn test_search_root() {
        let done = AtomicBool::new(false);
        let walk_data = WalkData::new(Some(Path::new("/System/Volumes/Data")), false, None);
        std::thread::scope(|s| {
            s.spawn(|| {
                let node = walk_it(Path::new("/"), &walk_data).unwrap();
                println!("root has {} children", node.children.len());
                done.store(true, Ordering::Relaxed);
            });
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let files = walk_data.num_files.load(Ordering::Relaxed);
                    let dirs = walk_data.num_dirs.load(Ordering::Relaxed);
                    println!("so far: {files} files, {dirs} dirs");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });
        });
    }

    #[test]
    #[ignore]
    fn test_search_simulator() {
        let done = AtomicBool::new(false);
        let walk_data = WalkData::new(Some(Path::new("/System/Volumes/Data")), true, None);
        std::thread::scope(|s| {
            s.spawn(|| {
                let node = walk_it(
                    Path::new("/Library/Developer/CoreSimulator/Volumes/iOS_23A343"),
                    &walk_data,
                )
                .unwrap();
                println!("sim has {} children", node.children.len());
                done.store(true, Ordering::Relaxed);
            });
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    let files = walk_data.num_files.load(Ordering::Relaxed);
                    let dirs = walk_data.num_dirs.load(Ordering::Relaxed);
                    println!("so far: {files} files, {dirs} dirs");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });
        });
    }

    #[test]
    fn test_search_cancel() {
        let cancel = AtomicBool::new(false);
        let done = AtomicBool::new(false);
        let walk_data = WalkData::new(
            Some(Path::new("/System/Volumes/Data")),
            false,
            Some(&cancel),
        );
        std::thread::scope(|s| {
            s.spawn(|| {
                let node = walk_it(Path::new("/"), &walk_data);
                done.store(true, Ordering::Relaxed);
                assert!(node.is_none(), "expected walk to be cancelled");
            });
            s.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let time = Instant::now();
                cancel.store(true, Ordering::Relaxed);
                while !done.load(Ordering::Relaxed) {
                    std::thread::yield_now();
                }
                // Ensure cancellation happened quickly
                dbg!(time.elapsed());
                assert!(time.elapsed() < Duration::from_secs(1));
            });
        });
    }
}
