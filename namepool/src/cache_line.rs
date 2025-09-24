use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::ffi::CStr;

#[derive(Serialize, Deserialize)]
pub struct CacheLine<const CAPACITY: usize> {
    // len: 9
    // data: b"\0aaa\0bbb\0"
    len: usize,
    data: Box<[u8]>,
}

impl<const CAPACITY: usize> CacheLine<CAPACITY> {
    pub fn new() -> Self {
        Self {
            len: 1, // reserve a leading \0 guard
            data: vec![0; CAPACITY].into_boxed_slice(),
        }
    }

    pub fn push(&mut self, name: &str) -> Option<(*const u8, usize)> {
        let len = self.len;
        let name_len = name.len();
        // reserve an ending \0 guard
        if len + name_len + 1 > CAPACITY {
            return None;
        }
        self.data[len..len + name_len].copy_from_slice(name.as_bytes());
        self.len = len + name_len + 1;
        Some((self.data.as_ptr().wrapping_add(len), name_len))
    }

    pub fn get(&self, offset: usize) -> (usize, (*const u8, usize)) {
        // as this function should only be called by ourselves
        debug_assert!(offset < CAPACITY);
        // offset seperates string like this `\0 aaa\0 bbb\0 ccc\0`
        let begin = self.data[..offset]
            .iter()
            .rposition(|&x| x == 0)
            .map(|x| x + 1)
            .unwrap_or(0);
        let end = self.data[offset..]
            .iter()
            .position(|&x| x == 0)
            .map(|x| x + offset)
            .unwrap_or(self.data.len());
        (end, (self.data.as_ptr().wrapping_add(begin), end - begin))
    }

    pub fn search_substr<'search, 'pool: 'search>(
        &'pool self,
        substr: &'search str,
    ) -> impl Iterator<Item = (*const u8, usize)> + 'search {
        memchr::memmem::find_iter(&self.data, substr.as_bytes())
            .map(|x| self.get(x))
            .dedup_by(|(x, _), (y, _)| x == y)
            .map(|(_, s)| s)
    }

    pub fn search_subslice<'search, 'pool: 'search>(
        &'pool self,
        subslice: &'search [u8],
    ) -> impl Iterator<Item = (*const u8, usize)> + 'search {
        memchr::memmem::find_iter(&self.data, subslice)
            .map(|x| self.get(x))
            .dedup_by(|(x, _), (y, _)| x == y)
            .map(|(_, s)| s)
    }

    pub fn search_suffix<'search, 'pool: 'search>(
        &'pool self,
        suffix: &'search CStr,
    ) -> impl Iterator<Item = (*const u8, usize)> + 'search {
        memchr::memmem::find_iter(&self.data, suffix.to_bytes_with_nul())
            .map(|x| self.get(x))
            .dedup_by(|(x, _), (y, _)| x == y)
            .map(|(_, s)| s)
    }

    // prefix should starts with a \0, e.g. b"\0hello"
    pub fn search_prefix<'search, 'pool: 'search>(
        &'pool self,
        prefix: &'search [u8],
    ) -> impl Iterator<Item = (*const u8, usize)> + 'search {
        assert_eq!(prefix[0], 0);
        memchr::memmem::find_iter(&self.data, prefix)
            // To make sure it points to the end of the prefix. If we use the begin index, we will get a string before the correct one.
            .map(|x| x + prefix.len() - 1)
            .map(|x| self.get(x))
            .dedup_by(|(x, _), (y, _)| x == y)
            .map(|(_, s)| s)
    }

    // `exact` should starts with a '\0', and ends with a '\0',
    // e.g. b"\0hello\0"
    pub fn search_exact<'search, 'pool: 'search>(
        &'pool self,
        exact: &'search [u8],
    ) -> impl Iterator<Item = (*const u8, usize)> + 'search {
        assert_eq!(exact[0], 0);
        assert_eq!(exact[exact.len() - 1], 0);
        memchr::memmem::find_iter(&self.data, exact)
            .map(|x| x + exact.len() - 1)
            .map(|x| self.get(x))
            .dedup_by(|(x, _), (y, _)| x == y)
            .map(|(_, s)| s)
    }
}

#[cfg(test)]
mod cacheline_tests {
    use super::*;

    #[test]
    fn test_new_cacheline() {
        const CAPACITY: usize = 1024;
        let cl = CacheLine::<CAPACITY>::new();
        assert_eq!(cl.len, 1);
        assert_eq!(cl.data.len(), CAPACITY);
        assert_eq!(cl.data[0], 0); // leading guard
    }

    #[test]
    fn test_push_and_get_basic() {
        let mut cl = CacheLine::<1024>::new();
        let old_len = cl.len;
        let result = cl.push("hello");
        assert!(result.is_some());
        let (ptr, len) = result.unwrap();
        assert_eq!(len, 5);
        assert_eq!(cl.len, old_len + 5 + 1);
        let (_, (ptr2, len2)) = cl.get(old_len);
        assert_eq!(len, len2);
        unsafe {
            assert_eq!(std::slice::from_raw_parts(ptr, len), b"hello");
            assert_eq!(std::slice::from_raw_parts(ptr2, len2), b"hello");
        }
    }

    #[test]
    fn test_push_multiple_strings() {
        let mut cl = CacheLine::<1024>::new();
        let old_len1 = cl.len;
        let (ptr1, len1) = cl.push("first").unwrap();
        let old_len2 = cl.len;
        let (ptr2, len2) = cl.push("second").unwrap();

        let (_, (ptr1_g, len1_g)) = cl.get(old_len1);
        let (_, (ptr2_g, len2_g)) = cl.get(old_len2);

        unsafe {
            assert_eq!(std::slice::from_raw_parts(ptr1, len1), b"first");
            assert_eq!(std::slice::from_raw_parts(ptr1_g, len1_g), b"first");
            assert_eq!(std::slice::from_raw_parts(ptr2, len2), b"second");
            assert_eq!(std::slice::from_raw_parts(ptr2_g, len2_g), b"second");
        }
    }

    #[test]
    fn test_push_empty_string() {
        let mut cl = CacheLine::<1024>::new();
        let old_len = cl.len;
        let (_, len) = cl.push("").unwrap();
        assert_eq!(len, 0);
        assert_eq!(cl.len, old_len + 1);
        let (_, (_, len_g)) = cl.get(old_len);
        assert_eq!(len_g, 0);
    }

    #[test]
    fn test_push_unicode_string() {
        let mut cl = CacheLine::<1024>::new();
        let old_len = cl.len;
        let (ptr, len) = cl.push("こんにちは").unwrap();
        let (_, (ptr_g, len_g)) = cl.get(old_len);
        unsafe {
            assert_eq!(
                std::slice::from_raw_parts(ptr, len),
                "こんにちは".as_bytes()
            );
            assert_eq!(
                std::slice::from_raw_parts(ptr_g, len_g),
                "こんにちは".as_bytes()
            );
        }
    }

    #[test]
    fn test_capacity_limit() {
        let mut cl = CacheLine::<10>::new(); // Small capacity
        let _ = cl.push("abc").unwrap(); // 1 + 3 + 1 = 5
        assert_eq!(cl.len, 5);
        let _ = cl.push("def").unwrap(); // 5 + 3 + 1 = 9
        assert_eq!(cl.len, 9);
        assert!(cl.push("g").is_none()); // 9 + 1 + 1 = 11 > 10
    }

    #[test]
    fn test_fill_to_capacity() {
        let mut cl = CacheLine::<20>::new();
        let mut count = 0;
        loop {
            let name = format!("x{count}");
            if cl.push(&name).is_none() {
                break;
            }
            count += 1;
        }
        assert!(count > 0);
        // Ensure no more can be added
        assert!(cl.push("extra").is_none());
    }

    #[test]
    fn test_search_substr() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("hello");
        cl.push("world");
        cl.push("hello world");

        let results: Vec<_> = cl.search_substr("hello").collect();
        assert_eq!(results.len(), 2); // "hello" and "hello world"
        // Note: results are (*const u8, usize), we can check lengths
        assert!(results.iter().any(|&(_, len)| len == 5)); // "hello"
        assert!(results.iter().any(|&(_, len)| len == 11)); // "hello world"
    }

    #[test]
    fn test_search_subslice() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("test");
        cl.push("testing");

        let results: Vec<_> = cl.search_subslice(b"test").collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_suffix() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("file.txt");
        cl.push("data.txt");

        let suffix = c".txt";
        let results: Vec<_> = cl.search_suffix(suffix).collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_prefix() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("hello");
        cl.push("help");

        let results: Vec<_> = cl.search_prefix(b"\0hel").collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_exact() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("exact");
        cl.push("exact match");

        let results: Vec<_> = cl.search_exact(b"\0exact\0").collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, 5); // length of "exact"
    }

    #[test]
    fn test_search_no_matches() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("hello");

        let results: Vec<_> = cl.search_substr("world").collect();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_boundary_cases() {
        let mut cl = CacheLine::<1024>::new();
        cl.push("a");
        cl.push("b");

        // Test get with offset at the start of a string
        // This is internal, but we can test indirectly
        // Actually, get is called internally by search methods
    }

    #[test]
    #[should_panic]
    fn test_search_prefix_invalid() {
        let cl = CacheLine::<1024>::new();
        let _ = cl.search_prefix(b"no_null").collect::<Vec<_>>();
    }

    #[test]
    #[should_panic]
    fn test_search_exact_invalid() {
        let cl = CacheLine::<1024>::new();
        let _ = cl.search_exact(b"no_nulls").collect::<Vec<_>>();
    }
}
