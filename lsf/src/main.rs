mod query;

use anyhow::{Context, Result};
use bincode::{Decode, Encode, config::Configuration};
use clap::Parser;
use fswalk::{Node, WalkData, walk_it};
use namepool::NamePool;
use query::{Segment, query_segmentation};
use serde::{Deserialize, Serialize};
use slab::Slab;
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, File, Metadata},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    thread::available_parallelism,
    time::{Instant, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Encode, Decode)]
struct SlabNode {
    parent: Option<usize>,
    children: Vec<usize>,
    name: String,
}

impl SlabNode {
    /// Get the path of the node in the slab.
    pub fn path(&self, slab: &Slab<SlabNode>) -> String {
        let mut segments = vec![self.name.clone()];
        // Write code like this to avoid the root node, which has no node name and shouldn't be put into semgents.
        if let Some(mut parent) = self.parent {
            while let Some(new_parent) = slab[parent].parent {
                segments.push(slab[parent].name.clone());
                parent = new_parent
            }
        }
        let mut result = String::new();
        for segment in segments.into_iter().rev() {
            result.push('/');
            result.push_str(&segment);
        }
        result
    }
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
    // 先多线程构建树形文件名列表(不能直接创建 slab 因为 slab 无法多线程构建)
    let walk_data = WalkData::with_ignore_directory(PathBuf::from("/System/Volumes/Data"));
    let visit_time = Instant::now();
    let node = walk_it(PathBuf::from("/"), &walk_data).expect("failed to walk");
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

#[derive(Encode, Decode)]
struct PersistentStorage {
    // slab_root: usize,
    slab: Slab<SlabNode>,
    name_index: BTreeMap<String, Vec<usize>>,
}

const CACHE_PATH: &str = "target/cache.zstd";
const CACHE_TMP_PATH: &str = "target/cache.zstd.tmp";
const BINCODE_CONDFIG: Configuration = bincode::config::standard();

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (slab, name_index) = if cli.refresh || !Path::new(CACHE_PATH).exists() {
        let (_slab_root, slab) = walkfs_to_slab();
        let name_index = name_index(&slab);
        (slab, name_index)
    } else {
        let read_cache = || -> Result<_> {
            let cache_decode_time = Instant::now();
            let input = File::open(CACHE_PATH).context("Failed to open cache file")?;
            let input = zstd::Decoder::new(input).context("Failed to create zstd decoder")?;
            let mut input = BufReader::new(input);
            let slab: PersistentStorage =
                bincode::decode_from_std_read(&mut input, BINCODE_CONDFIG)
                    .context("Failed to decode cache")?;
            dbg!(cache_decode_time.elapsed());
            Ok((slab.slab, slab.name_index))
        };
        read_cache().unwrap_or_else(|e| {
            eprintln!("Failed to read cache: {:?}", e);
            let (_slab_root, slab) = walkfs_to_slab();
            let name_index = name_index(&slab);
            (slab, name_index)
        })
    };
    let name_pool = name_pool(&name_index);

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!(">");
        stdout.flush().unwrap();
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();
        let line = line.trim();
        if line.is_empty() {
            continue;
        } else if line == "/bye" {
            break;
        }
        if line.contains('/') {
            let segments = query_segmentation(line);
            let search_time = Instant::now();
            let mut node_set: Option<Vec<usize>> = None;
            if segments.is_empty() {
                eprintln!("unprocessed query: {:?}", segments);
                continue;
            }
            for segment in &segments {
                if let Some(nodes) = &node_set {
                    let mut new_node_set = Vec::with_capacity(nodes.len());
                    for &node in nodes {
                        let childs = &slab[node].children;
                        for child in childs {
                            if match segment {
                                Segment::Substr(substr) => slab[*child].name.contains(*substr),
                                Segment::Prefix(prefix) => slab[*child].name.starts_with(*prefix),
                                Segment::Exact(exact) => slab[*child].name == *exact,
                                Segment::Suffix(suffix) => slab[*child].name.ends_with(*suffix),
                            } {
                                new_node_set.push(*child);
                            }
                        }
                    }
                    node_set = Some(new_node_set);
                } else {
                    let names: Vec<_> = match segment {
                        Segment::Substr(substr) => name_pool.search_substr(*substr).collect(),
                        Segment::Prefix(prefix) => {
                            let mut buffer = vec![0u8];
                            buffer.extend_from_slice(prefix.as_bytes());
                            name_pool.search_prefix(&buffer).collect()
                        }
                        Segment::Exact(exact) => {
                            let mut buffer = vec![0u8];
                            buffer.extend_from_slice(exact.as_bytes());
                            buffer.push(0);
                            name_pool.search_exact(&buffer).collect()
                        }
                        Segment::Suffix(suffix) => {
                            // Query contains nul is very rare
                            let suffix = CString::new(*suffix).expect("Query contains nul");
                            name_pool.search_suffix(&suffix).collect()
                        }
                    };
                    let mut nodes = Vec::with_capacity(names.len());
                    for name in names {
                        nodes.extend_from_slice(
                            name_index
                                .get(name)
                                .context("Name index or name pool corrupted")?,
                        );
                    }
                    node_set = Some(nodes);
                }
            }
            let search_time = search_time.elapsed();
            for (i, node) in node_set.unwrap().into_iter().enumerate() {
                println!("[{}] {}", i, slab[node].path(&slab));
            }
            dbg!(search_time);
        } else {
            // Search out all leafs that contain the substring
            // e.g. "foo": ["/System/foo", "/System/Library/aaafoo"]
            // "/System/Library/aaafool/heck" won't be presented
            let search_time = Instant::now();
            for (i, name) in name_pool.search_substr(line).enumerate() {
                // TODO(ldm0): this can be parallelized
                if let Some(nodes) = name_index.get(name) {
                    for &node in nodes {
                        println!("[{}] {}", i, slab[node].path(&slab));
                    }
                }
            }
            dbg!(search_time.elapsed());
        }
    }

    {
        let cache_encode_time = Instant::now();
        {
            let output = File::create(CACHE_TMP_PATH).context("Failed to create cache file")?;
            let mut output =
                zstd::Encoder::new(output, 6).context("Failed to create zstd encoder")?;
            output
                .multithread(available_parallelism().map(|x| x.get() as u32).unwrap_or(4))
                .context("Failed to create parallel zstd encoder")?;
            let output = output.auto_finish();
            let mut output = BufWriter::new(output);
            bincode::encode_into_std_write(
                &PersistentStorage { slab, name_index },
                &mut output,
                BINCODE_CONDFIG,
            )
            .context("Failed to encode cache")?;
        }
        fs::rename(CACHE_TMP_PATH, CACHE_PATH).unwrap();
        dbg!(cache_encode_time.elapsed());
        dbg!(fs::metadata(CACHE_PATH).unwrap().len() / 1024 / 1024);
    }
    Ok(())
}

// TODO(ldm0):
// - file removal routine
// - file addition routine
// - segment search cache(same search routine will be triggered while user is typing, should cache exact[..], suffix, suffix/exact[..])
// [] tui?
// - lazy metadata design
