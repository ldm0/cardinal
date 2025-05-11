use crate::{
    SlabNode,
    persistent::{PersistentStorage, read_cache_from_file, write_cache_to_file},
    query::{Segment, query_segmentation},
};
use anyhow::{Context, Result, anyhow, bail};
use cardinal_sdk::{
    fsevent::{EventFlag, FsEvent, ScanType},
    utils::current_event_id,
};
use fswalk::{Node, WalkData, walk_it};
use namepool::NamePool;
use slab::Slab;
use std::{
    collections::BTreeMap,
    ffi::{CString, OsStr},
    io::ErrorKind,
    path::{MAIN_SEPARATOR_STR, Path, PathBuf},
    time::Instant,
};
use typed_num::Num;

pub struct SearchCache {
    path: PathBuf,
    last_event_id: u64,
    slab_root: usize,
    slab: Slab<SlabNode>,
    name_index: BTreeMap<String, Vec<usize>>,
    name_pool: NamePool,
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
            let walk_data = WalkData::with_ignore_directory(PathBuf::from("/System/Volumes/Data"));
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
        println!("Walking filesystem...");
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
                    nodes.extend_from_slice(
                        self.name_index
                            .get(name)
                            .context("Name index or name pool corrupted")?,
                    );
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
    pub fn node_path(&self, index: usize) -> String {
        let node = &self.slab[index];
        let mut segments = vec![node.name.clone()];
        // Write code like this to avoid the root node, which has no node name and shouldn't be put into semgents.
        if let Some(mut parent) = node.parent {
            while let Some(new_parent) = self.slab[parent].parent {
                segments.push(self.slab[parent].name.clone());
                parent = new_parent;
            }
        }
        let mut result = String::new();
        for segment in self
            .path
            .iter()
            .filter(|&x| x != OsStr::new(MAIN_SEPARATOR_STR))
            .map(|x| x.to_string_lossy().into_owned())
            .chain(segments.into_iter().rev())
        {
            result.push('/');
            result.push_str(&segment);
        }
        result
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
                };
                let index = self.push_node(node);
                self.slab[current].children.push(index);
                index
            };
        }
        current
    }

    // `Self::scan_path_recursive`function returns index of the constructed node.
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    // - `raw_path` is not allowed to be root.
    pub fn scan_path_recursive(&mut self, raw_path: &Path) -> Option<usize> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(&self.path) else {
            return None;
        };
        if raw_path.metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        // Root scanning is not allowed, under such circumstances, we should
        // clear all things and do a full rescan(It should be processed outside
        // this function or add a separate branch to prune name index slab namepool etc.).
        let parent = path.parent().expect("Scanning root?");
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
        let walk_data = WalkData::with_ignore_directory(PathBuf::from("/System/Volumes/Data"));
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

    pub fn rescan(&mut self) -> Option<usize> {
        unimplemented!()
    }

    /// Removes a node and its children recursively by index.
    fn remove_node(&mut self, index: usize) {
        fn remove_single_node(cache: &mut SearchCache, index: usize) {
            if let Some(node) = cache.slab.try_remove(index) {
                if let Some(parent) = node.parent {
                    cache.slab[parent].children.retain(|&x| x != index);
                }
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

    fn update_event_id(&mut self, event_id: u64) {
        if event_id <= self.last_event_id {
            eprintln!("Event id is not increasing, ignoring");
            return;
        }
        self.last_event_id = event_id;
    }

    pub fn query_files(&self, query: String) -> Result<Vec<String>> {
        self.search(&query)
            .map(|nodes| nodes.into_iter().map(|node| self.node_path(node)).collect())
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
                let root = self.rescan();
                println!("Rescan done: {root:?}");
            }
            ScanType::Nop => {}
        }
        self.update_event_id(event.id);
    }

    pub fn handle_fs_events(&mut self, events: Vec<FsEvent>) {
        for event in events {
            if event.flag.contains(EventFlag::HistoryDone) {
                println!("History processing done");
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
}
