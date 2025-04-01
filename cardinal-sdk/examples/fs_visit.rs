use cardinal_sdk::fs_visit::{Node, WalkData, walk_it};
use radix_trie::{Trie, TrieCommon};
use std::{
    fs::{self, File},
    io::BufWriter,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

#[derive(Default)]
struct NamePool {
    pool: Vec<u8>,
}

impl NamePool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.pool.len()
    }

    pub fn push(&mut self, name: &str) -> usize {
        let start = self.pool.len();
        self.pool.extend_from_slice(name.as_bytes());
        self.pool.push(0);
        start
    }

    fn get(&self, offset: usize) -> &str {
        let begin = self.pool[..offset]
            .iter()
            .rposition(|&x| x == 0)
            .map(|x| x + 1)
            .unwrap_or(0);
        let end = self.pool[offset..]
            .iter()
            .position(|&x| x == 0)
            .unwrap_or(self.pool.len() - offset);
        unsafe { std::str::from_utf8_unchecked(&self.pool[begin..offset + end]) }
    }

    pub fn search_substr<'a>(&'a self, substr: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        memchr::memmem::find_iter(&self.pool, substr.as_bytes()).map(|x| self.get(x))
    }
}

fn construct_trie_and_namepool(
    node: &Arc<Node>,
    node_trie: &mut Trie<String, Vec<Arc<Node>>>,
    name_pool: &mut NamePool,
) {
    if let Some(nodes) = node_trie.get_mut(&node.name) {
        nodes.push(node.clone());
    } else {
        name_pool.push(&node.name);
        node_trie.insert(node.name.clone(), vec![node.clone()]);
    };
    for child in &node.children {
        if let Some(nodes) = node_trie.get_mut(&child.name) {
            nodes.push(child.clone());
        } else {
            name_pool.push(&child.name);
            node_trie.insert(child.name.clone(), vec![child.clone()]);
        };
        for grandchild in &child.children {
            construct_trie_and_namepool(&grandchild, node_trie, name_pool);
        }
    }
}

fn main() {
    let walk_data = WalkData::default();
    let visit_time = Instant::now();
    let node = walk_it(PathBuf::from("/"), &walk_data).expect("failed to walk");
    let node = Arc::new(node);
    dbg!(walk_data);
    dbg!(visit_time.elapsed());

    {
        let cache_time = Instant::now();
        let mut node_trie = Trie::new();
        let mut name_pool = NamePool::new();
        construct_trie_and_namepool(&node, &mut node_trie, &mut name_pool);
        dbg!(cache_time.elapsed());
        dbg!(node_trie.len());

        let search_time = Instant::now();
        for (i, name) in name_pool.search_substr("athbyt").enumerate() {
            if let Some(subtrie) = node_trie.subtrie(name) {
                for (key, _) in subtrie.iter() {
                    println!("[{}] key: {}", i, key);
                }
            }
        }
        dbg!(name_pool.len() / 1024 / 1024);
        dbg!(search_time.elapsed());
    }

    {
        let cbor_time = Instant::now();
        let output = File::create("target/tree.cbor").unwrap();
        let mut output = BufWriter::new(output);
        cbor4ii::serde::to_writer(&mut output, &node).unwrap();
        dbg!(cbor_time.elapsed());
        dbg!(fs::metadata("target/tree.cbor").unwrap().len() / 1024 / 1024);
    }

    {
        let bincode_time = Instant::now();
        let output = File::create("target/tree.bin").unwrap();
        let mut output = BufWriter::new(output);
        bincode::encode_into_std_write(&node, &mut output, bincode::config::standard()).unwrap();
        dbg!(bincode_time.elapsed());
        dbg!(fs::metadata("target/tree.bin").unwrap().len() / 1024 / 1024);
    }

    {
        let zstd_bincode_time = Instant::now();
        let output = File::create("target/tree.bin.zstd").unwrap();
        let mut output = zstd::Encoder::new(output, 3).unwrap();
        bincode::encode_into_std_write(&node, &mut output, bincode::config::standard()).unwrap();
        dbg!(zstd_bincode_time.elapsed());
        dbg!(fs::metadata("target/tree.bin.zstd").unwrap().len() / 1024 / 1024);
    }
}
