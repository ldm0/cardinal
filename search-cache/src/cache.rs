use crate::{
    OptionSlabIndex, SlabIndex, State, ThinSlab,
    persistent::{PersistentStorage, read_cache_from_file, write_cache_to_file},
    type_and_size::StateTypeSize,
};
use anyhow::{Context, Result, anyhow, bail};
use cardinal_sdk::{EventFlag, FsEvent, ScanType, current_event_id};
pub use fswalk::WalkData;
use fswalk::{Node, NodeFileType, NodeMetadata, walk_it};
use hashbrown::HashSet;
use namepool::NamePool;
use query_segmentation::{Segment, query_segmentation};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, OsStr},
    io::ErrorKind,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{LazyLock, atomic::AtomicBool},
    time::Instant,
};
use thin_vec::ThinVec;
use tracing::{debug, info};
use typed_num::Num;

#[derive(Debug, Serialize, Deserialize)]
pub struct SlabNode {
    parent: OptionSlabIndex,
    children: ThinVec<SlabIndex>,
    name: Box<str>,
    metadata: SlabNodeMetadataCompact,
}

impl SlabNode {
    pub fn parent(&self) -> Option<SlabIndex> {
        self.parent.to_option()
    }

    pub fn add_children(&mut self, children: SlabIndex) {
        if !self.children.contains(&children) {
            self.children.push(children);
        }
    }

    pub fn new(
        parent: Option<SlabIndex>,
        name: Box<str>,
        metadata: SlabNodeMetadataCompact,
    ) -> Self {
        Self {
            parent: OptionSlabIndex::from_option(parent),
            children: ThinVec::new(),
            name,
            metadata,
        }
    }
}

/// SlabNodeMetadataCompact with state ensured to be Some
pub struct SlabNodeMetadata<'a>(&'a SlabNodeMetadataCompact);

impl<'a> SlabNodeMetadata<'a> {
    pub fn r#type(&self) -> NodeFileType {
        self.0.state_type_and_size.r#type()
    }

    pub fn size(&self) -> u64 {
        self.0.state_type_and_size.size()
    }

    pub fn ctime(&self) -> Option<NonZeroU32> {
        NonZeroU32::new(self.0.ctime)
    }

    pub fn mtime(&self) -> Option<NonZeroU32> {
        NonZeroU32::new(self.0.mtime)
    }
}

/// Use a compact form so that
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct SlabNodeMetadataCompact {
    state_type_and_size: StateTypeSize,
    // Actually a Option<NonZeroU32>, but using u32 here due to https://github.com/serde-rs/serde/issues/1834
    ctime: u32,
    mtime: u32,
}

impl SlabNodeMetadataCompact {
    pub fn unaccessible() -> Self {
        Self {
            state_type_and_size: StateTypeSize::unaccessible(),
            ctime: 0,
            mtime: 0,
        }
    }

    pub fn some(
        NodeMetadata {
            r#type,
            size,
            ctime,
            mtime,
        }: NodeMetadata,
    ) -> Self {
        Self {
            state_type_and_size: StateTypeSize::some(r#type, size),
            ctime: ctime
                .and_then(|x| NonZeroU32::try_from(x).ok())
                .map(|x| x.get())
                .unwrap_or_default(),
            mtime: mtime
                .and_then(|x| NonZeroU32::try_from(x).ok())
                .map(|x| x.get())
                .unwrap_or_default(),
        }
    }

    pub fn none() -> Self {
        Self {
            state_type_and_size: StateTypeSize::none(),
            ctime: 0,
            mtime: 0,
        }
    }

    pub fn state(&self) -> State {
        self.state_type_and_size.state()
    }

    pub fn as_ref(&self) -> Option<SlabNodeMetadata<'_>> {
        match self.state() {
            State::Some => Some(SlabNodeMetadata(self)),
            State::Unaccessible | State::None => None,
        }
    }

    pub fn is_some(&self) -> bool {
        matches!(self.state(), State::Some)
    }

    pub fn is_none(&self) -> bool {
        matches!(self.state(), State::None)
    }

    pub fn is_unaccessible(&self) -> bool {
        matches!(self.state(), State::Unaccessible)
    }
}

#[derive(Debug)]
pub struct SearchResultNode {
    pub path: PathBuf,
    pub metadata: SlabNodeMetadataCompact,
}

pub struct SearchCache {
    path: PathBuf,
    last_event_id: u64,
    slab_root: SlabIndex,
    slab: ThinSlab<SlabNode>,
    name_index: BTreeMap<&'static str, HashSet<SlabIndex>>,
    ignore_path: Option<&'static Path>,
    cancel: Option<&'static AtomicBool>,
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

impl SearchCache {
    /// The `path` is the root path of the constructed cache and fsevent watch path.
    pub fn try_read_persistent_cache(
        path: &Path,
        cache_path: &Path,
        ignore_path: Option<&'static Path>,
        cancel: Option<&'static AtomicBool>,
    ) -> Result<Self> {
        read_cache_from_file(cache_path)
            .and_then(|x| {
                (x.path == path)
                    .then_some(())
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
                     version: _,
                     path,
                     slab_root,
                     slab,
                     name_index,
                     last_event_id,
                 }| {
                    Self::new(
                        path,
                        last_event_id,
                        slab_root,
                        slab,
                        name_index,
                        ignore_path,
                        cancel,
                    )
                },
            )
    }

    /// Get the total number of files and directories in the cache.
    pub fn get_total_files(&self) -> usize {
        self.slab.len()
    }

    pub fn walk_fs_with_ignore(path: PathBuf, ignore_path: &'static Path) -> Self {
        Self::walk_fs_with_walk_data(
            path,
            &WalkData::new(Some(ignore_path), false, None),
            Some(ignore_path),
            None,
        )
        .unwrap()
    }

    pub fn walk_fs(path: PathBuf) -> Self {
        Self::walk_fs_with_walk_data(path, &WalkData::new(None, false, None), None, None).unwrap()
    }

    /// This function is expected to be called with WalkData which metadata is not fetched.
    /// If cancelled during walking, None is returned.
    pub fn walk_fs_with_walk_data(
        path: PathBuf,
        walk_data: &WalkData,
        ignore_path: Option<&'static Path>,
        cancel: Option<&'static AtomicBool>,
    ) -> Option<Self> {
        // Return None if cancelled
        fn walkfs_to_slab(
            path: &Path,
            walk_data: &WalkData,
        ) -> Option<(SlabIndex, ThinSlab<SlabNode>)> {
            // 先多线程构建树形文件名列表(不能直接创建 slab 因为 slab 无法多线程构建(slab 节点有相互引用，不想加锁))
            let visit_time = Instant::now();
            let node = walk_it(path, walk_data)?;
            info!(
                "Walk data: {:?}, time: {:?}",
                walk_data,
                visit_time.elapsed()
            );

            // 然后创建 slab
            let slab_time = Instant::now();
            let mut slab = ThinSlab::new();
            let slab_root = construct_node_slab(None, &node, &mut slab);
            info!(
                "Slab construction time: {:?}, slab root: {:?}, slab len: {:?}",
                slab_time.elapsed(),
                slab_root,
                slab.len()
            );

            Some((slab_root, slab))
        }
        fn name_index(slab: &ThinSlab<SlabNode>) -> BTreeMap<Box<str>, HashSet<SlabIndex>> {
            // TODO(ldm0): Memory optimization can be done by letting name index reference the name in the pool(gc need to be considered though)
            fn construct_name_index(
                slab: &ThinSlab<SlabNode>,
                name_index: &mut BTreeMap<Box<str>, HashSet<SlabIndex>>,
            ) {
                // The slab is newly constructed, thus though slab.iter() iterates all slots, it won't waste too much.
                slab.iter().for_each(|(i, node)| {
                    if let Some(nodes) = name_index.get_mut(&node.name) {
                        nodes.insert(i);
                    } else {
                        let mut nodes = HashSet::with_capacity(1);
                        nodes.insert(i);
                        name_index.insert(node.name.clone(), nodes);
                    };
                });
            }

            let name_index_time = Instant::now();
            let mut name_index = BTreeMap::default();
            construct_name_index(slab, &mut name_index);
            info!(
                "Name index construction time: {:?}, len: {}",
                name_index_time.elapsed(),
                name_index.len()
            );
            name_index
        }

        let last_event_id = current_event_id();
        let (slab_root, slab) = walkfs_to_slab(&path, walk_data)?;
        let name_index = name_index(&slab);
        // metadata cache inits later
        Some(Self::new(
            path,
            last_event_id,
            slab_root,
            slab,
            name_index,
            ignore_path,
            cancel,
        ))
    }

    fn new(
        path: PathBuf,
        last_event_id: u64,
        slab_root: SlabIndex,
        slab: ThinSlab<SlabNode>,
        name_index: BTreeMap<Box<str>, HashSet<SlabIndex>>,
        ignore_path: Option<&'static Path>,
        cancel: Option<&'static AtomicBool>,
    ) -> Self {
        // name pool construction speed is fast enough that caching it doesn't worth it.
        let name_index = name_pool(name_index);
        Self {
            path,
            last_event_id,
            slab_root,
            slab,
            name_index,
            ignore_path,
            cancel,
        }
    }

    pub fn search_empty(&self) -> Vec<SlabIndex> {
        self.name_index.values().flatten().copied().collect()
    }

    pub fn search(&self, line: &str) -> Result<Vec<SlabIndex>> {
        let segments = query_segmentation(line);
        if segments.is_empty() {
            bail!("Unprocessable query: {:?}", line);
        }
        let search_time = Instant::now();
        let mut node_set: Option<Vec<SlabIndex>> = None;
        for segment in &segments {
            if let Some(nodes) = &node_set {
                let mut new_node_set = Vec::with_capacity(nodes.len());
                for &node in nodes {
                    let childs = &self.slab[node].children;
                    for &child in childs {
                        if match segment {
                            Segment::Substr(substr) => self.slab[child].name.contains(*substr),
                            Segment::Prefix(prefix) => self.slab[child].name.starts_with(*prefix),
                            Segment::Exact(exact) => &*self.slab[child].name == *exact,
                            Segment::Suffix(suffix) => self.slab[child].name.ends_with(*suffix),
                        } {
                            new_node_set.push(child);
                        }
                    }
                }
                node_set = Some(new_node_set);
            } else {
                // Use BTreeSet here to:
                // 1. deduplicate names
                // 2. keep the search result in order
                let names: BTreeSet<_> = match segment {
                    Segment::Substr(substr) => NAME_POOL.search_substr(substr),
                    Segment::Prefix(prefix) => {
                        let mut buffer = Vec::with_capacity(prefix.len() + 1);
                        buffer.push(0);
                        buffer.extend_from_slice(prefix.as_bytes());
                        NAME_POOL.search_prefix(&buffer)
                    }
                    Segment::Exact(exact) => {
                        let mut buffer = Vec::with_capacity(exact.len() + 2);
                        buffer.push(0);
                        buffer.extend_from_slice(exact.as_bytes());
                        buffer.push(0);
                        NAME_POOL.search_exact(&buffer)
                    }
                    Segment::Suffix(suffix) => {
                        // Query contains nul is very rare
                        let suffix = CString::new(*suffix).expect("Query contains nul");
                        NAME_POOL.search_suffix(&suffix)
                    }
                };
                let mut nodes = Vec::with_capacity(names.len());
                names.into_iter().for_each(|name| {
                    // namepool doesn't shrink, so it can contains non-existng names. Therefore, we don't error out on None branch here.
                    if let Some(x) = self.name_index.get(name) {
                        nodes.extend(x.iter().copied());
                    }
                });
                node_set = Some(nodes);
            }
        }
        let search_time = search_time.elapsed();
        info!("Search time: {:?}", search_time);
        // Safety: node_set can't be None since segments is not empty.
        Ok(node_set.unwrap())
    }

    /// Get the path of the node in the slab.
    pub fn node_path(&self, index: SlabIndex) -> Option<PathBuf> {
        let mut current = index;
        let mut segments = vec![];
        while let Some(parent) = self.slab.get(current)?.parent() {
            segments.push(self.slab.get(current)?.name.clone());
            current = parent;
        }
        Some(
            self.path
                .iter()
                .chain(segments.iter().rev().map(|x| OsStr::new(x.as_ref())))
                .collect(),
        )
    }

    fn push_node(&mut self, node: SlabNode) -> SlabIndex {
        let node_name = node.name.clone();
        let index = self.slab.insert(node);
        if let Some(indexes) = self.name_index.get_mut(&*node_name) {
            indexes.insert(index);
        } else {
            let mut indices = HashSet::with_capacity(1);
            indices.insert(index);
            let node_name = NAME_POOL.push(&node_name);
            self.name_index.insert(node_name, indices);
        }
        index
    }

    /// Removes a node by path and its children recursively.
    fn remove_node_path(&mut self, path: &Path) -> Option<SlabIndex> {
        let mut current = self.slab_root;
        for name in path.components().map(|x| x.as_os_str()) {
            if let Some(&index) = self.slab[current]
                .children
                .iter()
                .find(|&&x| &*self.slab[x].name == name)
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
    fn create_node_chain(&mut self, path: &Path) -> SlabIndex {
        let mut current = self.slab_root;
        let mut current_path = self.path.clone();
        for name in path.components().map(|x| x.as_os_str()) {
            current_path.push(name);
            let name = name.to_string_lossy().into_owned().into_boxed_str();
            current = if let Some(&index) = self.slab[current]
                .children
                .iter()
                .find(|&&x| self.slab[x].name == name)
            {
                index
            } else {
                // TODO(ldm0): optimize: slab node children is empty, we can create a node chain directly.
                let metadata = std::fs::symlink_metadata(&current_path)
                    .map(NodeMetadata::from)
                    .ok();
                let node = SlabNode::new(
                    Some(current),
                    name,
                    match metadata {
                        Some(metadata) => SlabNodeMetadataCompact::some(metadata),
                        None => SlabNodeMetadataCompact::unaccessible(),
                    },
                );
                let index = self.push_node(node);
                self.slab[current].add_children(index);
                index
            };
        }
        current
    }

    // `Self::scan_path_recursive`function returns index of the constructed node(with metadata provided).
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    fn scan_path_recursive(&mut self, raw_path: &Path) -> Option<SlabIndex> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(&self.path) else {
            return None;
        };
        if raw_path.symlink_metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        let parent = path.parent().expect(
            "scan_path_recursive doesn't expected to scan root(should be filtered outside)",
        );
        // Ensure node of the path parent is existed
        let parent = self.create_node_chain(parent);
        // Remove node(if exists) and do a full rescan
        if let Some(&old_node) = self.slab[parent]
            .children
            .iter()
            .find(|&&x| path.file_name() == Some(OsStr::new(&*self.slab[x].name)))
        {
            self.remove_node(old_node);
        }
        // For incremental data, we need metadata
        let walk_data = WalkData::new(self.ignore_path, true, self.cancel);
        walk_it(raw_path, &walk_data).map(|node| {
            let node = self.create_node_slab_update_name_index_and_name_pool(Some(parent), &node);
            // Push the newly created node to the parent's children
            self.slab[parent].add_children(node);
            node
        })
    }

    // `Self::scan_path_nonrecursive`function returns index of the constructed node.
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    #[allow(dead_code)]
    fn scan_path_nonrecursive(&mut self, raw_path: &Path) -> Option<SlabIndex> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(&self.path) else {
            return None;
        };
        if raw_path.symlink_metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        Some(self.create_node_chain(path))
    }

    pub fn rescan(&mut self) {
        // Remove all memory consuming cache early for memory consumption in Self::walk_fs_new.
        let Some(new_cache) = Self::walk_fs_with_walk_data(
            self.path.clone(),
            &WalkData::new(self.ignore_path, false, self.cancel),
            self.ignore_path,
            self.cancel,
        ) else {
            info!("Rescan cancelled.");
            return;
        };
        *self = new_cache;
    }

    /// Removes a node and its children recursively by index.
    fn remove_node(&mut self, index: SlabIndex) {
        fn remove_single_node(cache: &mut SearchCache, index: SlabIndex) {
            if let Some(node) = cache.slab.try_remove(index) {
                let indexes = cache
                    .name_index
                    .get_mut(&*node.name)
                    .expect("inconsistent name index and node");
                indexes.remove(&index);
                if indexes.is_empty() {
                    cache.name_index.remove(&*node.name);
                    // TODO(ldm0): actually we need to remove name in the name pool,
                    // but currently name pool doesn't support remove. (GC is needed for name pool)
                    // self.name_pool.remove(&node.name);
                }
            }
        }

        // Remove parent reference, make whole subtree unreachable.
        if let Some(parent) = self.slab[index].parent() {
            self.slab[parent].children.retain(|&x| x != index);
        }
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            stack.extend_from_slice(&self.slab[current].children);
            remove_single_node(self, current);
        }
    }

    pub fn flush_to_file(self, cache_path: &Path) -> Result<()> {
        let Self {
            path,
            last_event_id,
            slab_root,
            slab,
            name_index,
            ignore_path: _,
            cancel: _,
        } = self;
        let name_index = name_index
            .into_iter()
            .map(|(k, v)| (k.to_string().into_boxed_str(), v))
            .collect();
        write_cache_to_file(
            cache_path,
            PersistentStorage {
                version: Num,
                path,
                slab_root,
                slab,
                name_index,
                last_event_id,
            },
        )
        .context("Write cache to file failed.")
    }

    fn update_last_event_id(&mut self, event_id: u64) {
        if event_id <= self.last_event_id {
            debug!("last_event_id {} |< {event_id}", self.last_event_id);
        } else {
            debug!("last_event_id {} => {event_id}", self.last_event_id);
            self.last_event_id = event_id;
        }
    }

    pub fn last_event_id(&mut self) -> u64 {
        self.last_event_id
    }

    /// Note that this function doesn't fetch metadata(even if it's not cahced) for the nodes.
    pub fn query_files(&mut self, query: String) -> Result<Vec<SearchResultNode>> {
        self.search(&query)
            .map(|nodes| self.expand_file_nodes_inner::<false>(nodes))
    }

    /// Returns a node info vector with the same length as the input nodes.
    /// If the given node is not found, an empty SearchResultNode is returned.
    pub fn expand_file_nodes(&mut self, nodes: Vec<SlabIndex>) -> Vec<SearchResultNode> {
        self.expand_file_nodes_inner::<true>(nodes)
    }

    fn expand_file_nodes_inner<const FETCH_META: bool>(
        &mut self,
        nodes: impl IntoIterator<Item = SlabIndex>,
    ) -> Vec<SearchResultNode> {
        nodes
            .into_iter()
            .map(|node_index| {
                let path = self.node_path(node_index);
                let metadata = self
                    .slab
                    .get_mut(node_index)
                    .map(|node| {
                        match (node.metadata.state(), &path) {
                            (State::None, Some(path)) if FETCH_META => {
                                // try fetching metadata if it's not cached and cache them
                                let metadata = match std::fs::symlink_metadata(path) {
                                    Ok(metadata) => SlabNodeMetadataCompact::some(metadata.into()),
                                    Err(_) => SlabNodeMetadataCompact::unaccessible(),
                                };
                                node.metadata = metadata;
                                metadata
                            }
                            _ => node.metadata,
                        }
                    })
                    .unwrap_or_else(SlabNodeMetadataCompact::unaccessible);
                SearchResultNode {
                    path: path.unwrap_or_default(),
                    metadata,
                }
            })
            .collect()
    }

    pub fn handle_fs_events(&mut self, events: Vec<FsEvent>) -> Result<(), HandleFSEError> {
        let max_event_id = events.iter().map(|e| e.id).max();
        // If rescan needed, early exit.
        if events.iter().any(|event| {
            if event.flag.contains(EventFlag::HistoryDone) {
                info!("History processing done: {:?}", event);
            }
            if event.should_rescan(&self.path) {
                info!("Event rescan: {:?}", event);
                true
            } else {
                false
            }
        }) {
            return Err(HandleFSEError::Rescan);
        }
        for scan_path in scan_paths(events) {
            info!("Scanning path: {scan_path:?}");
            let folder = self.scan_path_recursive(&scan_path);
            if folder.is_some() {
                info!("Node changed: {folder:?}");
            }
        }
        if let Some(max_event_id) = max_event_id {
            self.update_last_event_id(max_event_id);
        }
        Ok(())
    }
}

/// 根据一批 FsEvent 计算需要进行递归/单节点扫描的最小路径集合。
///
/// 功能 & 目标:
/// 1. 过滤掉不需要增量扫描的事件 (例如 `ReScan` / `Nop` 类型: RootChanged, HistoryDone 等)，这些事件在更高层逻辑里会触发完整重建或者仅更新 event id。
/// 2. 只保留 `ScanType::SingleNode` 与 `ScanType::Folder` 的事件路径。
/// 3. 对路径做“祖先去重”与“祖先覆盖”：
///    - 如果即将插入的路径已被某个祖先路径覆盖 (path.starts_with(ancestor))，跳过它；
///    - 如果新路径是现有若干路径的祖先，则移除所有这些后代，只保留祖先；
///    - 相同路径只会出现一次 (后续重复事件会被 starts_with 判定为已覆盖)。
/// 4. 结果是需要最少扫描次数即可覆盖所有事件影响的最小集合 (Minimal Cover)。
///
/// 使用场景:
/// - `SearchCache::handle_fs_events` 中遍历返回的路径，对每个路径执行 `scan_path_recursive`，避免对子孙/重复路径做浪费的多次扫描。
/// - FSEvents 高频且可能“冒泡”出大量同一子树的文件/目录更改，合并可以显著降低后续 IO / 元数据抓取开销。
///
/// 算法复杂度:
/// - 最坏 O(n^2)：当输入是严格降序的深层路径链 (如 a/b/c/d/e ... 之后再插入 a)，会产生多次 retain 过滤。
/// - 典型批次 (几十个以内) 成本可接受；如需进一步优化，可改为先排序再线性扫描或使用 Trie/前缀树。
///
/// Corner Cases & 处理方式:
/// - 空输入 => 返回空 Vec。
/// - 重复相同路径多次 => 只保留一次 (后续会被 starts_with 匹配跳过)。
/// - 先出现子路径, 后出现其祖先 => 祖先覆盖子路径, 仅祖先保留。
/// - 先出现祖先, 后出现子路径 => 子路径被跳过。
/// - 兄弟路径互不影响 => 全部保留。
/// - 路径名字前缀但不是父子关系 (如 /foo/bar 与 /foo/barista) => 二者都保留 (Path::starts_with 以组件匹配，不会把 barista 当作 bar 的子路径)。
/// - 混合 Folder / SingleNode 事件 => 一起参与最小化；不区分类型只看路径祖先关系。
/// - 使用 PathBuf 原样比较，不做规范化：不会展开符号链接；调用方需保证一致性。
///
/// 效果: 本地测试跳过了 415449 个事件中 173034 个事件的扫描
fn scan_paths(events: Vec<FsEvent>) -> Vec<PathBuf> {
    let num_events = events.len();
    events
        .into_iter()
        .filter(|event| {
            // Sometimes there are rediculous events assuming dir as file, so we always scan them as folder
            matches!(
                event.flag.scan_type(),
                ScanType::SingleNode | ScanType::Folder
            )
        })
        .map(|event| event.path)
        .fold(Vec::with_capacity(num_events), |mut events, path| {
            if !events.iter().any(|p: &PathBuf| path.starts_with(p)) {
                events.retain(|p: &PathBuf| !p.starts_with(&path));
                events.push(path);
            }
            events
        })
}

/// Error type for `SearchCache::handle_fs_event`.
#[derive(Debug)]
pub enum HandleFSEError {
    /// Full rescan is required.
    Rescan,
}

/// Note: This function is expected to be called with WalkData which metadata is not fetched.
fn construct_node_slab(
    parent: Option<SlabIndex>,
    node: &Node,
    slab: &mut ThinSlab<SlabNode>,
) -> SlabIndex {
    let metadata = match node.metadata {
        Some(metadata) => SlabNodeMetadataCompact::some(metadata),
        None => SlabNodeMetadataCompact::none(),
    };
    let slab_node = SlabNode::new(parent, node.name.clone(), metadata);
    let index = slab.insert(slab_node);
    slab[index].children = node
        .children
        .iter()
        .map(|node| construct_node_slab(Some(index), node, slab))
        .collect();
    index
}

impl SearchCache {
    /// ATTENTION: This function doesn't remove existing node, you should remove it
    /// before creating the new subtree, or the old subtree nodes will be dangling.
    ///
    /// ATTENTION1: This function should only called with Node fetched with metadata.
    fn create_node_slab_update_name_index_and_name_pool(
        &mut self,
        parent: Option<SlabIndex>,
        node: &Node,
    ) -> SlabIndex {
        let metadata = match node.metadata {
            Some(metadata) => SlabNodeMetadataCompact::some(metadata),
            // This function should only be called with Node fetched with metadata
            None => SlabNodeMetadataCompact::unaccessible(),
        };
        let slab_node = SlabNode::new(parent, node.name.clone(), metadata);
        let index = self.push_node(slab_node);
        self.slab[index].children = node
            .children
            .iter()
            .map(|node| self.create_node_slab_update_name_index_and_name_pool(Some(index), node))
            .collect();
        index
    }
}

pub static NAME_POOL: LazyLock<NamePool> = LazyLock::new(NamePool::new);

fn name_pool(
    name_index: BTreeMap<Box<str>, HashSet<SlabIndex>>,
) -> BTreeMap<&'static str, HashSet<SlabIndex>> {
    let name_pool_time = Instant::now();
    let mut new_name_index = BTreeMap::new();
    for (name, indices) in name_index {
        let name: &'static str = NAME_POOL.push(&name);
        new_name_index.insert(name, indices);
    }
    info!(
        "Name pool construction time: {:?}, count: {}",
        name_pool_time.elapsed(),
        NAME_POOL.len(),
    );
    new_name_index
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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap();

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 0);
    }

    #[test]
    #[ignore]
    fn test_handle_fs_event_simulator() {
        let instant = std::time::Instant::now();
        let mut cache = SearchCache::walk_fs(PathBuf::from("/Library/Developer/CoreSimulator"));
        let mut event_id = cache.last_event_id + 1;
        println!(
            "Cache size: {}, process time: {:?}",
            cache.slab.len(),
            instant.elapsed()
        );
        // test speed of handling fs event
        loop {
            let instant = std::time::Instant::now();
            let mock_events = vec![FsEvent {
                path: PathBuf::from("/Library/Developer/CoreSimulator/Volumes/iOS_23A343"),
                id: event_id,
                flag: EventFlag::ItemCreated,
            }];

            cache.handle_fs_events(mock_events).unwrap();
            event_id += 1;
            println!(
                "Event id: {}, process time: {:?}",
                cache.last_event_id,
                instant.elapsed()
            );
        }
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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap_err();

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

        cache.handle_fs_events(mock_events).unwrap_err();

        // Rescan is required
        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);
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

        cache.handle_fs_events(mock_events).unwrap_err();

        assert_eq!(cache.slab.len(), 1);
        assert_eq!(cache.name_index.len(), 1);
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

        cache.handle_fs_events(mock_events).unwrap();

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

        cache.handle_fs_events(mock_events).unwrap();

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
    fn test_walk_fs_new_metadata_is_always_none() {
        let temp_dir =
            TempDir::new("test_walk_fs_new_meta").expect("Failed to create temp directory");
        let root_path = temp_dir.path();

        fs::File::create(root_path.join("file1.txt")).expect("Failed to create file1.txt");
        fs::create_dir(root_path.join("subdir1")).expect("Failed to create subdir1");
        fs::File::create(root_path.join("subdir1/file2.txt")).expect("Failed to create file1.txt");

        let cache = SearchCache::walk_fs(root_path.to_path_buf());

        // 目录的 metadata 都会是 Some()
        assert!(cache.slab[cache.slab_root].metadata.is_some());

        // Check metadata for a file node
        let file_nodes = cache
            .search("file1.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1, "Expected 1 node for file1.txt");
        let file_node_idx = file_nodes.into_iter().next().unwrap();
        // 文件的 metadata 都会是 None
        assert!(
            cache.slab[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs_new should be None"
        );

        // Check metadata for a file node
        let file_nodes = cache
            .search("file2.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1);
        let file_node_idx = file_nodes.into_iter().next().unwrap();
        // 文件的 metadata 都会是 None
        assert!(
            cache.slab[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs_new should be None"
        );

        // Check metadata for a subdirectory node
        let dir_nodes = cache.search("subdir1").expect("Search for subdir1 failed");
        assert_eq!(dir_nodes.len(), 1, "Expected 1 node for subdir1");
        let dir_node_idx = dir_nodes.into_iter().next().unwrap();
        // 目录的 metadata 都会是 Some()
        assert!(
            cache.slab[dir_node_idx].metadata.is_some(),
            "Metadata for directory node created by walk_fs_new should be Some"
        );
    }

    #[test]
    fn test_handle_fs_events_metadata() {
        let temp_dir = TempDir::new("test_event_meta").expect("Failed to create temp directory");
        let root_path = temp_dir.path();

        fs::File::create(root_path.join("file1.txt")).expect("Failed to create file1.txt");
        fs::create_dir(root_path.join("subdir1")).expect("Failed to create subdir1");
        fs::File::create(root_path.join("subdir1/file2.txt")).expect("Failed to create file1.txt");

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());
        let mut last_event_id = cache.last_event_id();

        let new_file_path = root_path.join("event_file.txt");
        fs::write(&new_file_path, b"heck").expect("Failed to create event_file.txt");

        let new_file_meta_on_disk = fs::symlink_metadata(&new_file_path).unwrap();
        last_event_id += 1;

        let file_event = FsEvent {
            path: new_file_path.clone(),
            id: last_event_id,
            flag: EventFlag::ItemCreated,
        };
        cache.handle_fs_events(vec![file_event]).unwrap();

        let file_nodes = cache
            .search("event_file.txt")
            .expect("Search for event_file.txt failed");
        assert_eq!(
            file_nodes.len(),
            1,
            "Expected 1 node for event_file.txt after event"
        );
        let file_node_idx = file_nodes.into_iter().next().unwrap();
        let file_slab_meta = cache.slab[file_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for event_file.txt should be populated by event handler");
        assert_eq!(
            file_slab_meta.size(),
            new_file_meta_on_disk.len(),
            "Size mismatch for event_file.txt"
        );
        assert_eq!(file_slab_meta.size(), 4, "Size mismatch for event_file.txt");
        assert!(
            file_slab_meta.mtime().is_some(),
            "mtime should be populated for event_file.txt"
        );

        // Part 2: Event for a newly created directory (should populate metadata for itself and its children)
        let new_subdir_path = root_path.join("event_subdir");
        fs::create_dir(&new_subdir_path).expect("Failed to create event_subdir");

        let file_in_subdir_path = new_subdir_path.join("file_in_event_subdir.txt");
        fs::File::create(&file_in_subdir_path).expect("Failed to create file_in_event_subdir.txt");
        let file_in_subdir_meta_on_disk = fs::symlink_metadata(&file_in_subdir_path).unwrap();
        last_event_id += 1;

        let dir_event = FsEvent {
            path: new_subdir_path.clone(), // Event is for the directory
            id: last_event_id,
            flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
        };
        cache.handle_fs_events(vec![dir_event]).unwrap();

        // Check metadata for the directory itself
        let dir_nodes = cache
            .search("/event_subdir/")
            .expect("Search for event_subdir failed");
        assert_eq!(
            dir_nodes.len(),
            1,
            "Expected 1 node for event_subdir after event"
        );
        let dir_node_idx = dir_nodes.into_iter().next().unwrap();
        let dir_slab_meta = cache.slab[dir_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for event_subdir should be populated by event handler");
        assert!(
            dir_slab_meta.mtime().is_some(),
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
        let file_in_subdir_node_idx = file_in_subdir_nodes.into_iter().next().unwrap();
        let file_in_subdir_slab_meta = cache.slab[file_in_subdir_node_idx]
            .metadata
            .as_ref()
            .expect("Metadata for file_in_event_subdir.txt should be populated");
        assert_eq!(
            file_in_subdir_slab_meta.size(),
            file_in_subdir_meta_on_disk.len(),
            "Size mismatch for file_in_event_subdir.txt"
        );
        assert!(
            file_in_subdir_slab_meta.mtime().is_some(),
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

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());

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
            "File metadata should be None after walk_fs_new"
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
            "Directory metadata should be Some after walk_fs_new"
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

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());

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

        let mut cache = SearchCache::walk_fs(root_path.to_path_buf());
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
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());
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

        let mut cache = SearchCache::walk_fs(root.to_path_buf());

        // Query for the deep file directly
        let results_deep_file = cache.query_files("gamma_file.txt".to_string()).unwrap();
        assert_eq!(results_deep_file.len(), 1);
        let expected_suffix_deep = "alpha_dir/beta_subdir/gamma_file.txt".to_string();
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

        // Check metadata from initial walk_fs_new
        let results_file_walk = cache.query_files("walk_file.txt".to_string()).unwrap();
        assert_eq!(results_file_walk.len(), 1);
        assert!(
            results_file_walk[0].metadata.is_none(),
            "File metadata from walk_fs_new should be None"
        );

        let results_dir_walk = cache.query_files("walk_dir".to_string()).unwrap();
        assert_eq!(results_dir_walk.len(), 1);
        assert!(
            results_dir_walk[0].metadata.is_some(),
            "Directory metadata from walk_fs_new should be Some"
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
        cache.handle_fs_events(vec![event]).unwrap();

        let results_event_file = cache
            .query_files("event_added_file.txt".to_string())
            .unwrap();
        assert_eq!(results_event_file.len(), 1);
        let event_file_meta = results_event_file[0]
            .metadata
            .as_ref()
            .expect("File metadata should be Some after event processing");
        assert_eq!(event_file_meta.size(), 10);

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
        cache.handle_fs_events(vec![event_dir]).unwrap();

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
        assert_eq!(inner_file_meta.size(), 4);
    }

    // --- scan_paths 专项测试 ---
    #[test]
    fn test_scan_paths_empty() {
        assert!(scan_paths(vec![]).is_empty());
    }

    #[test]
    fn test_scan_paths_only_rescan_events_kept_when_called_directly() {
        let root = PathBuf::from("/tmp/root");
        let events = vec![FsEvent {
            path: root.clone(),
            id: 1,
            flag: EventFlag::RootChanged,
        }];
        assert!(scan_paths(events).is_empty());
    }

    #[test]
    fn test_scan_paths_history_done_filtered() {
        let p = PathBuf::from("/tmp/a");
        let events = vec![FsEvent {
            path: p,
            id: 1,
            flag: EventFlag::HistoryDone,
        }];
        // HistoryDone => ScanType::Nop
        assert!(scan_paths(events).is_empty());
    }

    #[test]
    fn test_scan_paths_dedup_same_path() {
        let p = PathBuf::from("/tmp/a/b");
        let events = vec![
            FsEvent {
                path: p.clone(),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: p.clone(),
                id: 2,
                flag: EventFlag::ItemModified | EventFlag::ItemIsFile,
            }, // 假设错误标记, 仍然 SingleNode
            FsEvent {
                path: p,
                id: 3,
                flag: EventFlag::ItemRemoved | EventFlag::ItemIsFile,
            },
        ];
        let out = scan_paths(events);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], PathBuf::from("/tmp/a/b"));
    }

    #[test]
    fn test_scan_paths_child_then_parent_collapses() {
        let events = vec![
            FsEvent {
                path: PathBuf::from("/t/a/b/c"),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/t/a/b"),
                id: 2,
                flag: EventFlag::ItemModified | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: PathBuf::from("/t/a"),
                id: 3,
                flag: EventFlag::ItemModified | EventFlag::ItemIsDir,
            },
        ];
        let out = scan_paths(events);
        // 最终只剩 /t/a
        assert_eq!(out, vec![PathBuf::from("/t/a")]);
    }

    #[test]
    fn test_scan_paths_parent_then_child_skip_child() {
        let events = vec![
            FsEvent {
                path: PathBuf::from("/t/a"),
                id: 1,
                flag: EventFlag::ItemModified | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: PathBuf::from("/t/a/b"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/t/a/b/c"),
                id: 3,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
        ];
        let out = scan_paths(events);
        assert_eq!(out, vec![PathBuf::from("/t/a")]);
    }

    #[test]
    fn test_scan_paths_siblings_all_retained() {
        let events = vec![
            FsEvent {
                path: PathBuf::from("/t/a/x"),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/t/a/y"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/t/a/z"),
                id: 3,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
        ];
        let mut out = scan_paths(events);
        out.sort();
        assert_eq!(
            out,
            vec!["/t/a/x", "/t/a/y", "/t/a/z"]
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_scan_paths_prefix_but_not_parent() {
        // /foo/bar 与 /foo/barista 不是父子关系（组件不同），应同时保留
        let events = vec![
            FsEvent {
                path: PathBuf::from("/foo/bar"),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: PathBuf::from("/foo/barista"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
        ];
        let mut out = scan_paths(events);
        out.sort();
        assert_eq!(
            out,
            vec![PathBuf::from("/foo/bar"), PathBuf::from("/foo/barista")]
        );
    }

    #[test]
    fn test_scan_paths_mix_folder_and_single_node() {
        // 创建目录事件 + 文件修改事件, 目录应吸收其子文件
        let events = vec![
            FsEvent {
                path: PathBuf::from("/mix/dir/sub/file.txt"),
                id: 1,
                flag: EventFlag::ItemModified | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/mix/dir/sub"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
        ];
        let out = scan_paths(events);
        assert_eq!(out, vec![PathBuf::from("/mix/dir/sub")]);
    }

    #[test]
    fn test_scan_paths_large_chain_collapse() {
        // 模拟较长链条，最后祖先出现
        let mut events = Vec::new();
        let depth = ["a", "b", "c", "d", "e", "f"];
        for i in 0..depth.len() {
            let path = format!("/long/{}", depth[..=i].join("/"));
            events.push(FsEvent {
                path: PathBuf::from(path),
                id: i as u64,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            });
        }
        // 插入真正的祖先 /long
        events.push(FsEvent {
            path: PathBuf::from("/long"),
            id: 99,
            flag: EventFlag::ItemModified | EventFlag::ItemIsDir,
        });
        let out = scan_paths(events);
        assert_eq!(out, vec![PathBuf::from("/long")]);
    }
}
