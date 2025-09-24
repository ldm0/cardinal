use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct OptionSlabIndex(u32);

impl OptionSlabIndex {
    pub fn none() -> Self {
        Self(u32::MAX)
    }

    pub fn some(index: SlabIndex) -> Self {
        Self(index.0)
    }

    pub fn from_option(index: Option<SlabIndex>) -> Self {
        index.map_or(Self::none(), Self::some)
    }

    pub fn to_option(self) -> Option<SlabIndex> {
        if self.0 == u32::MAX {
            None
        } else {
            Some(SlabIndex(self.0))
        }
    }
}

// 0..=(u32::MAX-1), u32::MAX is reserved
//
// slab index starts from 0, therefore we can say if parent is u32::MAX, it means no parent
// small and dirty size optimization :(
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[serde(transparent)]
pub struct SlabIndex(u32);

impl SlabIndex {
    fn new(index: usize) -> Self {
        assert!(
            index < u32::MAX as usize,
            "slab index must be less than u32::MAX"
        );
        Self(index as u32)
    }

    fn get(&self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct ThinSlab<T>(slab::Slab<T>);

impl<T> Default for ThinSlab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ThinSlab<T> {
    pub fn new() -> Self {
        Self(slab::Slab::new())
    }

    pub fn insert(&mut self, value: T) -> SlabIndex {
        SlabIndex::new(self.0.insert(value))
    }

    pub fn get(&self, index: SlabIndex) -> Option<&T> {
        self.0.get(index.get())
    }

    pub fn get_mut(&mut self, index: SlabIndex) -> Option<&mut T> {
        self.0.get_mut(index.get())
    }

    pub fn try_remove(&mut self, index: SlabIndex) -> Option<T> {
        self.0.try_remove(index.get())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> ThinSlabIter<'_, T> {
        ThinSlabIter(self.0.iter())
    }
}

impl<T> std::ops::Index<SlabIndex> for ThinSlab<T> {
    type Output = T;

    fn index(&self, index: SlabIndex) -> &Self::Output {
        &self.0[index.get()]
    }
}

impl<T> std::ops::IndexMut<SlabIndex> for ThinSlab<T> {
    fn index_mut(&mut self, index: SlabIndex) -> &mut Self::Output {
        &mut self.0[index.get()]
    }
}

pub struct ThinSlabIter<'a, T>(slab::Iter<'a, T>);

impl<'a, T> Iterator for ThinSlabIter<'a, T> {
    type Item = (SlabIndex, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(index, value)| (SlabIndex::new(index), value))
    }
}
