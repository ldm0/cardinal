use crate::{
    FileNodes, NameIndex, SearchOptions, SearchResultNode, SlabIndex, SlabNode,
    SlabNodeMetadataCompact, State, ThinSlab,
    persistent::{PersistentStorage, read_cache_from_file, write_cache_to_file},
};
use anyhow::{Context, Result, anyhow};
use cardinal_sdk::{EventFlag, FsEvent, ScanType, current_event_id};
use cardinal_syntax::parse_query;
use fswalk::{Node, NodeMetadata, WalkData, walk_it};
use hashbrown::HashSet;
use namepool::NamePool;
use search_cancel::CancellationToken;
use std::{
    ffi::OsStr,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{LazyLock, atomic::AtomicBool},
    time::Instant,
};
use thin_vec::ThinVec;
use tracing::{debug, info};
use typed_num::Num;

pub struct SearchCache {
    pub(crate) file_nodes: FileNodes,
    last_event_id: u64,
    pub(crate) name_index: NameIndex,
    ignore_paths: Option<Vec<PathBuf>>,
    cancel: Option<&'static AtomicBool>,
}

impl std::fmt::Debug for SearchCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchCache")
            .field("path", &self.file_nodes.path())
            .field("last_event_id", &self.last_event_id)
            .field("slab_root", &self.file_nodes.root())
            .field("slab.len()", &self.file_nodes.len())
            .field("name_index.len()", &self.name_index.len())
            .finish()
    }
}

impl SearchCache {
    /// The `path` is the root path of the constructed cache and fsevent watch path.
    pub fn try_read_persistent_cache(
        path: &Path,
        cache_path: &Path,
        ignore_paths: Option<Vec<PathBuf>>,
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
                    // name pool construction speed is fast enough that caching it doesn't worth it.
                    let name_index = NameIndex::construct_name_pool(name_index);
                    let slab = FileNodes::new(path, slab, slab_root);
                    Self::new(slab, last_event_id, name_index, ignore_paths, cancel)
                },
            )
    }

    /// Get the total number of files and directories in the cache.
    pub fn get_total_files(&self) -> usize {
        self.file_nodes.len()
    }

    pub fn walk_fs_with_ignore(path: PathBuf, ignore_paths: Vec<PathBuf>) -> Self {
        let ignore_paths_opt = if ignore_paths.is_empty() {
            None
        } else {
            Some(ignore_paths)
        };
        Self::walk_fs_with_walk_data(
            path,
            &WalkData::new(ignore_paths_opt.clone(), false, None),
            ignore_paths_opt,
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
        ignore_paths: Option<Vec<PathBuf>>,
        cancel: Option<&'static AtomicBool>,
    ) -> Option<Self> {
        // Return None if cancelled
        fn walkfs_to_slab(
            path: &Path,
            walk_data: &WalkData,
        ) -> Option<(SlabIndex, ThinSlab<SlabNode>, NameIndex)> {
            // Build the tree of file names in parallel first (we cannot construct the slab directly
            // because slab nodes reference each other and we prefer to avoid locking).
            let visit_time = Instant::now();
            let node = walk_it(path, walk_data)?;
            info!(
                "Walk data: {:?}, time: {:?}",
                walk_data,
                visit_time.elapsed()
            );

            // Then create the slab.
            let slab_time = Instant::now();
            let mut slab = ThinSlab::new();
            let mut name_index = NameIndex::default();
            let slab_root = construct_node_slab_name_index(None, &node, &mut slab, &mut name_index);
            info!(
                "Slab & NameIndex construction time: {:?}, slab root: {:?}, slab len: {:?}",
                slab_time.elapsed(),
                slab_root,
                slab.len()
            );

            Some((slab_root, slab, name_index))
        }

        let last_event_id = current_event_id();
        let (slab_root, slab, name_index) = walkfs_to_slab(&path, walk_data)?;
        let slab = FileNodes::new(path, slab, slab_root);
        // metadata cache inits later
        Some(Self::new(
            slab,
            last_event_id,
            name_index,
            ignore_paths,
            cancel,
        ))
    }

    fn new(
        slab: FileNodes,
        last_event_id: u64,
        name_index: NameIndex,
        ignore_paths: Option<Vec<PathBuf>>,
        cancel: Option<&'static AtomicBool>,
    ) -> Self {
        Self {
            file_nodes: slab,
            last_event_id,
            name_index,
            ignore_paths,
            cancel,
        }
    }

    pub fn search_empty(&self, cancellation_token: CancellationToken) -> Option<Vec<SlabIndex>> {
        self.name_index.all_indices(cancellation_token)
    }

    #[cfg(test)]
    pub fn search(&self, line: &str) -> Result<Vec<SlabIndex>> {
        self.search_with_options(line, SearchOptions::default(), CancellationToken::noop())
            .map(|x| x.unwrap_or_default())
    }

    pub fn search_with_options(
        &self,
        line: &str,
        options: SearchOptions,
        cancellation_token: CancellationToken,
    ) -> Result<Option<Vec<SlabIndex>>> {
        let parsed = parse_query(line).map_err(|err| anyhow!("Failed to parse query: {err}"))?;
        let search_time = Instant::now();
        let result = self.evaluate_expr(&parsed.expr, options, cancellation_token);
        info!("Search time: {:?}", search_time.elapsed());
        result
    }

    /// Get the path of the node in the slab.
    pub fn node_path(&self, index: SlabIndex) -> Option<PathBuf> {
        self.file_nodes.node_path(index)
    }

    /// Locate the slab index for a path relative to the watch root.
    pub fn node_index_for_relative_path(&self, relative: &Path) -> Option<SlabIndex> {
        let mut current = self.file_nodes.root();
        if relative.as_os_str().is_empty() {
            return Some(current);
        }
        for segment in relative.components().map(|component| component.as_os_str()) {
            let next = self.file_nodes[current]
                .children
                .iter()
                .find_map(|&child| {
                    let name = self.file_nodes[child].name_and_parent.as_str();
                    if OsStr::new(name) == segment {
                        Some(child)
                    } else {
                        None
                    }
                })?;
            current = next;
        }
        Some(current)
    }

    /// Locate the slab index for an absolute path when it belongs to the watch root.
    pub fn node_index_for_raw_path(&self, raw_path: &Path) -> Option<SlabIndex> {
        let relative = raw_path.strip_prefix(self.file_nodes.path()).ok()?;
        self.node_index_for_relative_path(relative)
    }

    fn push_node(&mut self, node: SlabNode) -> SlabIndex {
        let node_name = node.name_and_parent;
        let index = self.file_nodes.insert(node);
        self.name_index
            .add_index(node_name.as_str(), index, &self.file_nodes);
        index
    }

    /// Removes a node by path and its children recursively.
    fn remove_node_path(&mut self, path: &Path) -> Option<SlabIndex> {
        let mut current = self.file_nodes.root();
        for name in path.components().map(|x| x.as_os_str()) {
            if let Some(&index) = self.file_nodes[current]
                .children
                .iter()
                .find(|&&x| self.file_nodes[x].name_and_parent.as_str() == name)
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
        let mut current = self.file_nodes.root();
        let mut current_path = self.file_nodes.path().to_path_buf();
        for name in path.components().map(|x| x.as_os_str()) {
            current_path.push(name);
            current = if let Some(&index) = self.file_nodes[current]
                .children
                .iter()
                .find(|&&x| self.file_nodes[x].name_and_parent.as_str() == name)
            {
                index
            } else {
                // TODO(ldm0): optimize: slab node children is empty, we can create a node chain directly.
                let metadata = std::fs::symlink_metadata(&current_path)
                    .map(NodeMetadata::from)
                    .ok();
                let name = NAME_POOL.push(name.to_string_lossy().as_ref());
                let node = SlabNode::new(
                    Some(current),
                    name,
                    match metadata {
                        Some(metadata) => SlabNodeMetadataCompact::some(metadata),
                        None => SlabNodeMetadataCompact::unaccessible(),
                    },
                );
                let index = self.push_node(node);
                self.file_nodes[current].add_children(index);
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
        let Ok(path) = raw_path.strip_prefix(self.file_nodes.path()) else {
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
        if let Some(&old_node) = self.file_nodes[parent].children.iter().find(|&&x| {
            path.file_name() == Some(OsStr::new(self.file_nodes[x].name_and_parent.as_str()))
        }) {
            self.remove_node(old_node);
        }
        // For incremental data, we need metadata
        let walk_data = WalkData::new(self.ignore_paths.clone(), true, self.cancel);
        walk_it(raw_path, &walk_data).map(|node| {
            let node = self.create_node_slab_update_name_index_and_name_pool(Some(parent), &node);
            // Push the newly created node to the parent's children
            self.file_nodes[parent].add_children(node);
            node
        })
    }

    // `Self::scan_path_nonrecursive`function returns index of the constructed node.
    // - If path is not under the watch root, None is returned.
    // - Procedure contains metadata fetching, if metadata fetching failed, None is returned.
    #[allow(dead_code)]
    fn scan_path_nonrecursive(&mut self, raw_path: &Path) -> Option<SlabIndex> {
        // Ensure path is under the watch root
        let Ok(path) = raw_path.strip_prefix(self.file_nodes.path()) else {
            return None;
        };
        if raw_path.symlink_metadata().err().map(|e| e.kind()) == Some(ErrorKind::NotFound) {
            self.remove_node_path(path);
            return None;
        };
        Some(self.create_node_chain(path))
    }

    pub fn walk_data(&self) -> WalkData<'static> {
        WalkData::new(self.ignore_paths.clone(), false, self.cancel)
    }

    pub fn rescan_with_walk_data(&mut self, walk_data: &WalkData) -> Option<()> {
        let Some(new_cache) = Self::walk_fs_with_walk_data(
            self.file_nodes.path().to_path_buf(),
            walk_data,
            self.ignore_paths.clone(),
            self.cancel,
        ) else {
            info!("Rescan cancelled.");
            return None;
        };
        *self = new_cache;
        Some(())
    }

    pub fn rescan(&mut self) {
        // Remove all memory consuming cache early for memory consumption in Self::walk_fs_new.
        let Some(new_cache) = Self::walk_fs_with_walk_data(
            self.file_nodes.path().to_path_buf(),
            &WalkData::new(self.ignore_paths.clone(), false, self.cancel),
            self.ignore_paths.clone(),
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
            if let Some(node) = cache.file_nodes.try_remove(index) {
                let removed = cache
                    .name_index
                    .remove_index(node.name_and_parent.as_str(), index);
                assert!(removed, "inconsistent name index and node");
            }
        }

        // Remove parent reference, make whole subtree unreachable.
        if let Some(parent) = self.file_nodes[index].name_and_parent.parent() {
            self.file_nodes[parent].children.retain(|&x| x != index);
        }
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            stack.extend_from_slice(&self.file_nodes[current].children);
            remove_single_node(self, current);
        }
    }

    pub fn flush_to_file(self, cache_path: &Path) -> Result<()> {
        let Self {
            file_nodes: slab,
            last_event_id,
            name_index,
            ignore_paths: _,
            cancel: _,
        } = self;
        let (path, slab_root, slab) = slab.into_parts();
        let name_index = name_index.into_persistent();
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
    pub fn query_files(
        &mut self,
        query: String,
        cancellation_token: CancellationToken,
    ) -> Result<Option<Vec<SearchResultNode>>> {
        self.query_files_with_options(query, SearchOptions::default(), cancellation_token)
    }

    pub fn query_files_with_options(
        &mut self,
        query: String,
        options: SearchOptions,
        cancellation_token: CancellationToken,
    ) -> Result<Option<Vec<SearchResultNode>>> {
        self.search_with_options(&query, options, cancellation_token)
            .map(|nodes| nodes.map(|nodes| self.expand_file_nodes_inner::<false>(&nodes)))
    }

    /// Returns a node info vector with the same length as the input nodes.
    /// If the given node is not found, an empty SearchResultNode is returned.
    pub fn expand_file_nodes(&mut self, nodes: &[SlabIndex]) -> Vec<SearchResultNode> {
        self.expand_file_nodes_inner::<true>(nodes)
    }

    fn expand_file_nodes_inner<const FETCH_META: bool>(
        &mut self,
        nodes: &[SlabIndex],
    ) -> Vec<SearchResultNode> {
        nodes
            .iter()
            .copied()
            .map(|node_index| {
                let path = self.node_path(node_index);
                let metadata = self
                    .file_nodes
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
            if event.should_rescan(self.file_nodes.path()) {
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

/// Compute the minimal set of paths that must be rescanned for a batch of FsEvents.
///
/// Goals:
/// 1. Filter out events that do not require incremental rescans (e.g. `ReScan` / `Nop` variants
///    such as RootChanged or HistoryDone). Higher-level logic either rebuilds the cache or simply
///    updates the event id for those.
/// 2. Keep only `ScanType::SingleNode` and `ScanType::Folder` paths.
/// 3. Deduplicate ancestors and descendants:
///    - Skip a path if it is already covered by an ancestor (`path.starts_with(ancestor)`).
///    - When inserting an ancestor, remove all of its descendants that were previously added.
///    - Keep only a single entry for identical paths; later duplicates are considered covered.
/// 4. Return the minimal cover—the smallest set of paths whose rescans still cover every change.
///
/// Usage:
/// - `SearchCache::handle_fs_events` iterates over the returned paths and calls
///   `scan_path_recursive` on each of them to avoid redundant rescans of descendants or duplicates.
/// - High-frequency FSEvents often bubble many changes from the same subtree; merging them here
///   significantly reduces IO and metadata fetch work downstream.
///
/// Complexity:
/// - Approximately O(n log n + m * depth): sort by depth first, then scan linearly while checking
///   ancestors.
/// - If we ever need additional speed we can explore trie/prefix-tree structures.
///
/// Corner cases:
/// - Empty input → returns an empty `Vec`.
/// - Duplicate identical paths → only one is kept (later duplicates are skipped via `starts_with`).
/// - Child path seen before its ancestor → the ancestor replaces all children, so only the ancestor remains.
/// - Ancestor seen before its child → the child is skipped.
/// - Sibling paths never interfere with each other and are all kept.
/// - Paths that merely share prefixes (e.g. `/foo/bar` vs `/foo/barista`) are both retained because
///   `Path::starts_with` compares path components.
/// - Folder and `SingleNode` events participate together; we only look at the hierarchy.
/// - `PathBuf` values are compared as-is without normalisation, so symlinks are left untouched—the
///   caller must provide consistent inputs.
///
/// Result:
/// - Local benchmarks skipped rescans for 173,034 events out of 415,449.
fn scan_paths(events: Vec<FsEvent>) -> Vec<PathBuf> {
    let mut candidates: Vec<(PathBuf, usize)> = events
        .into_iter()
        .filter(|event| {
            // Sometimes there are ridiculous events assuming dir as file, so we always scan them as folder
            matches!(
                event.flag.scan_type(),
                ScanType::SingleNode | ScanType::Folder
            )
        })
        .map(|event| {
            let path = event.path;
            let depth = path_depth(&path);
            (path, depth)
        })
        .collect();

    candidates.sort_unstable_by(|(path_a, depth_a), (path_b, depth_b)| {
        depth_a.cmp(depth_b).then_with(|| path_a.cmp(path_b))
    });
    candidates.dedup_by(|(path_a, _), (path_b, _)| path_a == path_b);

    let mut selected = Vec::with_capacity(candidates.len());
    let mut selected_set = HashSet::with_capacity(candidates.len());
    for (path, _) in candidates {
        if has_selected_ancestor(&path, &selected_set) {
            continue;
        }
        selected_set.insert(path.clone());
        selected.push(path);
    }
    selected
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn has_selected_ancestor(path: &Path, selected: &HashSet<PathBuf>) -> bool {
    if selected.is_empty() {
        return false;
    }
    if selected.contains(path) {
        return true;
    }
    let mut ancestor = path.to_path_buf();
    while ancestor.pop() {
        if selected.contains(&ancestor) {
            return true;
        }
    }
    false
}

/// Error type for `SearchCache::handle_fs_event`.
#[derive(Debug)]
pub enum HandleFSEError {
    /// Full rescan is required.
    Rescan,
}

/// Note: This function is expected to be called with WalkData which metadata is not fetched.
fn construct_node_slab_name_index(
    parent: Option<SlabIndex>,
    node: &Node,
    slab: &mut ThinSlab<SlabNode>,
    name_index: &mut NameIndex,
) -> SlabIndex {
    let metadata = match node.metadata {
        Some(metadata) => SlabNodeMetadataCompact::some(metadata),
        None => SlabNodeMetadataCompact::none(),
    };
    let name = NAME_POOL.push(&node.name);
    let slab_node = SlabNode::new(parent, name, metadata);
    let index = slab.insert(slab_node);
    unsafe {
        // SAFETY: fswalk sorts each directory's children by name before we recurse,
        // so this preorder traversal visits nodes in lexicographic path order.
        name_index.add_index_ordered(name, index);
    }
    slab[index].children = node
        .children
        .iter()
        .map(|node| construct_node_slab_name_index(Some(index), node, slab, name_index))
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
        let name = NAME_POOL.push(&node.name);
        let slab_node = SlabNode::new(parent, name, metadata);
        let index = self.push_node(slab_node);
        self.file_nodes[index].children = node
            .children
            .iter()
            .map(|node| self.create_node_slab_update_name_index_and_name_pool(Some(index), node))
            .collect::<ThinVec<_>>();
        index
    }
}

pub static NAME_POOL: LazyLock<NamePool> = LazyLock::new(NamePool::new);

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use tempdir::TempDir;

    fn guard_indices(result: Result<Option<Vec<SlabIndex>>>) -> Vec<SlabIndex> {
        result
            .expect("search should succeed")
            .expect("noop cancellation token should not cancel")
    }

    fn guard_nodes(result: Result<Option<Vec<SearchResultNode>>>) -> Vec<SearchResultNode> {
        result
            .expect("query should succeed")
            .expect("noop cancellation token should not cancel")
    }

    fn query(cache: &mut SearchCache, query: impl Into<String>) -> Vec<SearchResultNode> {
        guard_nodes(cache.query_files(query.into(), CancellationToken::noop()))
    }

    fn make_node(name: &str, children: Vec<Node>) -> Node {
        Node {
            children,
            name: name.into(),
            metadata: None,
        }
    }

    fn make_leaf(name: &str) -> Node {
        make_node(name, vec![])
    }

    fn push_child(slab: &mut ThinSlab<SlabNode>, parent: SlabIndex, name: &str) -> SlabIndex {
        let idx = slab.insert(SlabNode::new(
            Some(parent),
            NAME_POOL.push(name),
            SlabNodeMetadataCompact::none(),
        ));
        slab[parent].children.push(idx);
        idx
    }

    fn manual_target_tree_file_nodes() -> (FileNodes, [SlabIndex; 3]) {
        let mut slab = ThinSlab::new();
        let root_idx = slab.insert(SlabNode::new(
            None,
            NAME_POOL.push("root"),
            SlabNodeMetadataCompact::none(),
        ));
        let alpha = push_child(&mut slab, root_idx, "alpha");
        let beta = push_child(&mut slab, root_idx, "beta");
        let root_target = push_child(&mut slab, root_idx, "target.txt");
        let alpha_target = push_child(&mut slab, alpha, "target.txt");
        let beta_target = push_child(&mut slab, beta, "target.txt");
        let file_nodes = FileNodes::new(PathBuf::from("/virtual/root"), slab, root_idx);
        (file_nodes, [root_target, alpha_target, beta_target])
    }

    #[test]
    fn test_construct_node_slab_name_index_preserves_path_order() {
        let tree = make_node(
            "root",
            vec![
                make_node("alpha", vec![make_leaf("shared")]),
                make_node("beta", vec![make_node("gamma", vec![make_leaf("shared")])]),
                make_leaf("shared"),
            ],
        );
        let mut slab = ThinSlab::new();
        let mut name_index = NameIndex::default();
        let root = construct_node_slab_name_index(None, &tree, &mut slab, &mut name_index);
        let file_nodes = FileNodes::new(PathBuf::from("/virtual/root"), slab, root);

        let shared_entries = name_index.get("shared").expect("shared entries");
        assert_eq!(shared_entries.len(), 3);
        let paths: Vec<PathBuf> = shared_entries
            .iter()
            .map(|index| file_nodes.node_path(*index).expect("path must exist"))
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(
            paths, sorted,
            "shared entries must follow lexicographic path order"
        );
    }

    #[test]
    fn test_name_index_add_index_sorts_paths() {
        let (file_nodes, targets) = manual_target_tree_file_nodes();
        let mut name_index = NameIndex::default();

        for &index in targets.iter().rev() {
            name_index.add_index("target.txt", index, &file_nodes);
        }

        let entries = name_index
            .get("target.txt")
            .expect("target.txt entries must exist");
        assert_eq!(entries.len(), 3);
        let paths: Vec<PathBuf> = entries
            .iter()
            .map(|index| file_nodes.node_path(*index).expect("path exists"))
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "add_index must maintain lexicographic order");
    }

    #[test]
    fn test_walk_fs_with_walk_data_preserves_name_index_order() {
        let temp_dir =
            TempDir::new("walk_fs_with_walk_data_orders").expect("Failed to create temp dir");
        let root = temp_dir.path();
        fs::create_dir(root.join("beta")).unwrap();
        fs::create_dir(root.join("alpha")).unwrap();
        fs::File::create(root.join("target.txt")).unwrap();
        fs::File::create(root.join("alpha/target.txt")).unwrap();
        fs::File::create(root.join("beta/target.txt")).unwrap();

        let walk_data = WalkData::simple(false);
        let cache = SearchCache::walk_fs_with_walk_data(root.to_path_buf(), &walk_data, None, None)
            .expect("walk cache");

        let entries = cache
            .name_index
            .get("target.txt")
            .expect("target.txt entries");
        assert_eq!(entries.len(), 3);
        let paths: Vec<PathBuf> = entries
            .iter()
            .map(|index| cache.file_nodes.node_path(*index).expect("path exists"))
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(
            paths, sorted,
            "walk_fs_with_walk_data must yield lexicographically ordered slab indices"
        );
    }

    #[test]
    fn test_search_cache_walk_and_verify() {
        let temp_dir = TempDir::new("test_cache").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        fs::create_dir_all(temp_path.join("subdir")).expect("Failed to create subdirectory");
        fs::File::create(temp_path.join("file1.txt")).expect("Failed to create file");
        fs::File::create(temp_path.join("subdir/file2.txt")).expect("Failed to create file");

        let cache = SearchCache::walk_fs(temp_path.to_path_buf());

        assert_eq!(cache.file_nodes.len(), 4);
        assert_eq!(cache.name_index.len(), 4);
    }

    #[test]
    fn test_handle_fs_event_add() {
        // Create a temporary directory.
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events).unwrap();

        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_add_before_search() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events).unwrap();

        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    // Processing outdated fs event is required to avoid bouncing.
    #[test]
    fn test_handle_outdated_fs_event() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id.saturating_sub(1),
            flag: EventFlag::ItemCreated,
        }];

        cache.handle_fs_events(mock_events).unwrap();

        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_search_with_regex_option() {
        let temp_dir = TempDir::new("test_search_regex_option").unwrap();
        let dir = temp_dir.path();

        fs::File::create(dir.join("foo123.txt")).unwrap();
        fs::File::create(dir.join("bar.txt")).unwrap();

        let mut cache = SearchCache::walk_fs(dir.to_path_buf());
        let opts = SearchOptions {
            use_regex: true,
            case_insensitive: false,
        };
        let indices =
            guard_indices(cache.search_with_options("foo\\d+", opts, CancellationToken::noop()));
        assert_eq!(indices.len(), 1);
        let nodes = cache.expand_file_nodes(&indices);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].path.ends_with("foo123.txt"));

        // ensure other names are not matched
        let opts = SearchOptions {
            use_regex: true,
            case_insensitive: false,
        };
        let miss =
            guard_indices(cache.search_with_options("bar\\d+", opts, CancellationToken::noop()));
        assert!(miss.is_empty());
    }

    #[test]
    fn test_search_case_insensitive_option() {
        let temp_dir = TempDir::new("test_search_case_insensitive_option").unwrap();
        let dir = temp_dir.path();

        fs::File::create(dir.join("Alpha.TXT")).unwrap();
        fs::File::create(dir.join("beta.txt")).unwrap();

        let mut cache = SearchCache::walk_fs(dir.to_path_buf());
        let opts = SearchOptions {
            use_regex: false,
            case_insensitive: true,
        };
        let indices =
            guard_indices(cache.search_with_options("alpha.txt", opts, CancellationToken::noop()));
        assert_eq!(indices.len(), 1);
        let nodes = cache.expand_file_nodes(&indices);
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].path.ends_with("Alpha.TXT"));

        let opts = SearchOptions {
            use_regex: false,
            case_insensitive: true,
        };
        let miss =
            guard_indices(cache.search_with_options("gamma.txt", opts, CancellationToken::noop()));
        assert!(miss.is_empty());
    }

    #[test]
    fn test_search_empty_cancelled_returns_none() {
        let temp_dir = TempDir::new("search_empty_cancelled").unwrap();
        fs::File::create(temp_dir.path().join("alpha.txt")).unwrap();
        let cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        let token = CancellationToken::new(1000);
        let _ = CancellationToken::new(1001);

        assert!(cache.search_empty(token).is_none());
    }

    #[test]
    fn test_search_with_options_cancelled_returns_none() {
        let temp_dir = TempDir::new("search_with_options_cancelled").unwrap();
        fs::File::create(temp_dir.path().join("file_a.txt")).unwrap();
        let cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        let token = CancellationToken::new(2000);
        let _ = CancellationToken::new(2001);

        let result = cache.search_with_options(
            "file_a",
            SearchOptions {
                use_regex: false,
                case_insensitive: false,
            },
            token,
        );
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_query_files_cancelled_returns_none() {
        let temp_dir = TempDir::new("query_files_cancelled").unwrap();
        fs::File::create(temp_dir.path().join("item.txt")).unwrap();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        let token = CancellationToken::new(3000);
        let _ = CancellationToken::new(3001);

        let result = cache.query_files("item.txt".to_string(), token);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_handle_fs_event_removal() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);

        fs::remove_file(temp_path.join("new_file.txt")).expect("Failed to remove file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved,
        }];

        cache.handle_fs_events(mock_events).unwrap();

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.file_nodes.len(), 1);
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
            cache.file_nodes.len(),
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

        assert_eq!(cache.file_nodes.len(), 1);
        assert_eq!(cache.name_index.len(), 1);

        fs::File::create(temp_path.join("new_file.txt")).expect("Failed to create file");

        let mock_events = vec![FsEvent {
            path: temp_path.join("new_file.txt"),
            id: cache.last_event_id + 1,
            flag: EventFlag::ItemRemoved,
        }];

        cache.handle_fs_events(mock_events).unwrap();

        // Though the file in fsevents removed, we should still preserve it since it exists on disk.
        assert_eq!(cache.file_nodes.len(), 2);
        assert_eq!(cache.name_index.len(), 2);
        assert_eq!(cache.search("new_file.txt").unwrap().len(), 1);
    }

    #[test]
    fn test_handle_fs_event_add_and_removal() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 1);
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
        assert_eq!(cache.file_nodes.len(), 2);
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

        assert_eq!(cache.file_nodes.len(), 7);
        assert_eq!(cache.name_index.len(), 7);

        let mock_events = vec![FsEvent {
            path: temp_path.to_path_buf(),
            id: cache.last_event_id + 1,
            flag: EventFlag::RootChanged,
        }];

        cache.handle_fs_events(mock_events).unwrap_err();

        assert_eq!(cache.file_nodes.len(), 7);
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

        assert_eq!(cache.file_nodes.len(), 1);
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
        assert_eq!(cache.file_nodes.len(), 1);
        assert_eq!(cache.name_index.len(), 1);
    }

    #[test]
    fn test_handle_fs_event_rescan_by_modify() {
        let temp_dir = TempDir::new("test_events").expect("Failed to create temp directory");
        let temp_path = temp_dir.path();
        let mut cache = SearchCache::walk_fs(temp_dir.path().to_path_buf());

        assert_eq!(cache.file_nodes.len(), 1);
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

        assert_eq!(cache.file_nodes.len(), 1);
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

        assert_eq!(cache.file_nodes.len(), 11);
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

        assert_eq!(cache.file_nodes.len(), 5);
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

        assert_eq!(cache.file_nodes.len(), 11);
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

        assert_eq!(cache.file_nodes.len(), 9);
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

        // Directory nodes should always carry metadata.
        assert!(cache.file_nodes[cache.file_nodes.root()].metadata.is_some());

        // Check metadata for a file node
        let file_nodes = cache
            .search("file1.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1, "Expected 1 node for file1.txt");
        let file_node_idx = file_nodes.into_iter().next().unwrap();
        // File nodes should always have `metadata` set to `None`.
        assert!(
            cache.file_nodes[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs_new should be None"
        );

        // Check metadata for a file node
        let file_nodes = cache
            .search("file2.txt")
            .expect("Search for file1.txt failed");
        assert_eq!(file_nodes.len(), 1);
        let file_node_idx = file_nodes.into_iter().next().unwrap();
        // File nodes should always have `metadata` set to `None`.
        assert!(
            cache.file_nodes[file_node_idx].metadata.is_none(),
            "Metadata for file node created by walk_fs_new should be None"
        );

        // Check metadata for a subdirectory node
        let dir_nodes = cache.search("subdir1").expect("Search for subdir1 failed");
        assert_eq!(dir_nodes.len(), 1, "Expected 1 node for subdir1");
        let dir_node_idx = dir_nodes.into_iter().next().unwrap();
        // Directory nodes should always carry metadata.
        assert!(
            cache.file_nodes[dir_node_idx].metadata.is_some(),
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
        let file_slab_meta = cache.file_nodes[file_node_idx]
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
        let dir_slab_meta = cache.file_nodes[dir_node_idx]
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
        let file_in_subdir_slab_meta = cache.file_nodes[file_in_subdir_node_idx]
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
        let results1 = query(&mut cache, "file_a.txt");
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
        let results2 = query(&mut cache, "file_c.md");
        assert_eq!(results2.len(), 1);
        assert!(
            results2[0].path.ends_with("dir_b/file_c.md"),
            "Path was: {:?}",
            results2[0].path
        );
        assert!(results2[0].metadata.is_none());

        // 3. Query for a directory
        let results3 = query(&mut cache, "dir_b");
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
        let results4 = query(&mut cache, "non_existent.zip");
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
        let results5 = query(&mut cache, "file_a");
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
        let results6 = query(&mut cache, "dir_b/file_c");
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

        let results = query(&mut cache, root_dir_name.to_string());
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
        // Empty queries match everything.
        let result = cache.query_files("".to_string(), CancellationToken::noop());
        assert!(result.is_ok(), "empty query should succeed");
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
        let results_deep_file = query(&mut cache, "gamma_file.txt");
        assert_eq!(results_deep_file.len(), 1);
        let expected_suffix_deep = "alpha_dir/beta_subdir/gamma_file.txt".to_string();
        assert!(
            results_deep_file[0].path.ends_with(&expected_suffix_deep),
            "Path was: {:?}",
            results_deep_file[0].path
        );

        // Query for intermediate directory
        let results_sub1 = query(&mut cache, "alpha_dir");
        assert_eq!(results_sub1.len(), 1);
        assert!(
            results_sub1[0].path.ends_with("alpha_dir"),
            "Path was: {:?}",
            results_sub1[0].path
        );

        // Query for nested intermediate directory
        let results_sub2 = query(&mut cache, "beta_subdir");
        assert_eq!(results_sub2.len(), 1);
        assert!(
            results_sub2[0].path.ends_with("alpha_dir/beta_subdir"),
            "Path was: {:?}",
            results_sub2[0].path
        );

        // Test multi-segment query for the deep file
        let results_multi_segment = query(&mut cache, "alpha_dir/beta_subdir/gamma_file");
        assert_eq!(results_multi_segment.len(), 1);
        assert!(
            results_multi_segment[0]
                .path
                .ends_with(&expected_suffix_deep),
            "Path was: {:?}",
            results_multi_segment[0].path
        );

        // Test multi-segment query for an intermediate directory
        let results_multi_segment_dir = query(&mut cache, "alpha_dir/beta_subdir");
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
    fn test_boolean_queries() {
        let temp_dir = TempDir::new("test_boolean_queries").unwrap();
        let root = temp_dir.path();
        fs::File::create(root.join("foo.txt")).unwrap();
        fs::File::create(root.join("bar.txt")).unwrap();
        fs::File::create(root.join("foobar.txt")).unwrap();

        let mut cache = SearchCache::walk_fs(root.to_path_buf());

        let results_and = query(&mut cache, "foo bar");
        assert_eq!(results_and.len(), 1);
        assert!(
            results_and[0].path.ends_with("foobar.txt"),
            "AND query should keep files matching both terms"
        );

        let mut names_or: Vec<_> = query(&mut cache, "foo|bar")
            .into_iter()
            .filter_map(|node| {
                node.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        names_or.sort();
        assert!(
            names_or.contains(&"foo.txt".to_string())
                && names_or.contains(&"bar.txt".to_string())
                && names_or.contains(&"foobar.txt".to_string()),
            "OR query should include any matching term. Found: {names_or:?}"
        );

        let excluded = query(&mut cache, "!foo");
        assert!(
            excluded.iter().all(|node| !node.path.ends_with("foo.txt")),
            "NOT query should exclude foo.txt"
        );
        assert!(
            excluded.iter().any(|node| node.path.ends_with("bar.txt")),
            "NOT query should keep unrelated files"
        );
    }

    #[test]
    fn test_type_filters() {
        let temp_dir = TempDir::new("test_type_filters").unwrap();
        let root = temp_dir.path();
        fs::create_dir(root.join("alpha_dir")).unwrap();
        fs::File::create(root.join("alpha_dir/file_a.txt")).unwrap();
        fs::File::create(root.join("beta.txt")).unwrap();

        let mut cache = SearchCache::walk_fs(root.to_path_buf());

        let files = query(&mut cache, "file:beta");
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("beta.txt"));

        let folders = query(&mut cache, "folder:alpha");
        assert_eq!(folders.len(), 1);
        assert!(folders[0].path.ends_with("alpha_dir"));
    }

    #[test]
    fn test_extension_and_path_filters() {
        let temp_dir = TempDir::new("test_extension_filters").unwrap();
        let root = temp_dir.path();
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::File::create(root.join("top.txt")).unwrap();
        fs::File::create(root.join("top.md")).unwrap();
        fs::File::create(nested.join("child.txt")).unwrap();

        let mut cache = SearchCache::walk_fs(root.to_path_buf());

        let txt_results = query(&mut cache, "ext:txt");
        let mut txt_paths: Vec<_> = txt_results
            .into_iter()
            .filter_map(|node| {
                node.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        txt_paths.sort();
        assert_eq!(
            txt_paths,
            vec!["child.txt".to_string(), "top.txt".to_string()]
        );

        let parent_query = format!(r#"parent:"{}""#, root.to_string_lossy());
        let direct_children = query(&mut cache, parent_query);
        let mut child_names: Vec<_> = direct_children
            .into_iter()
            .filter_map(|node| {
                node.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        child_names.sort();
        assert!(
            child_names.contains(&"top.txt".to_string())
                && child_names.contains(&"top.md".to_string()),
            "parent: filter should return direct children"
        );

        let infolder_query = format!(r#"infolder:"{}""#, nested.to_string_lossy());
        let infolder_results = query(&mut cache, infolder_query);
        assert_eq!(infolder_results.len(), 1);
        assert!(infolder_results[0].path.ends_with("nested/child.txt"));
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
        let results_file_walk = query(&mut cache, "walk_file.txt");
        assert_eq!(results_file_walk.len(), 1);
        assert!(
            results_file_walk[0].metadata.is_none(),
            "File metadata from walk_fs_new should be None"
        );

        let results_dir_walk = query(&mut cache, "walk_dir");
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

        let results_event_file = query(&mut cache, "event_added_file.txt");
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

        let results_event_dir = query(&mut cache, "event_added_dir");
        assert_eq!(results_event_dir.len(), 1);
        assert!(
            results_event_dir[0].metadata.is_some(),
            "Dir metadata should be Some after event processing for dir"
        );

        let results_file_in_event_dir = query(&mut cache, "inner_event.dat");
        assert_eq!(results_file_in_event_dir.len(), 1);
        let inner_file_meta = results_file_in_event_dir[0]
            .metadata
            .as_ref()
            .expect("File in event-added dir metadata should be Some");
        assert_eq!(inner_file_meta.size(), 4);
    }

    // --- scan_paths focused tests ---
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
            }, // Assume the flag is incorrect; treat it as SingleNode anyway.
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
        // Expect the ancestor /t/a to absorb the whole subtree.
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
        // /foo/bar and /foo/barista share a prefix but are not parent/child; both should stay.
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
        // Directory creation plus file modification: the directory should absorb its child.
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
    fn test_scan_paths_depth_then_lexicographic_ordering() {
        let events = vec![
            FsEvent {
                path: PathBuf::from("/z/child"),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/a"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: PathBuf::from("/m"),
                id: 3,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
            FsEvent {
                path: PathBuf::from("/a/child"),
                id: 4,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
        ];
        let out = scan_paths(events);
        assert_eq!(
            out,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/m"),
                PathBuf::from("/z/child")
            ]
        );
    }

    #[test]
    fn test_scan_paths_handles_root_ancestor() {
        let events = vec![
            FsEvent {
                path: PathBuf::from("/foo/bar"),
                id: 1,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsFile,
            },
            FsEvent {
                path: PathBuf::from("/"),
                id: 2,
                flag: EventFlag::ItemCreated | EventFlag::ItemIsDir,
            },
        ];
        let out = scan_paths(events);
        assert_eq!(out, vec![PathBuf::from("/")]);
    }

    #[test]
    fn test_scan_paths_large_chain_collapse() {
        // Build a long chain where the ancestor arrives at the end.
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
        // Add the real ancestor /long.
        events.push(FsEvent {
            path: PathBuf::from("/long"),
            id: 99,
            flag: EventFlag::ItemModified | EventFlag::ItemIsDir,
        });
        let out = scan_paths(events);
        assert_eq!(out, vec![PathBuf::from("/long")]);
    }
}
