mod cache;
mod query;

use anyhow::{Context, Result, bail};
use bincode::{Decode, Encode};
use cache::{PersistentStorage, cache_exists, read_cache_from_file, write_cache_to_file};
use clap::Parser;
use fswalk::{Node, WalkData, walk_it};
use namepool::NamePool;
use query::{Segment, query_segmentation};
use serde::{Deserialize, Serialize};
use slab::Slab;
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::Metadata,
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Encode, Decode)]
struct SlabNode {
    parent: Option<usize>,
    children: Vec<usize>,
    name: String,
}

pub struct SlabNodeData {
    pub name: String,
    pub ctime: Option<u64>,
    pub mtime: Option<u64>,
}

impl SlabNodeData {
    pub fn new(name: String, metadata: &Option<Metadata>) -> Self {
        let (ctime, mtime) = match metadata {
            Some(metadata) => ctime_mtime_from_metadata(metadata),
            None => (None, None),
        };
        Self { name, ctime, mtime }
    }
}

fn ctime_mtime_from_metadata(metadata: &Metadata) -> (Option<u64>, Option<u64>) {
    // TODO(ldm0): is this fast enough?
    let ctime = metadata
        .created()
        .ok()
        .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
        .map(|x| x.as_secs());
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|x| x.duration_since(UNIX_EPOCH).ok())
        .map(|x| x.as_secs());
    (ctime, mtime)
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

/// Combine the construction routine of NamePool and BTreeMap since we can deduplicate node name for free.
// TODO(ldm0): Memory optimization can be done by letting name index reference the name in the pool(gc need to be considered though)
fn construct_name_index(slab: &Slab<SlabNode>, name_index: &mut BTreeMap<String, Vec<usize>>) {
    // The slab is newly constructed, thus though slab.iter() iterates all slots, it won't waste too much.
    for (i, node) in slab.iter() {
        if let Some(nodes) = name_index.get_mut(&node.name) {
            nodes.push(i);
        } else {
            name_index.insert(node.name.clone(), vec![i]);
        };
    }
}

#[derive(Parser)]
struct Cli {
    #[clap(short, long, default_value = "false")]
    /// Open enabled, cache was ignored and filesystem will be rewalked.
    refresh: bool,
}

fn walkfs_to_slab() -> (usize, Slab<SlabNode>) {
    // 先多线程构建树形文件名列表(不能直接创建 slab 因为 slab 无法多线程构建(slab 节点有相互引用，不想加锁))
    let walk_data = WalkData::with_ignore_directory(PathBuf::from("/System/Volumes/Data"));
    let visit_time = Instant::now();
    let node = walk_it(&Path::new("/"), &walk_data).expect("failed to walk");
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
    let name_index_time = Instant::now();
    let mut name_index = BTreeMap::default();
    construct_name_index(&slab, &mut name_index);
    dbg!(name_index_time.elapsed());
    println!("name index len: {}", name_index.len());
    name_index
}

fn name_pool(name_index: &BTreeMap<String, Vec<usize>>) -> NamePool {
    let name_pool_time = Instant::now();
    let mut name_pool = NamePool::new();
    for name in name_index.keys() {
        name_pool.push(name);
    }
    dbg!(name_pool_time.elapsed());
    println!("name pool size: {}MB", name_pool.len() / 1024 / 1024);
    name_pool
}

struct SearchCache {
    slab_root: usize,
    slab: Slab<SlabNode>,
    name_index: BTreeMap<String, Vec<usize>>,
    name_pool: NamePool,
}

impl SearchCache {
    fn new(
        slab_root: usize,
        slab: Slab<SlabNode>,
        name_index: BTreeMap<String, Vec<usize>>,
    ) -> Self {
        // name pool construction speed is fast enough that caching it doesn't worth it.
        let name_pool = name_pool(&name_index);
        Self {
            slab_root,
            slab,
            name_index,
            name_pool,
        }
    }

    fn search(&self, line: &str) -> Result<Vec<usize>> {
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
                    Segment::Substr(substr) => self.name_pool.search_substr(*substr).collect(),
                    Segment::Prefix(prefix) => {
                        let mut buffer = vec![0u8];
                        buffer.extend_from_slice(prefix.as_bytes());
                        self.name_pool.search_prefix(&buffer).collect()
                    }
                    Segment::Exact(exact) => {
                        let mut buffer = vec![0u8];
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
                node_set = Some(nodes);
            }
        }
        let search_time = search_time.elapsed();
        dbg!(search_time);
        // Safety: node_set can't be None since segments is not empty.
        Ok(node_set.unwrap())
    }

    /// Get the path of the node in the slab.
    fn node_path(&self, index: usize) -> String {
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
        for segment in segments.into_iter().rev() {
            result.push('/');
            result.push_str(&segment);
        }
        result
    }

    fn push_node(&mut self, node: SlabNode) {
        let node_name = node.name.clone();
        let index = self.slab.insert(node);
        if let Some(indexes) = self.name_index.get_mut(&node_name) {
            indexes.push(index);
        } else {
            self.name_pool.push(&node_name);
            self.name_index.insert(node_name, vec![index]);
        }
    }

    fn create_dir_all(&mut self, path: &Path) -> Result<()> {
        // self.slab
        todo!()
    }

    fn scan_dir(&mut self, path: &Path) -> Result<()> {
        /*
        let walk_data = WalkData::new();
        let node = walk_it(path, &walk_data);
        if let Some(node) = node {
            self.push
        }
         */
        todo!()
    }

    fn scan_file(&mut self, path: PathBuf) -> Result<()> {
        todo!()
    }

    fn remove_node(&mut self, index: usize) {
        if let Some(node) = self.slab.try_remove(index) {
            let indexes = self
                .name_index
                .get_mut(&node.name)
                .expect("inconsistent name index and node");
            indexes.retain(|&x| x != index);
            if indexes.is_empty() {
                self.name_index.remove(&node.name);
                // TODO(ldm0): actually we need to remove name in the name pool,
                // but currently name pool doesn't support remove. (GC is needed for name pool)
                // self.name_pool.remove(&node.name);
            }
        }
    }

    fn into_persistent_storage(self) -> PersistentStorage {
        PersistentStorage {
            slab_root: self.slab_root,
            slab: self.slab,
            name_index: self.name_index,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cache = if cli.refresh || !cache_exists() {
        println!("Walking filesystem...");
        let (slab_root, slab) = walkfs_to_slab();
        let name_index = name_index(&slab);
        SearchCache::new(slab_root, slab, name_index)
    } else {
        println!("Reading cache...");
        read_cache_from_file()
            .map(
                |PersistentStorage {
                     slab_root,
                     slab,
                     name_index,
                 }| SearchCache::new(slab_root, slab, name_index),
            ) // 从 PersistentStorage 中解构
            .unwrap_or_else(|e| {
                eprintln!("Failed to read cache: {:?}. Re-walking filesystem...", e);
                let (slab_root, slab) = walkfs_to_slab();
                let name_index = name_index(&slab);
                SearchCache::new(slab_root, slab, name_index)
            })
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("> ");
        stdout.flush().unwrap();
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();
        let line = line.trim();
        if line.is_empty() {
            continue;
        } else if line == "/bye" {
            break;
        }

        match cache.search(line) {
            Ok(node_set) => {
                for (i, node) in node_set.into_iter().enumerate() {
                    println!("[{}] {}", i, cache.node_path(node));
                }
            }
            Err(e) => {
                eprintln!("{:?}", e);
            }
        }
    }

    write_cache_to_file(cache.into_persistent_storage()).context("Write cache to file failed.")?;
    Ok(())
}

// TODO(ldm0):
// - file removal routine
// - file addition routine
// - segment search cache(same search routine will be triggered while user is typing, should cache exact[..], suffix, suffix/exact[..])
// [] tui?
// - lazy metadata design
// 或许最后可以在首次扫描过程中就把中间结果 在索引逻辑和搜索逻辑之间抛来抛去，做到边索引边搜索
