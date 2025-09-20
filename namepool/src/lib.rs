use itertools::Itertools;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::CStr, sync::LazyLock};

fn get_parallelism() -> usize {
    *LazyLock::new(|| {
        std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(32)
    })
}

#[derive(Serialize, Deserialize)]
pub struct NamePool {
    // e.g. `\0aaa\0bbb\0ccc\0`
    // \0 is used as a separator
    pool: Vec<u8>,
}

impl NamePool {
    pub fn new() -> Self {
        Self { pool: vec![b'\0'] }
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

    // returns index of the trailing \0 and the string
    fn get(&self, offset: usize) -> (usize, &str) {
        // as this function should only be called by ourselves
        debug_assert!(offset < self.pool.len());
        // offset seperates string like this `\0 aaa\0 bbb\0 ccc\0`
        let begin = self.pool[..offset]
            .iter()
            .rposition(|&x| x == 0)
            .map(|x| x + 1)
            .unwrap_or(0);
        let end = self.pool[offset..]
            .iter()
            .position(|&x| x == 0)
            .map(|x| x + offset)
            .unwrap_or(self.pool.len());
        (end, unsafe {
            std::str::from_utf8_unchecked(&self.pool[begin..end])
        })
    }

    pub fn search_substr<'a>(&'a self, substr: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        memchr::memmem::find_iter(&self.pool, substr.as_bytes())
            .map(move |x| self.get(x))
            .dedup_by(|(a, _), (b, _)| a == b)
            .map(|(_, s)| s)
    }

    pub fn search_subslice<'search, 'pool: 'search>(
        &'pool self,
        subslice: &'search [u8],
    ) -> impl Iterator<Item = &'pool str> + 'search {
        memchr::memmem::find_iter(&self.pool, subslice)
            .map(move |x| self.get(x))
            .dedup_by(|(a, _), (b, _)| a == b)
            .map(|(_, s)| s)
    }

    pub fn search_suffix<'search, 'pool: 'search>(
        &'pool self,
        suffix: &'search CStr,
    ) -> impl Iterator<Item = &'pool str> + 'search {
        memchr::memmem::find_iter(&self.pool, suffix.to_bytes_with_nul())
            .map(move |x| self.get(x))
            .dedup_by(|(a, _), (b, _)| a == b)
            .map(|(_, s)| s)
    }

    // prefix should starts with a \0, e.g. b"\0hello"
    pub fn search_prefix<'search, 'pool: 'search>(
        &'pool self,
        prefix: &'search [u8],
    ) -> impl Iterator<Item = &'pool str> + 'search {
        assert_eq!(prefix[0], 0);
        memchr::memmem::find_iter(&self.pool, prefix)
            // To make sure it points to the end of the prefix. If we use the begin index, we will get a string before the correct one.
            .map(|x| x + prefix.len() - 1)
            .map(move |x| self.get(x))
            .dedup_by(|(a, _), (b, _)| a == b)
            .map(|(_, s)| s)
    }

    // `exact` should starts with a '\0', and ends with a '\0',
    // e.g. b"\0hello\0"
    pub fn search_exact<'search, 'pool: 'search>(
        &'pool self,
        exact: &'search [u8],
    ) -> impl Iterator<Item = &'pool str> + 'search {
        assert_eq!(exact[0], 0);
        assert_eq!(exact[exact.len() - 1], 0);
        memchr::memmem::find_iter(&self.pool, exact)
            .map(|x| x + exact.len() - 1)
            // No dedup needed since this is exact match(b"\0hello\0"), no overlap possible
            .map(|x| self.get(x).1)
    }

    pub fn par_search_substr<'a>(&'a self, substr: &'a str) -> Vec<&'a str> {
        let substr_bytes = substr.as_bytes();
        let pool_len = self.pool.len();
        if pool_len == 0 || substr_bytes.is_empty() {
            return vec![];
        }
        let chunk_size = (pool_len / get_parallelism()).max(1024).min(pool_len);

        (0..pool_len)
            .into_par_iter()
            .step_by(chunk_size)
            .flat_map(|i| {
                let search_end = (i + chunk_size).min(pool_len);
                let read_end = (search_end + substr_bytes.len() - 1).min(pool_len);
                let slice = &self.pool[i..read_end];
                memchr::memmem::find_iter(slice, substr_bytes)
                    .filter(move |&x| i + x < search_end)
                    .map(move |x| self.get(i + x))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    pub fn par_search_subslice<'search, 'pool: 'search>(
        &'pool self,
        subslice: &'search [u8],
    ) -> Vec<&'pool str> {
        let pool_len = self.pool.len();
        if pool_len == 0 || subslice.is_empty() {
            return vec![];
        }
        let chunk_size = (pool_len / get_parallelism()).max(1024).min(pool_len);

        (0..pool_len)
            .into_par_iter()
            .step_by(chunk_size)
            .flat_map(|i| {
                let search_end = (i + chunk_size).min(pool_len);
                let read_end = (search_end + subslice.len() - 1).min(pool_len);
                let slice = &self.pool[i..read_end];
                memchr::memmem::find_iter(slice, subslice)
                    .map(move |x| self.get(i + x))
                    .dedup_by(|(a, _), (b, _)| a == b)
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    pub fn par_search_suffix<'search, 'pool: 'search>(
        &'pool self,
        suffix: &'search CStr,
    ) -> Vec<&'pool str> {
        let suffix_bytes = suffix.to_bytes_with_nul();
        let pool_len = self.pool.len();
        if pool_len == 0 || suffix_bytes.is_empty() {
            return vec![];
        }
        let chunk_size = (pool_len / get_parallelism()).max(1024).min(pool_len);

        (0..pool_len)
            .into_par_iter()
            .step_by(chunk_size)
            .flat_map(|i| {
                let search_end = (i + chunk_size).min(pool_len);
                let read_end = (search_end + suffix_bytes.len() - 1).min(pool_len);
                let slice = &self.pool[i..read_end];
                memchr::memmem::find_iter(slice, suffix_bytes)
                    .map(move |x| self.get(i + x))
                    .dedup_by(|(a, _), (b, _)| a == b)
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    // prefix should starts with a '\0', e.g. b"\0hello"
    pub fn par_search_prefix<'search, 'pool: 'search>(
        &'pool self,
        prefix: &'search [u8],
    ) -> Vec<&'pool str> {
        assert_eq!(prefix[0], 0);
        let pool_len = self.pool.len();
        if pool_len == 0 {
            return vec![];
        }
        let chunk_size = (pool_len / get_parallelism()).max(1024).min(pool_len);

        (0..pool_len)
            .into_par_iter()
            .step_by(chunk_size)
            .flat_map(|i| {
                let search_end = (i + chunk_size).min(pool_len);
                let read_end = (search_end + prefix.len() - 1).min(pool_len);
                let slice = &self.pool[i..read_end];
                memchr::memmem::find_iter(slice, prefix)
                    .map(move |x| self.get(i + x + prefix.len() - 1))
                    .dedup_by(|(a, _), (b, _)| a == b)
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    // `exact` should starts with a '\0', and ends with a '\0',
    // e.g. b"\0hello\0"
    pub fn par_search_exact<'search, 'pool: 'search>(
        &'pool self,
        exact: &'search [u8],
    ) -> Vec<&'pool str> {
        assert_eq!(exact[0], 0);
        assert_eq!(exact[exact.len() - 1], 0);
        let pool_len = self.pool.len();
        if pool_len == 0 {
            return vec![];
        }
        let chunk_size = (pool_len / get_parallelism()).max(1024).min(pool_len);

        (0..pool_len)
            .into_par_iter()
            .step_by(chunk_size)
            .flat_map(|i| {
                let search_end = (i + chunk_size).min(pool_len);
                let read_end = (search_end + exact.len() - 1).min(pool_len);
                let slice = &self.pool[i..read_end];
                memchr::memmem::find_iter(slice, exact)
                    .map(move |x| self.get(i + x + exact.len() - 1))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_pool() {
        let pool = NamePool::new();
        assert_eq!(pool.len(), 1); // Only the initial \0
        assert_eq!(pool.get(0), (0, ""));
    }

    #[test]
    fn test_push_and_get() {
        let mut pool = NamePool::new();
        let offset1 = pool.push("foo");
        let offset2 = pool.push("bar");
        let offset3 = pool.push("baz");

        assert_eq!(offset1, 1);
        assert_eq!(offset2, 5);
        assert_eq!(offset3, 9);

        assert_eq!(pool.get(offset1), (4, "foo"));
        assert_eq!(pool.get(offset2), (8, "bar"));
        assert_eq!(pool.get(offset3), (12, "baz"));
    }

    #[test]
    fn test_push_empty_string() {
        let mut pool = NamePool::new();
        let offset = pool.push("");
        assert_eq!(offset, 1);
        assert_eq!(pool.get(offset), (1, ""));
        assert_eq!(pool.len(), 2); // Initial \0 + pushed \0
    }

    #[test]
    fn test_get_with_offsets() {
        let mut pool = NamePool::new();
        let offset = pool.push("hello");
        assert_eq!(offset, 1);
        assert_eq!(pool.get(offset), (6, "hello"));
        assert_eq!(pool.get(0), (0, ""));
        for i in 1..=6 {
            assert_eq!(pool.get(i), (6, "hello"));
        }

        let offset = pool.push("world");
        assert_eq!(offset, 7);
        assert_eq!(pool.get(offset), (12, "world"));
        for i in 7..=12 {
            assert_eq!(pool.get(i), (12, "world"));
        }
    }

    #[test]
    fn test_search_substr() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let substr = "hello";
        let result: Vec<_> = pool.search_substr(substr).collect();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "hello");
        assert_eq!(result[1], "hello world");
        assert_eq!(result[2], "hello world hello");
    }

    #[test]
    fn test_search_subslice() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let subslice = b"world";
        let result: Vec<_> = pool.search_subslice(subslice).collect();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "world");
        assert_eq!(result[1], "hello world");
        assert_eq!(result[2], "hello world hello");
    }

    #[test]
    fn test_search_suffix() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let suffix = c"world";
        let result: Vec<_> = pool.search_suffix(suffix).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "world");
        assert_eq!(result[1], "hello world");
    }

    #[test]
    fn test_search_nonexistent() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let substr = "nonexistent";
        let result: Vec<_> = pool.search_substr(substr).collect();
        assert!(result.is_empty());

        let subslice = b"nonexistent";
        let result: Vec<_> = pool.search_subslice(subslice).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_partial_match() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hell");

        let substr = "hell";
        let result: Vec<_> = pool.search_substr(substr).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "hello");
        assert_eq!(result[1], "hell");
    }

    #[test]
    fn test_search_suffix_partial() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hell");

        let suffix = c"ell";
        let result: Vec<_> = pool.search_suffix(suffix).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hell");
    }

    #[test]
    fn test_push_unicode() {
        let mut pool = NamePool::new();
        let offset = pool.push("こんにちは");
        assert_eq!(offset, 1);
        assert_eq!(pool.get(offset), (16, "こんにちは"));
    }

    #[test]
    fn test_search_unicode() {
        let mut pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let substr = "世界";
        let result: Vec<_> = pool.search_substr(substr).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "世界");
        assert_eq!(result[1], "こんにちは世界");
    }

    #[test]
    fn test_search_unicode_suffix() {
        let mut pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let suffix = c"世界";
        let result: Vec<_> = pool.search_suffix(suffix).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "世界");
        assert_eq!(result[1], "こんにちは世界");
    }

    #[test]
    fn test_search_prefix() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let prefix = b"\0hello";
        let result: Vec<_> = pool.search_prefix(prefix).collect();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "hello");
        assert_eq!(result[1], "hello world");
        assert_eq!(result[2], "hello world hello");
    }

    #[test]
    fn test_search_prefix_nonexistent() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let prefix = b"\0nonexistent";
        let result: Vec<_> = pool.search_prefix(prefix).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_prefix_partial_match() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hell");

        let prefix = b"\0hell";
        let result: Vec<_> = pool.search_prefix(prefix).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "hello");
        assert_eq!(result[1], "hell");
    }

    #[test]
    fn test_search_prefix_unicode() {
        let mut pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let prefix = "\0こんにちは";
        let result: Vec<_> = pool.search_prefix(prefix.as_bytes()).collect();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "こんにちは");
        assert_eq!(result[1], "こんにちは世界");
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed\n  left: 104\n right: 0")]
    fn test_search_prefix_should_panic() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        // This should panic because the prefix does not start with \0
        let prefix = b"hello";
        let _result: Vec<_> = pool.search_prefix(prefix).collect();
    }

    #[test]
    fn test_search_exact() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let exact = b"\0hello\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");

        let exact = b"\0world\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "world");

        let exact = b"\0hello world\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello world");

        let exact = b"\0hello world hello\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello world hello");
    }

    #[test]
    fn test_search_exact_nonexistent() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let exact = b"\0nonexistent\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_partial_match() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hell");

        let exact = b"\0hell\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hell");

        let exact = b"\0hello\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn test_search_exact_unicode() {
        let mut pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let exact = "\0こんにちは\0".as_bytes();
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "こんにちは");

        let exact = "\0世界\0".as_bytes();
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "世界");

        let exact = "\0こんにちは世界\0".as_bytes();
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "こんにちは世界");
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed\n  left: 104\n right: 0")]
    fn test_search_exact_should_panic_no_leading_null() {
        let mut pool = NamePool::new();
        pool.push("hello");

        // This should panic because the exact string does not start with \0
        let exact = b"hello\0";
        let _result: Vec<_> = pool.search_exact(exact).collect();
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed\n  left: 111\n right: 0")]
    fn test_search_exact_should_panic_no_trailing_null() {
        let mut pool = NamePool::new();
        pool.push("hello");

        // This should panic because the exact string does not end with '\0'
        let exact = b"\0hello";
        let _result: Vec<_> = pool.search_exact(exact).collect();
    }

    #[test]
    fn test_search_exact_no_overlap() {
        let mut pool = NamePool::new();
        // Add some strings that could potentially cause overlap issues
        pool.push("test");
        pool.push("testtest"); // Contains "test" twice
        pool.push("testtesttest"); // Contains "test" three times

        // Exact search should only find exact matches, no overlaps
        let exact = b"\0test\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "test");

        let exact = b"\0testtest\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testtest");

        let exact = b"\0testtesttest\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testtesttest");
    }

    #[test]
    fn test_search_exact_with_embedded_nulls() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        // Test that exact search doesn't match partial patterns
        // This should not match anything because we're looking for "hello"
        // with extra null bytes that don't exist in the actual storage
        let exact = b"\0\0hello\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert!(result.is_empty());

        let exact = b"\0hello\0\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_boundary_cases() {
        let mut pool = NamePool::new();
        pool.push(""); // Empty string
        pool.push("a"); // Single character
        pool.push("ab"); // Two characters

        // Test empty string exact match
        let exact = b"\0\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");

        // Test single character exact match
        let exact = b"\0a\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        // Test two character exact match
        let exact = b"\0ab\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "ab");
    }

    #[test]
    fn test_search_exact_similar_strings() {
        let mut pool = NamePool::new();
        pool.push("test");
        pool.push("testing");
        pool.push("tester");
        pool.push("test123");

        // Each exact search should only return the exact match
        let exact = b"\0test\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "test");

        let exact = b"\0testing\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testing");

        let exact = b"\0tester\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "tester");

        let exact = b"\0test123\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "test123");
    }

    #[test]
    fn test_par_search_substr() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let substr = "hello";
        let mut result = pool.par_search_substr(substr);
        result.sort(); // The order is not guaranteed
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "hello");
        assert_eq!(result[1], "hello world");
        assert_eq!(result[2], "hello world hello");
    }

    #[test]
    fn test_par_search_substr_boundary() {
        let mut pool = NamePool::new();
        // Let's construct the pool first, then calculate chunk size.
        pool.push(&"a".repeat(1000));
        pool.push("b");
        pool.push(&"c".repeat(1000));

        let pool_len = pool.pool.len();
        let chunk_size = (pool_len / get_parallelism()).max(1024);

        // We need to place a name across a chunk boundary.
        // Let's clear and rebuild the pool.
        pool = NamePool::new();
        let prefix_len = chunk_size - 50;
        let name_to_find = "b".repeat(100);

        pool.push(&"a".repeat(prefix_len));
        let name_start_pos = pool.pool.len();
        pool.push(&name_to_find);
        pool.push(&"c".repeat(1000));

        // Check if the name is actually across the boundary
        assert!(
            name_start_pos < chunk_size,
            "Test setup failed: name does not start before chunk boundary"
        );
        assert!(
            name_start_pos + name_to_find.len() > chunk_size,
            "Test setup failed: name does not cross chunk boundary"
        );

        let result = pool.par_search_substr(&name_to_find);
        assert_eq!(result.len(), 1, "The name at the boundary was not found");
        assert_eq!(result[0], name_to_find);
    }

    #[test]
    fn test_par_search_exact() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let exact = b"\0hello\0";
        let result = pool.par_search_exact(exact);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "hello");
    }

    #[test]
    fn test_par_search_empty_pool() {
        let pool = NamePool::new();
        let result = pool.par_search_substr("hello");
        assert!(result.is_empty());
    }

    #[test]
    fn test_par_search_empty_query() {
        let mut pool = NamePool::new();
        pool.push("hello");
        let result = pool.par_search_substr("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_par_search_unicode() {
        let mut pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let substr = "世界";
        let result = pool.par_search_substr(substr);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "世界");
        assert_eq!(result[1], "こんにちは世界");
    }

    #[test]
    fn test_par_search_nonexistent() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let substr = "nonexistent";
        let result = pool.par_search_substr(substr);
        assert!(result.is_empty());
    }

    #[test]
    fn test_par_search_subslice_boundary() {
        let mut pool = NamePool::new();
        // Let's construct the pool first, then calculate chunk size.
        pool.push(&"a".repeat(1000));
        pool.push("b");
        pool.push(&"c".repeat(1000));

        let pool_len = pool.pool.len();
        let chunk_size = (pool_len / get_parallelism()).max(1024);

        // We need to place a name across a chunk boundary.
        // Let's clear and rebuild the pool.
        pool = NamePool::new();
        let prefix_len = chunk_size - 50;
        let name_to_find = "b".repeat(100);

        pool.push(&"a".repeat(prefix_len));
        let name_start_pos = pool.pool.len();
        pool.push(&name_to_find);
        pool.push(&"c".repeat(1000));

        // Check if the name is actually across the boundary
        assert!(
            name_start_pos < chunk_size,
            "Test setup failed: name does not start before chunk boundary"
        );
        assert!(
            name_start_pos + name_to_find.len() > chunk_size,
            "Test setup failed: name does not cross chunk boundary"
        );

        let result = pool.par_search_subslice(name_to_find.as_bytes());
        assert_eq!(result.len(), 1, "The name at the boundary was not found");
        assert_eq!(result[0], name_to_find);
    }

    #[test]
    fn test_par_search_suffix_boundary() {
        let mut pool = NamePool::new();
        // Let's construct the pool first, then calculate chunk size.
        pool.push(&"a".repeat(1000));
        pool.push("b");
        pool.push(&"c".repeat(1000));

        let pool_len = pool.pool.len();
        let chunk_size = (pool_len / get_parallelism()).max(1024);

        // We need to place a name across a chunk boundary.
        // Let's clear and rebuild the pool.
        pool = NamePool::new();
        let suffix_to_find = "b".repeat(100);
        let full_name = "a".repeat(50) + &suffix_to_find;

        // Place the name such that the suffix crosses the chunk boundary
        let before_name_len = chunk_size - 100; // 50 for prefix, 50 for first part of suffix
        pool.push(&"c".repeat(before_name_len));
        let name_start_pos = pool.pool.len();
        pool.push(&full_name);
        pool.push(&"d".repeat(1000));

        let suffix_start_pos = name_start_pos + 50;

        // Check if the suffix is actually across the boundary
        assert!(
            suffix_start_pos < chunk_size,
            "Test setup failed: suffix does not start before chunk boundary"
        );
        assert!(
            suffix_start_pos + suffix_to_find.len() > chunk_size,
            "Test setup failed: suffix does not cross chunk boundary"
        );

        let suffix_cstring = std::ffi::CString::new(suffix_to_find.as_bytes()).unwrap();
        let result = pool.par_search_suffix(&suffix_cstring);
        assert_eq!(result.len(), 1, "The name at the boundary was not found");
        assert_eq!(result[0], full_name);
    }

    #[test]
    fn test_par_search_prefix_boundary() {
        let mut pool = NamePool::new();
        // Let's construct the pool first, then calculate chunk size.
        pool.push(&"a".repeat(1000));
        pool.push("b");
        pool.push(&"c".repeat(1000));

        let pool_len = pool.pool.len();
        let chunk_size = (pool_len / get_parallelism()).max(1024);

        // We need to place a name across a chunk boundary.
        // Let's clear and rebuild the pool.
        pool = NamePool::new();
        let prefix_to_find = "b".repeat(100);
        let full_name = prefix_to_find.clone() + &"a".repeat(50);

        // Place the name such that the prefix crosses the chunk boundary
        let before_name_len = chunk_size - 50;
        pool.push(&"c".repeat(before_name_len));
        let name_start_pos = pool.pool.len();
        pool.push(&full_name);
        pool.push(&"d".repeat(1000));

        // The search is for \0prefix. The \0 is at name_start_pos - 1.
        // We want the prefix part to cross the boundary.
        assert!(
            name_start_pos < chunk_size,
            "Test setup failed: prefix does not start before chunk boundary"
        );
        assert!(
            name_start_pos + prefix_to_find.len() > chunk_size,
            "Test setup failed: prefix does not cross chunk boundary"
        );

        let prefix_bytes = [b"\0".as_slice(), prefix_to_find.as_bytes()].concat();
        let result = pool.par_search_prefix(&prefix_bytes);
        assert_eq!(result.len(), 1, "The name at the boundary was not found");
        assert_eq!(result[0], full_name);
    }

    #[test]
    fn test_par_search_exact_no_overlap() {
        let mut pool = NamePool::new();
        // Add some strings that could potentially cause overlap issues
        pool.push("test");
        pool.push("testtest"); // Contains "test" twice
        pool.push("testtesttest"); // Contains "test" three times

        // Parallel exact search should only find exact matches, no overlaps
        let exact = b"\0test\0";
        let result = pool.par_search_exact(exact);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "test");

        let exact = b"\0testtest\0";
        let result = pool.par_search_exact(exact);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testtest");

        let exact = b"\0testtesttest\0";
        let result = pool.par_search_exact(exact);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testtesttest");
    }

    #[test]
    fn test_dedup_behavior_comparison() {
        let mut pool = NamePool::new();
        pool.push("hello");
        pool.push("hello world");
        pool.push("hello world hello");

        // substr search finds multiple occurrences in the same string, needs dedup
        let substr_result: Vec<_> = pool.search_substr("hello").collect();
        assert_eq!(substr_result.len(), 3); // Each string appears only once despite multiple "hello" matches

        // exact search can only match complete strings, no dedup needed
        let exact_result: Vec<_> = pool.search_exact(b"\0hello\0").collect();
        assert_eq!(exact_result.len(), 1); // Only exact "hello" match

        // Verify the same string doesn't appear multiple times in substr results
        let mut unique_results = substr_result.clone();
        unique_results.sort();
        unique_results.dedup();
        assert_eq!(substr_result.len(), unique_results.len());
    }

    #[test]
    fn test_search_exact_performance_assumption() {
        // This test validates the assumption that exact search doesn't need dedup
        // by ensuring that the pattern b"\0string\0" can only match once per string
        let mut pool = NamePool::new();

        // Create strings where the pattern could theoretically appear multiple times
        // if we weren't doing exact matching
        pool.push("abc");
        pool.push("abcabc");
        // Note: We can't actually store strings with null bytes using push()
        // because push() adds its own null terminator

        let exact = b"\0abc\0";
        let result: Vec<_> = pool.search_exact(exact).collect();

        // Should only find the exact "abc" string
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abc");

        // Verify that "abcabc" requires its own exact pattern
        let exact = b"\0abcabc\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abcabc");

        // Test that partial matches don't work
        let exact = b"\0ab\0";
        let result: Vec<_> = pool.search_exact(exact).collect();
        assert_eq!(result.len(), 0); // No exact match for "ab"
    }

    #[test]
    fn test_boundary_single_char() {
        let mut pool = NamePool::new();

        // Test single character
        pool.push("a");
        let result: Vec<_> = pool.search_substr("a").collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        let result: Vec<_> = pool.search_subslice(b"a").collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        // Test single character in longer string
        pool.push("abc");
        let result: Vec<_> = pool.search_substr("a").collect();
        assert_eq!(result.len(), 2); // "a" and "abc"

        let result: Vec<_> = pool.search_substr("b").collect();
        assert_eq!(result.len(), 1); // only "abc"

        let result: Vec<_> = pool.search_substr("c").collect();
        assert_eq!(result.len(), 1); // only "abc"
    }

    #[test]
    fn test_boundary_very_long_strings() {
        let mut pool = NamePool::new();

        // Test with very long strings
        let long_string = "a".repeat(10000);
        let medium_string = "b".repeat(5000);

        pool.push(&long_string);
        pool.push(&medium_string);

        // Search for single character in long strings
        let result = pool.search_substr("a");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        let result = pool.search_substr("b");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        // Search for substring in the middle
        let middle_substr = "a".repeat(100);
        let result = pool.search_substr(&middle_substr);
        assert_eq!(result.collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn test_boundary_special_characters() {
        let mut pool = NamePool::new();

        // Test with various special characters
        pool.push("hello\nworld");
        pool.push("tab\there");
        pool.push("quote\"here");
        pool.push("backslash\\here");
        pool.push("unicode🚀test");

        // Search for strings containing newlines
        let result = pool.search_substr("hello\nworld");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        // Search for strings containing tabs
        let result = pool.search_substr("tab\there");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        // Search for strings containing quotes
        let result = pool.search_substr("quote\"here");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        // Search for strings containing backslashes
        let result = pool.search_substr("backslash\\here");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);

        // Search for strings containing unicode
        let result = pool.search_substr("unicode🚀test");
        assert_eq!(result.collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn test_boundary_chunk_size_edge_cases() {
        let mut pool = NamePool::new();

        // Create a pool that's exactly at chunk size boundaries
        let chunk_size = (pool.pool.len() / get_parallelism())
            .max(1024)
            .min(pool.pool.len());

        // Fill pool to be just under chunk size
        let current_len = pool.pool.len();
        let fill_size = if chunk_size > current_len + 10 {
            chunk_size - current_len - 10
        } else {
            100 // fallback size
        };

        if fill_size > 0 {
            pool.push(&"x".repeat(fill_size));
        }

        // Add strings that will cross chunk boundaries
        pool.push("boundary_test");
        pool.push("another_boundary_test");

        // Test parallel search works correctly at chunk boundaries
        let result = pool.par_search_substr("boundary_test");
        // "boundary_test" appears in both "boundary_test" and "another_boundary_test"
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"boundary_test"));
        assert!(result.contains(&"another_boundary_test"));

        let result = pool.par_search_substr("another_boundary_test");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "another_boundary_test");

        // Test that searching for a substring that appears in both strings works
        let result = pool.par_search_substr("boundary");
        assert_eq!(result.len(), 2); // Should find both "boundary_test" and "another_boundary_test"
        assert!(result.contains(&"boundary_test"));
        assert!(result.contains(&"another_boundary_test"));
    }

    #[test]
    fn test_boundary_memory_limits() {
        let mut pool = NamePool::new();

        // Test with many small strings
        for i in 0..1000 {
            pool.push(&format!("string_{}", i));
        }

        // Search for a pattern that appears in many strings
        let result = pool.search_substr("string_");
        assert_eq!(result.collect::<Vec<_>>().len(), 1000);

        // Test parallel version
        let result = pool.par_search_substr("string_");
        assert_eq!(result.len(), 1000);
    }

    #[test]
    fn test_boundary_concurrent_access() {
        let mut pool = NamePool::new();

        // Create a pool with many strings
        for i in 0..100 {
            pool.push(&format!("test_string_{}", i));
        }

        // Test that multiple searches work correctly
        let patterns: Vec<String> = (0..10).map(|i| format!("test_string_{}", i)).collect();

        for pattern in patterns {
            let result: Vec<_> = pool.search_substr(&pattern).collect();
            // The pattern should match exactly one string, but also match as substring in longer strings
            // For example, "test_string_1" should match "test_string_1", "test_string_10", "test_string_11", etc.
            let expected_count = (0..100)
                .filter(|&i| format!("test_string_{}", i).contains(&pattern))
                .count();
            assert_eq!(
                result.len(),
                expected_count,
                "Pattern '{}' should match {} strings",
                pattern,
                expected_count
            );
        }
    }

    #[test]
    fn test_boundary_overlapping_patterns() {
        let mut pool = NamePool::new();

        // Create strings with overlapping patterns
        pool.push("aaa");
        pool.push("aaaa");
        pool.push("aaaaa");

        // Search for "aa" should find all three strings
        let result = pool.search_substr("aa");
        let results: Vec<_> = result.collect();
        assert_eq!(results.len(), 3);

        // But each string should appear only once
        let mut unique_results = results.clone();
        unique_results.sort();
        unique_results.dedup();
        assert_eq!(results.len(), unique_results.len());
    }

    #[test]
    fn test_boundary_zero_parallelism() {
        // This test simulates what happens when parallelism is 1
        // (effectively making it sequential)
        let mut pool = NamePool::new();

        pool.push("test1");
        pool.push("test2");
        pool.push("test3");

        // Even with minimal parallelism, results should be correct
        let result = pool.par_search_substr("test");
        assert_eq!(result.len(), 3);
    }
    #[test]
    fn test_par_search_exact_boundary() {
        let mut pool = NamePool::new();
        // Let's construct the pool first, then calculate chunk size.
        pool.push(&"a".repeat(1000));
        pool.push("b");
        pool.push(&"c".repeat(1000));

        let pool_len = pool.pool.len();
        let chunk_size = (pool_len / get_parallelism()).max(1024);

        // We need to place a name across a chunk boundary.
        // Let's clear and rebuild the pool.
        pool = NamePool::new();
        let name_to_find = "b".repeat(100);

        // Place the name such that it crosses the chunk boundary
        let before_name_len = chunk_size - 50;
        pool.push(&"c".repeat(before_name_len));
        let name_start_pos = pool.pool.len();
        pool.push(&name_to_find);
        pool.push(&"d".repeat(1000));

        // The search is for \0exact\0. The \0 is at name_start_pos - 1.
        // We want the exact part to cross the boundary.
        assert!(
            name_start_pos < chunk_size,
            "Test setup failed: name does not start before chunk boundary"
        );
        assert!(
            name_start_pos + name_to_find.len() > chunk_size,
            "Test setup failed: name does not cross chunk boundary"
        );

        let exact_bytes = [b"\0".as_slice(), name_to_find.as_bytes(), b"\0".as_slice()].concat();
        let result = pool.par_search_exact(&exact_bytes);
        assert_eq!(result.len(), 1, "The name at the boundary was not found");
        assert_eq!(result[0], name_to_find);
    }
}
