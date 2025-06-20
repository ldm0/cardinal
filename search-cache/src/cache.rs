use crate::persistent::{read_cache_from_file, write_cache_to_file, PersistentStorage};
use anyhow::{anyhow, bail, Context, Result};
use bincode::{Decode, Encode};
use cardinal_sdk::{current_event_id, EventFlag, FsEvent, ScanType};
use fswalk::{walk_it, Node, NodeMetadata, WalkData};
use namepool::NamePool;
use query_segmentation::{query_segmentation, Segment};
use serde::{Deserialize, Serialize};
use slab::Slab;
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr},
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Instant,
};
use typed_num::Num;

#[derive(Debug, Serialize, Deserialize, Encode, Decode)]
pub struct SlabNode {
    parent: Option<usize>,
    children: Vec<usize>,
    name: String,
    metadata: Option<SlabNodeMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Encode, Decode, Clone, Copy)]
pub struct SlabNodeMetadata {
    pub ctime: Option<u64>,
    pub mtime: Option<u64>,
    pub size: u64,
}

impl SlabNodeMetadata {
    fn new(metadata: &NodeMetadata) -> Self {
        Self {
            ctime: metadata.ctime,
            mtime: metadata.mtime,
            size: metadata.size,
        }
    }
}

pub struct SearchCache {
    path: PathBuf,
    last_event_id: u64,
    slab_root: usize,
    slab: Slab<SlabNode>,
    name_index: BTreeMap<String, Vec<usize>>,
    name_pool: NamePool,
}

impl std::fmt::Debug for SearchCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchCache")
            .field("path", &self.path)
            .field("last_event_id", &self.last_event_id)
            .field("slab_root", &self.slab_root)
            .field("slab.len()", &self.slab.len())
            .field("name_index.len()", &self.name_index.len())
            .finish()
    }
}

#[derive(Debug)]
pub struct SearchNode {
    pub path: PathBuf,
    pub metadata: Option<SlabNodeMetadata>,
}

impl SearchCache {
    /// The `path` is the root path of the constructed cache and fsevent watch path.
    pub fn try_read_persistent_cache(path: &Path) -> Result<Self> {
        let last_event_id = current_event_id();
        read_cache_from_file()
            .and_then(|x| {
                (x.path == path)
                    .then(|| ())
                    .ok_or_else(|| {
                        anyhow!(
                            "Inconsistent root path: expected: {:?}, actual: {:?}",
                            path,
                            &x.path
                        )
                    })
                    .map(|()| x)
            })
            .map(
                |PersistentStorage {
                     path,
                     slab_root,
                     slab,
                     name_index,
                     ..
                 }| Self::new(path, last_event_id, slab_root, slab, name_index),
            )
    }

    pub fn walk_fs(path: PathBuf) -> Self {
        fn walkfs_to_slab(path: &Path) -> (usize, Slab<SlabNode>) {
            // 先多线程构建树形文件名列表(不能直接创建 slab 因为 slab 无法多线程构建(slab 节点有相互引用，不想加锁))
            let walk_data = WalkData::new(PathBuf::from("/System/Volumes/Data"), false);
            let visit_time = Instant::now();
            let node = walk_it(path, &walk_data).expect("failed to walk");
            dbg!(walk_data);
            dbg!(visit_time.elapsed());

            // 然后创建 slab
            let slab_time = Instant::now();
            let mut slab = Slab::new();
            let slab_root = construct_node_slab(None, &node, &mut slab);
            dbg!(slab_time.elapsed());
            dbg!(slab_root);
            dbg!(slab.len());

            (slab_root, slab)
        }
        fn name_index(slab: &Slab<SlabNode>) -> BTreeMap<String, Vec<usize>> {
            // TODO(ldm0): Memory optimization can be done by letting name index reference the name in the pool(gc need to be considered though)
            fn construct_name_index(
                slab: &Slab<SlabNode>,
                name_index: &mut BTreeMap<String, Vec<usize>>,
            ) {
                // The slab is newly constructed, thus though slab.iter() iterates all slots, it won't waste too much.
                for (i, node) in slab.iter() {
                    if let Some(nodes) = name_index.get_mut(&node.name) {
                        nodes.push(i);
                    } else {
                        name_index.insert(node.name.clone(), vec![i]);
                    };
                }
            }

            let name_index_time = Instant::now();
            let mut name_index = BTreeMap::default();
            construct_name_index(slab, &mut name_index);
            dbg!(name_index_time.elapsed());
            println!("name index len: {}", name_index.len());
            name_index
        }

        let last_event_id = current_event_id();
        let (slab_root, slab) = walkfs_to_slab(&path);
        let name_index = name_index(&slab);
        Self::new(path, last_event_id, slab_root, slab, name_index)
    }

    pub fn new(
        path: PathBuf,
        last_event_id: u64,
        slab_root: usize,
        slab: Slab<SlabNode>,
        name_index: BTreeMap<String, Vec<usize>>,
    ) -> Self {
        // name pool construction speed is fast enough that caching it doesn't worth it.
        let name_pool = name_pool(&name_index);
        Self {
            path,
            last_event_id,
            slab_root,
            slab,
            name_index,
            name_pool,
        }
    }

    pub fn search(&self, line: &str) -> Result<Vec<usize>> {
        let segments = query_segmentation(line);
        if segments.is_empty() {
            bail!("unprocessed query: {:?}", segments);
        }
        let search_time = Instant::now();
        let mut node_set: Option<Vec<usize>> = None;
        for segment in &segments {
            if let Some(nodes) = &node_set {
                let mut new_node_set = Vec::with_capacity(nodes.len());
                for &node in nodes {
                    let childs = &self.slab[node].children;
                    for child in childs {
                        if match segment {
                            Segment::Substr(substr) => self.slab[*child].name.contains(*substr),
                            Segment::Prefix(prefix) => self.slab[*child].name.starts_with(*prefix),
                            Segment::Exact(exact) => self.slab[*child].name == *exact,
                            Segment::Suffix(suffix) => self.slab[*child].name.ends_with(*suffix),
                        } {
                            new_node_set.push(*child);
                        }
                    }
                }
                node_set = Some(new_node_set);
            } else {
                let names: Vec<_> = match segment {
                    Segment::Substr(substr) => self.name_pool.search_substr(substr).collect(),
                    Segment::Prefix(prefix) => {
                        let mut buffer = Vec::with_capacity(prefix.len() + 1);
                        buffer.push(0);
                        buffer.extend_from_slice(prefix.as_bytes());
                        self.name_pool.search_prefix(&buffer).collect()
                    }
                    Segment::Exact(exact) => {
                        let mut buffer = Vec::with_capacity(exact.len() + 2);
                        buffer.push(0);
                        buffer.extend_from_slice(exact.as_bytes());
                        buffer.push(0);
                        self.name_pool.search_exact(&buffer).collect()
                    }
                    Segment::Suffix(suffix) => {
                        // Query contains nul is very rare
                        let suffix = CString::new(*suffix).expect("Query contains nul");
                        self.name_pool.search_suffix(&suffix).collect()
                    }
                };
                let mut nodes = Vec::with_capacity(names.len());
                for name in names {
                    // namepool doesn't shrink, so it can contains non-existng names. Therefore, we don't error out on None branch here.
                    if let Some(x) = self.name_index.get(name) {
                        nodes.extend_from_slice(x);
                    }
                }
                // name_pool doesn't dedup, so we need to dedup the results here.
                nodes.sort_unstable();
                nodes.dedup();
                node_set = Some(nodes);
            }
        }
        let search_time = search_time.elapsed();
        dbg!(search_time);
        // Safety: node_set can't be None since segments is not empty.
        Ok(node_set.unwrap())
    }

    /// Get the path of the node in the slab.
    pub fn node_path(&self, index: usize) -> PathBuf {
        let mut current = index;
        let mut segments = vec![];
        while let Some(parent) = self.slab[current].parent {
            segments.push(self.slab[current].name.clone());
            current = parent;
        }
        self.path
            .iter()
            .chain(segments.iter().rev().map(OsStr::new))
            .collect()
    }

    fn push_node(&mut self, node: SlabNode) -> usize {
        let node_name = node.name.clone();
        let index = self.slab.insert(node);
        if let Some(indexes) = self.name_index.get_mut(&node_name) {
            indexes.push(index);
        } else {
            self.name_pool.push(&node_name);
            self.name_index.insert(node_name, vec![index]);
        }
        index
    }

    /// Removes a node by path and its children recursively.
    fn remove_node_path(&mut self, path: &Path) -> Option<usize> {
        let mut current = self.slab_root;
        for name in path
            .components()
            .map(|x| x.as_os_str().to_string_lossy().into_owned())
        {
            if let Some(&index) = self.slab[current]
                .children
                .iter()
                .find(|&&x| self.slab[x].name == name)
            {
                current = index;
            } else {
                return None;
            }
        }
        self.remove_node(current);
        Some(current)
    }

    // Blindly try create node chain, it doesn't check if the path is really exist on disk.
    fn create_node_chain(&mut self, path: &Path) -> usize {
        let mut current = self.slab_root;
        for name in path
            .components()
            .map(|x| x.as_os_str().to_string_lossy().into_owned())
        {
            current = if let Some(&index) = self.slab[current]
                .children
                .iter()
                .find(|&&x| self.slab[x].name == name)
            {
                index
            } else {
                // TODO(ldm0): optimize: slab node children is empty, we can create a node chain directly.
                let node = SlabNode {
                    parent: Some(current),
                    children: vec![],
                    name,
                    metadata: None,
                };
                let index = self.push_node(node);
                self.slab[current].children.push(index);
                index
            };
        }
        current
    }

    // `Self::scan_path_recursive`function returns index of the constructed node(with metadata provided).
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    pub fn scan_path_recursive(&mut self, raw_path: &Path) -> Option<usize> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(&self.path) else {
            return None;
        };
        if raw_path.metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        let Some(parent) = path.parent() else {
            println!("!!! Full rescan is needed");
            // If path is watch root, we need to do a full rescan
            self.rescan();
            println!("!!! Full rescan done: {:?}", self.slab_root);
            return Some(self.slab_root);
        };
        // Ensure node of the path parent is existed
        let parent = self.create_node_chain(parent);
        // Remove node(if exists) and do a full rescan
        if let Some(&old_node) = self.slab[parent]
            .children
            .iter()
            .find(|&&x| path.file_name() == Some(OsStr::new(&self.slab[x].name)))
        {
            self.remove_node(old_node);
        }
        // For incremental data, we need metadata
        let walk_data = WalkData::new(PathBuf::from("/System/Volumes/Data"), true);
        walk_it(raw_path, &walk_data).map(|node| {
            let node = create_node_slab_update_name_index_and_name_pool(
                Some(parent),
                &node,
                &mut self.slab,
                &mut self.name_index,
                &mut self.name_pool,
            );
            // Push the newly created node to the parent's children
            self.slab[parent].children.push(node);
            node
        })
    }

    // `Self::scan_path_nonrecursive`function returns index of the constructed node.
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    #[allow(dead_code)]
    fn scan_path_nonrecursive(&mut self, raw_path: &Path) -> Option<usize> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(&self.path) else {
            return None;
        };
        if raw_path.metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        Some(self.create_node_chain(path))
    }

    pub fn rescan(&mut self) {
        // Remove all memory consuming cache
        self.slab = Slab::new();
        self.name_index = BTreeMap::default();
        self.name_pool = NamePool::new();
        let path = std::mem::take(&mut self.path);
        *self = Self::walk_fs(path);
    }

    /// Removes a node and its children recursively by index.
    fn remove_node(&mut self, index: usize) {
        fn remove_single_node(cache: &mut SearchCache, index: usize) {
            if let Some(node) = cache.slab.try_remove(index) {
                let indexes = cache
                    .name_index
                    .get_mut(&node.name)
                    .expect("inconsistent name index and node");
                indexes.retain(|&x| x != index);
                if indexes.is_empty() {
                    cache.name_index.remove(&node.name);
                    // TODO(ldm0): actually we need to remove name in the name pool,
                    // but currently name pool doesn't support remove. (GC is needed for name pool)
                    // self.name_pool.remove(&node.name);
                }
            }
        }

        // Remove parent reference, make whole subtree unreachable.
        if let Some(parent) = self.slab[index].parent {
            self.slab[parent].children.retain(|&x| x != index);
        }
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            stack.extend_from_slice(&self.slab[current].children);
            remove_single_node(self, current);
        }
    }

    pub fn flush_to_file(self) -> Result<()> {
        write_cache_to_file(PersistentStorage {
            version: Num,
            path: self.path,
            slab_root: self.slab_root,
            slab: self.slab,
            name_index: self.name_index,
        })
        .context("Write cache to file failed.")
    }

    fn update_last_event_id(&mut self, event_id: u64) {
        if event_id <= self.last_event_id {
            eprintln!("Event id is not increasing, ignoring");
            return;
        }
        self.last_event_id = event_id;
    }

    pub fn last_event_id(&mut self) -> u64 {
        self.last_event_id
    }

    pub fn query_files(&self, query: String) -> Result<Vec<SearchNode>> {
        self.search(&query).map(|nodes| {
            nodes
                .into_iter()
                .map(|node| SearchNode {
                    path: self.node_path(node),
                    metadata: self.slab[node].metadata.clone(),
                })
                .collect()
        })
    }

    fn handle_fs_event(&mut self, event: FsEvent) {
        match event.flag.scan_type() {
            ScanType::SingleNode => {
                // TODO(ldm0): use scan_path_nonrecursive until we are confident about each event flag meaning.
                let file = self.scan_path_recursive(&event.path);
                if file.is_some() {
                    println!("File changed: {:?}, {file:?}", event.path);
                }
            }
            ScanType::Folder => {
                let folder = self.scan_path_recursive(&event.path);
                if folder.is_some() {
                    println!("Folder changed: {:?}, {folder:?}", event.path);
                }
            }
            ScanType::ReScan => {
                println!("!!! Rescanning");
                self.rescan();
                println!("!!! Rescan done: {:?}", self.slab_root);
            }
            ScanType::Nop => {}
        }
        self.update_last_event_id(event.id);
    }

    pub fn handle_fs_events(&mut self, events: Vec<FsEvent>) {
        for event in events {
            if event.flag.contains(EventFlag::HistoryDone) {
                println!("History processing done: {:?}", event);
                continue;
            }
            self.handle_fs_event(event);
        }
    }
}

fn construct_node_slab(parent: Option<usize>, node: &Node, slab: &mut Slab<SlabNode>) -> usize {
    let slab_node = SlabNode {
        parent,
        children: vec![],
        name: node.name.clone(),
        metadata: node.metadata.as_ref().map(SlabNodeMetadata::new),
    };
    let index = slab.insert(slab_node);
    slab[index].children = node
        .children
        .iter()
        .map(|node| construct_node_slab(Some(index), node, slab))
        .collect();
    index
}

/// ATTENTION: This function doesn't remove existing node.
fn create_node_slab_update_name_index_and_name_pool(
    parent: Option<usize>,
    node: &Node,
    slab: &mut Slab<SlabNode>,
    name_index: &mut BTreeMap<String, Vec<usize>>,
    name_pool: &mut NamePool,
) -> usize {
    let slab_node = SlabNode {
        parent,
        children: vec![],
        name: node.name.clone(),
        metadata: node.metadata.as_ref().map(SlabNodeMetadata::new),
    };
    let index = slab.insert(slab_node);
    if let Some(indexes) = name_index.get_mut(&node.name) {
        // TODO(ldm0): optimize to binary search?
        if !indexes.iter().any(|&x| x == index) {
            indexes.push(index);
        }
    } else {
        name_pool.push(&node.name);
        name_index.insert(node.name.clone(), vec![index]);
    }
    slab[index].children = node
        .children
        .iter()
        .map(|node| {
            create_node_slab_update_name_index_and_name_pool(
                Some(index),
                node,
                slab,
                name_index,
                name_pool,
            )
        })
        .collect();
    index
}

fn name_pool(name_index: &BTreeMap<String, Vec<usize>>) -> NamePool {
    let name_pool_time = Instant::now();
    let mut name_pool = NamePool::new();
    for name in name_index.keys() {
        name_pool.push(name);
    }
    dbg!(name_pool_time.elapsed());
    println!(
        "name pool size: {}MB",
        name_pool.len() as f32 / 1024. / 1024.
    );
    name_pool
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempdir::TempDir;

    #[test]
    fn test_search_cache_walk_and_verify() {
        let temp_dir = TempDir::new("test_cache").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        fs::create_dir_all(temp_path.join("subdir")).expect("Failed to create subdirectory");
        fs::File::create(temp_path.join("file1.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("subdir/file2.txt")).expect("Failed to create file");

        let cache = SearchCache::walk_fs(temp_path.to_path_buf());

        assert_eq!(cache.slab.len(), 4);
        assert_eq!(cache.name_index.len(), 4);
        assert_eq!(
            cache.name_pool.len(),
            temp_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .as_bytes()
                .len()
                + b"subdir".len()
                + b"file1.txt".len()
                + b"file2.txt".len()
                + 5 * b"\0".len()
        );
    }

    #[test]
    fn test_handle_fs_event_add() {
        // 创建临时文件夹
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_add_before_search() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    // Processing outdated fs event is required to avoid bouncing.
    #[test]
    fn test_handle_outdated_fs_event() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id - 1,
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_removal() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);

        fs::remove_file(temp_path.join("new_file.txt")).expect("Failed to remove file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved,
        }];

        cache.handle_fs_events(mock_events);

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 0);
    }

    #[test]
    fn test_handle_fs_event_removal_fake() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved,
        }];

        cache.handle_fs_events(mock_events);

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_add_and_removal() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![
            FsEvent {
                path: temp_path.join("new_file.txt"),
                id: cache.last_event_id + 1,
                flag: EventFlag::ItemCreated,
            },
            FsEvent {
                path: temp_path.join("new_file.txt"),
                id: cache.last_event_id + 1,
                flag: EventFlag::ItemRemoved,
            },
        ];

        cache.handle_fs_events(mock_events);

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.slab.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_rescan0() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file2.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file3.txt")).expect("Failed to create file");
        fs::create_dir_all(temp_path.join("src/foo")).expect("Failed to create dir");
        fs::File::create(temp_path.join("src/foo/good.rs")).expect("Failed to create file");
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 7);
        assert_eq!(cache.name_index.len(), 7);

        let mock_events = vec![FsEvent {
            path: temp_path.to_path_buf(),
            id: cache.last_event_id + 1,
            flag: EventFlag::RootChanged,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 7);
        assert_eq!(cache.name_index.len(), 7);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 1);
        assert_eq!(cache.search("foo").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_rescan1() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file2.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file3.txt")).expect("Failed to create file");
        fs::create_dir_all(temp_path.join("src/foo")).expect("Failed to create dir");
        fs::File::create(temp_path.join("src/foo/good.rs")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.to_path_buf(),
            id: cache.last_event_id + 1,
            flag: EventFlag::RootChanged,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 7);
        assert_eq!(cache.name_index.len(), 7);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 1);
        assert_eq!(cache.search("foo").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_rescan_by_modify() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file2.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file3.txt")).expect("Failed to create file");
        fs::create_dir_all(temp_path.join("src/foo")).expect("Failed to create dir");
        fs::File::create(temp_path.join("src/foo/good.rs")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.to_path_buf(),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemModified,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 7);
        assert_eq!(cache.name_index.len(), 7);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 1);
        assert_eq!(cache.search("foo").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_dir_removal0() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::create_dir_all(temp_path.join("Cargo.toml")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file2.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file3.txt")).expect("Failed to create file");
        fs::create_dir_all(temp_path.join("src/foo")).expect("Failed to create dir");
        fs::File::create(temp_path.join("src/foo/good.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/foo.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/lib.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/boo.rs")).expect("Failed to create file");
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 11);
        assert_eq!(cache.name_index.len(), 11);
        assert_eq!(cache.search("src").unwrap().len(), 1);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 1);
        assert_eq!(cache.search("foo").unwrap().len(), 2);
        assert_eq!(cache.search("oo.rs/").unwrap().len(), 2);
        assert_eq!(cache.search("oo").unwrap().len(), 4);

        fs::remove_dir_all(temp_path.join("src")).expect("Failed to remove dir");

        let mock_events = vec![FsEvent {
            path: temp_path.join("src"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved | EventFlag::ItemIsDir,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 5);
        assert_eq!(cache.name_index.len(), 5);
        assert_eq!(cache.search("src").unwrap().len(), 0);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 0);
        assert_eq!(cache.search("foo").unwrap().len(), 0);
        assert_eq!(cache.search("/foo").unwrap().len(), 0);
    }

    #[test]
    fn test_handle_fs_event_dir_removal_triggered_by_subdir_event() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::create_dir_all(temp_path.join("Cargo.toml")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file2.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("new_file3.txt")).expect("Failed to create file");
        fs::create_dir_all(temp_path.join("src/foo")).expect("Failed to create dir");
        fs::File::create(temp_path.join("src/foo/good.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/foo.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/lib.rs")).expect("Failed to create file");
        fs::File::create(temp_path.join("src/boo.rs")).expect("Failed to create file");
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.slab.len(), 11);
        assert_eq!(cache.name_index.len(), 11);
        assert_eq!(cache.search("src").unwrap().len(), 1);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 1);
        assert_eq!(cache.search("foo").unwrap().len(), 2);
        assert_eq!(cache.search("oo.rs/").unwrap().len(), 2);
        assert_eq!(cache.search("oo").unwrap().len(), 4);

        fs::remove_dir_all(temp_path.join("src")).expect("Failed to remove dir");

        let mock_events = vec![FsEvent {
            path: temp_path.join("src/foo"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved | EventFlag::ItemIsDir,
        }];

        cache.handle_fs_events(mock_events);

        assert_eq!(cache.slab.len(), 9);
        assert_eq!(cache.name_index.len(), 9);
        assert_eq!(cache.search("src").unwrap().len(), 1);
        assert_eq!(cache.search("new_file").unwrap().len(), 3);
        assert_eq!(cache.search("good.rs").unwrap().len(), 0);
        assert_eq!(cache.search("foo").unwrap().len(), 1);
        assert_eq!(cache.search("/foo").unwrap().len(), 1);
        assert_eq!(cache.search("oo.rs/").unwrap().len(), 2);
        assert_eq!(cache.search("oo").unwrap().len(), 2);
    }

    #[test]
    fn test_walk_fs_metadata_is_always_none() {
        let temp_dir = TempDir::new("test_walk_fs_meta").expect("Failed to create temp directory");
        let root_path = temp_dir.path();

        fs::File::create(&root_path.join("file1.txt")).expect("Failed to create file1.txt");
        fs::create_dir(&root_path.join("subdir1")).expect("Failed to create subdir1");
        fs::File::create(&root_path.join("subdir1/file2.txt")).expect("Failed to create file1.txt");

        let cache = SearchCache::walk_fs(root_path.to_path_buf());

        // 目录的 metadata 都会是 Some()
        assert!(cache.slab[cache.slab_root].metadata.is_some());

        // Check metadata for a file node
        let file_nodes = cache
            .search("file1.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1, "Expected 1 node for file1.txt");
        let file_node_idx = file_nodes[0];
        // 文件的 metadata 都会是 None
        assert!(
            cache.slab[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs should be None"
        );

        // Check metadata for a file node
        let file_nodes = cache
            .search("file2.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1);
        let file_node_idx = file_nodes[0];
        // 文件的 metadata 都会是 None
        assert!(
            cache.slab[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs should be None"
        );

        // Check metadata for a subdirectory node
        let dir_nodes = cache.search("subdir1").expect("Search for subdir1 failed");
        assert_eq!(dir_nodes.len(), 1, "Expected 1 node for subdir1");
        let dir_node_idx = dir_nodes[0];
        // 目录的 metadata 都会是 Some()
        assert!(
            cache.slab[dir_node_idx].metadata.is_some(),
            "Metadata for directory node created by walk_fs should be None"
        );
    }

    #[test]
    fn test_handle_fs_events_metadata() {
        let temp_dir = TempDir::new("test_event_meta").expect("Failed to create temp directory");
        let root_path = temp_dir.path();

        fs::File::create(&root_path.join("file1.txt")).expect("Failed to create file1.txt");
        fs::create_dir(&root_path.join("subdir1")).expect("Failed to create subdir1");
        fs::File::create(&root_path.join("subdir1/file2.txt")).expect("Failed to create file1.txt");

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());
        let mut last_event_id = cache.last_event_id();

        let new_file_path = root_path.join("event_file.txt");
        fs::write(&new_file_path, b"heck").expect("Failed to create event_file.txt");

        let new_file_meta_on_disk = fs::metadata(&new_file_path).unwrap();
        last_event_id += 1;

        let file_event = FsEvent {
            path: new_file_path.clone(),
            id: last_event_id,
            flag: EventFlag::ItemCreated,
        };
        cache.handle_fs_events(vec![file_event]);

        let file_nodes = cache
            .search("event_file.txt")
            .expect("Search for event_file.txt failed");
        assert_eq!(
            file_nodes.len(),
            1,
            "Expected 1 node for event_file.txt after event"
        );
        let file_node_idx = file_nodes[0];
        let file_slab_meta = cache.slab[file_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for event_file.txt should be populated by event handler");
        assert_eq!(
            file_slab_meta.size,
            new_file_meta_on_disk.len(),
            "Size mismatch for event_file.txt"
        );
        assert_eq!(file_slab_meta.size, 4, "Size mismatch for event_file.txt");
        assert!(
            file_slab_meta.mtime.is_some(),
            "mtime should be populated for event_file.txt"
        );

        // Part 2: Event for a newly created directory (should populate metadata for itself and its children)
        let new_subdir_path = root_path.join("event_subdir");
        fs::create_dir(&new_subdir_path).expect("Failed to create event_subdir");

        let file_in_subdir_path = new_subdir_path.join("file_in_event_subdir.txt");
        fs::File::create(&file_in_subdir_path).expect("Failed to create file_in_event_subdir.txt");
        let file_in_subdir_meta_on_disk = fs::metadata(&file_in_subdir_path).unwrap();
        last_event_id += 1;

        let dir_event = FsEvent {
            path: new_subdir_path.clone(), // Event is for the directory
            id: last_event_id,
            flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
        };
        cache.handle_fs_events(vec![dir_event]);

        // Check metadata for the directory itself
        let dir_nodes = cache
            .search("/event_subdir/")
            .expect("Search for event_subdir failed");
        assert_eq!(
            dir_nodes.len(),
            1,
            "Expected 1 node for event_subdir after event"
        );
        let dir_node_idx = dir_nodes[0];
        let dir_slab_meta = cache.slab[dir_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for event_subdir should be populated by event handler");
        assert!(
            dir_slab_meta.mtime.is_some(),
            "mtime should be populated for event_subdir"
        );

        // Check metadata for the file inside the directory
        let file_in_subdir_nodes = cache
            .search("file_in_event_subdir.txt")
            .expect("Search for file_in_event_subdir.txt failed");
        assert_eq!(
            file_in_subdir_nodes.len(),
            1,
            "Expected 1 node for file_in_event_subdir.txt after event"
        );
        let file_in_subdir_node_idx = file_in_subdir_nodes[0];
        let file_in_subdir_slab_meta = cache.slab[file_in_subdir_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for file_in_event_subdir.txt should be populated");
        assert_eq!(
            file_in_subdir_slab_meta.size,
            file_in_subdir_meta_on_disk.len(),
            "Size mismatch for file_in_event_subdir.txt"
        );
        assert!(
            file_in_subdir_slab_meta.mtime.is_some(),
            "mtime should be populated for file_in_event_subdir.txt"
        );
    }

    #[test]
    fn test_query_files_basic_and_no_results() {
        let temp_dir = TempDir::new("test_query_files_basic").unwrap();
        let root_path = temp_dir.path();

        fs::File::create(root_path.join("file_a.txt")).unwrap();
        fs::create_dir(root_path.join("dir_b")).unwrap();
        fs::File::create(root_path.join("dir_b/file_c.md")).unwrap();

        let cache = SearchCache::walk_fs(root_path.to_path_buf());

        // 1. Query for a specific file
        let results1 = cache.query_files("file_a.txt".to_string()).unwrap();
        assert_eq!(results1.len(), 1);
        assert!(
            results1[0].path.ends_with("file_a.txt"),
            "Path was: {:?}",
            results1[0].path
        );
        assert!(
            results1[0].metadata.is_none(),
            "File metadata should be None after walk_fs"
        );

        // 2. Query for a file in a subdirectory
        let results2 = cache.query_files("file_c.md".to_string()).unwrap();
        assert_eq!(results2.len(), 1);
        assert!(
            results2[0].path.ends_with("dir_b/file_c.md"),
            "Path was: {:?}",
            results2[0].path
        );
        assert!(results2[0].metadata.is_none());

        // 3. Query for a directory
        let results3 = cache.query_files("dir_b".to_string()).unwrap();
        assert_eq!(results3.len(), 1);
        assert!(
            results3[0].path.ends_with("dir_b"),
            "Path was: {:?}",
            results3[0].path
        );
        assert!(
            results3[0].metadata.is_some(),
            "Directory metadata should be Some after walk_fs"
        );

        // 4. Query with no results
        let results4 = cache.query_files("non_existent.zip".to_string()).unwrap();
        assert_eq!(results4.len(), 0);
    }

    #[test]
    fn test_query_files_multiple_matches_and_segments() {
        let temp_dir = TempDir::new("test_query_files_multi").unwrap();
        let root_path = temp_dir.path();

        fs::File::create(root_path.join("file_a.txt")).unwrap();
        fs::File::create(root_path.join("another_file_a.log")).unwrap();
        fs::create_dir(root_path.join("dir_b")).unwrap();
        fs::File::create(root_path.join("dir_b/file_c.md")).unwrap();

        let cache = SearchCache::walk_fs(root_path.to_path_buf());

        // 5. Query matching multiple files (substring)
        let results5 = cache.query_files("file_a".to_string()).unwrap();
        assert_eq!(
            results5.len(),
            2,
            "Expected to find 'file_a.txt' and 'another_file_a.log'"
        );
        let paths5: Vec<_> = results5.iter().map(|r| r.path.clone()).collect();
        assert!(paths5.iter().any(|p| p.ends_with("file_a.txt")));
        assert!(paths5.iter().any(|p| p.ends_with("another_file_a.log")));

        // 6. Query with multiple segments (path-like search)
        // "dir_b/file_c" should find "dir_b/file_c.md"
        let results6 = cache.query_files("dir_b/file_c".to_string()).unwrap();
        assert_eq!(results6.len(), 1);
        assert!(
            results6[0].path.ends_with("dir_b/file_c.md"),
            "Path was: {:?}",
            results6[0].path
        );
    }

    #[test]
    fn test_query_files_root_directory() {
        let temp_dir = TempDir::new("test_query_files_root").unwrap();
        let root_path = temp_dir.path();
        fs::File::create(root_path.join("some_file.txt")).unwrap(); // Add a file to make cache non-trivial

        let cache = SearchCache::walk_fs(root_path.to_path_buf());
        let root_dir_name = root_path.file_name().unwrap().to_str().unwrap();

        let results = cache.query_files(root_dir_name.to_string()).unwrap();
        assert_eq!(results.len(), 1, "Should find the root directory itself");
        let expected_path = root_path;
        assert_eq!(
            results[0].path, expected_path,
            "Path for root query mismatch. Expected: {:?}, Got: {:?}",
            expected_path, results[0].path
        );
        assert!(
            results[0].metadata.is_some(),
            "Root directory metadata should be Some"
        );
    }

    #[test]
    fn test_query_files_empty_query_string() {
        let temp_dir = TempDir::new("test_query_files_empty_q").unwrap();
        let cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());
        // query_segmentation("") returns empty vec, search() then bails.
        let result = cache.query_files("".to_string());
        assert!(
            result.is_err(),
            "Empty query string should result in an error"
        );
    }

    #[test]
    fn test_query_files_deep_path_construction_and_multi_segment() {
        let temp_dir = TempDir::new("test_query_deep_path").unwrap();
        let root = temp_dir.path();
        let sub1 = root.join("alpha_dir");
        let sub2 = sub1.join("beta_subdir");
        let file_in_sub2 = sub2.join("gamma_file.txt");

        fs::create_dir_all(&sub2).unwrap();
        fs::File::create(&file_in_sub2).unwrap();

        let cache = SearchCache::walk_fs(root.to_path_buf());

        // Query for the deep file directly
        let results_deep_file = cache.query_files("gamma_file.txt".to_string()).unwrap();
        assert_eq!(results_deep_file.len(), 1);
        let expected_suffix_deep = format!("alpha_dir/beta_subdir/gamma_file.txt");
        assert!(
            results_deep_file[0].path.ends_with(&expected_suffix_deep),
            "Path was: {:?}",
            results_deep_file[0].path
        );

        // Query for intermediate directory
        let results_sub1 = cache.query_files("alpha_dir".to_string()).unwrap();
        assert_eq!(results_sub1.len(), 1);
        assert!(
            results_sub1[0].path.ends_with("alpha_dir"),
            "Path was: {:?}",
            results_sub1[0].path
        );

        // Query for nested intermediate directory
        let results_sub2 = cache.query_files("beta_subdir".to_string()).unwrap();
        assert_eq!(results_sub2.len(), 1);
        assert!(
            results_sub2[0].path.ends_with("alpha_dir/beta_subdir"),
            "Path was: {:?}",
            results_sub2[0].path
        );

        // Test multi-segment query for the deep file
        let results_multi_segment = cache
            .query_files("alpha_dir/beta_subdir/gamma_file".to_string())
            .unwrap();
        assert_eq!(results_multi_segment.len(), 1);
        assert!(
            results_multi_segment[0]
                .path
                .ends_with(&expected_suffix_deep),
            "Path was: {:?}",
            results_multi_segment[0].path
        );

        // Test multi-segment query for an intermediate directory
        let results_multi_segment_dir = cache
            .query_files("alpha_dir/beta_subdir".to_string())
            .unwrap();
        assert_eq!(results_multi_segment_dir.len(), 1);
        assert!(
            results_multi_segment_dir[0]
                .path
                .ends_with("alpha_dir/beta_subdir"),
            "Path was: {:?}",
            results_multi_segment_dir[0].path
        );
    }

    #[test]
    fn test_query_files_metadata_consistency_after_walk_and_event() {
        let temp_dir = TempDir::new("test_query_meta_consistency").unwrap();
        let root_path = temp_dir.path();

        let file_path_walk = root_path.join("walk_file.txt");
        let dir_path_walk = root_path.join("walk_dir");
        fs::File::create(&file_path_walk).unwrap();
        fs::create_dir(&dir_path_walk).unwrap();

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());

        // Check metadata from initial walk_fs
        let results_file_walk = cache.query_files("walk_file.txt".to_string()).unwrap();
        assert_eq!(results_file_walk.len(), 1);
        assert!(
            results_file_walk[0].metadata.is_none(),
            "File metadata from walk_fs should be None"
        );

        let results_dir_walk = cache.query_files("walk_dir".to_string()).unwrap();
        assert_eq!(results_dir_walk.len(), 1);
        assert!(
            results_dir_walk[0].metadata.is_some(),
            "Directory metadata from walk_fs should be Some"
        );

        // Simulate an event for a new file
        let event_file_path = root_path.join("event_added_file.txt");
        fs::write(&event_file_path, "content123").unwrap(); // content of size 10
        let last_event_id = cache.last_event_id();
        let event = FsEvent {
            path: event_file_path.clone(),
            id: last_event_id + 1,
            flag: EventFlag::ItemCreated,
        };
        cache.handle_fs_events(vec![event]);

        let results_event_file = cache
            .query_files("event_added_file.txt".to_string())
            .unwrap();
        assert_eq!(results_event_file.len(), 1);
        let event_file_meta = results_event_file[0]
            .metadata
            .as_ref()
            .expect("File metadata should be Some after event processing");
        assert_eq!(event_file_meta.size, 10);

        // Simulate an event for a new directory with a file in it
        let event_dir_path = root_path.join("event_added_dir");
        fs::create_dir(&event_dir_path).unwrap();
        let file_in_event_dir_path = event_dir_path.join("inner_event.dat");
        fs::write(&file_in_event_dir_path, "data").unwrap(); // content of size 4

        let last_event_id_2 = cache.last_event_id();
        let event_dir = FsEvent {
            path: event_dir_path.clone(), // Event is for the directory
            id: last_event_id_2 + 1,
            flag: EventFlag::ItemCreated | EventFlag::ItemIsDir, // scan_path_recursive will scan children
        };
        cache.handle_fs_events(vec![event_dir]);

        let results_event_dir = cache.query_files("event_added_dir".to_string()).unwrap();
        assert_eq!(results_event_dir.len(), 1);
        assert!(
            results_event_dir[0].metadata.is_some(),
            "Dir metadata should be Some after event processing for dir"
        );

        let results_file_in_event_dir = cache.query_files("inner_event.dat".to_string()).unwrap();
        assert_eq!(results_file_in_event_dir.len(), 1);
        let inner_file_meta = results_file_in_event_dir[0]
            .metadata
            .as_ref()
            .expect("File in event-added dir metadata should be Some");
        assert_eq!(inner_file_meta.size, 4);
    }
}
