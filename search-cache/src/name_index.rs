use crate::{NAME_POOL, SlabIndex, SlabNode, ThinSlab};
use hashbrown::HashSet;
use std::{collections::BTreeMap, time::Instant};
use tracing::info;

#[derive(Clone, Default)]
pub struct NameIndex {
    map: BTreeMap<&'static str, HashSet<SlabIndex>>,
}

impl NameIndex {
    pub fn new(map: BTreeMap<&'static str, HashSet<SlabIndex>>) -> Self {
        Self { map }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn all_indices(&self) -> Vec<SlabIndex> {
        self.map
            .values()
            .flat_map(|indices| indices.iter().copied())
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&HashSet<SlabIndex>> {
        self.map.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut HashSet<SlabIndex>> {
        self.map.get_mut(name)
    }

    pub fn insert_owned(&mut self, name: &'static str, indices: HashSet<SlabIndex>) {
        self.map.insert(name, indices);
    }

    pub fn add_index(&mut self, name: &str, index: SlabIndex) {
        if let Some(existing) = self.map.get_mut(name) {
            existing.insert(index);
        } else {
            let mut indices = HashSet::with_capacity(1);
            indices.insert(index);
            let interned = NAME_POOL.push(name);
            self.map.insert(interned, indices);
        }
    }

    pub fn remove_index(&mut self, name: &str, index: SlabIndex) -> bool {
        let Some(indices) = self.map.get_mut(name) else {
            return false;
        };
        let existed = indices.remove(&index);
        if indices.is_empty() {
            self.map.remove(name);
        }
        existed
    }

    pub fn remove(&mut self, name: &str) -> Option<HashSet<SlabIndex>> {
        self.map.remove(name)
    }

    pub fn into_persistent(self) -> BTreeMap<Box<str>, HashSet<SlabIndex>> {
        self.map
            .into_iter()
            .map(|(name, indices)| (name.to_string().into_boxed_str(), indices))
            .collect()
    }

    pub fn construct_name_pool(data: BTreeMap<Box<str>, HashSet<SlabIndex>>) -> Self {
        let name_pool_time = Instant::now();
        let mut map = BTreeMap::new();
        for (name, indices) in data {
            let interned = NAME_POOL.push(&name);
            map.insert(interned, indices);
        }
        info!(
            "Name pool construction time: {:?}, count: {}",
            name_pool_time.elapsed(),
            NAME_POOL.len(),
        );
        Self { map }
    }

    pub fn from_slab(slab: &ThinSlab<SlabNode>) -> Self {
        let mut name_index = NameIndex::default();
        // The slab is newly constructed, thus though slab.iter() iterates all slots, it won't waste too much.
        slab.iter().for_each(|(i, node)| {
            if let Some(nodes) = name_index.get_mut(node.name_and_parent.as_str()) {
                nodes.insert(i);
            } else {
                let mut nodes = HashSet::with_capacity(1);
                nodes.insert(i);
                name_index.map.insert(node.name_and_parent.as_str(), nodes);
            };
        });
        name_index
    }
}
