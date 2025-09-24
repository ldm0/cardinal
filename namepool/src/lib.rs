#![feature(str_from_raw_parts)]
use rustc_hash::FxHashSet;
use std::{collections::BTreeSet, ffi::CStr};
mod cache_line;
use crate::cache_line::CacheLine;
use core::str;
use parking_lot::Mutex;

const CACHE_LINE_CAPACITY: usize = 16 * 1024 * 1024;

pub struct NamePool<const CAPACITY: usize = CACHE_LINE_CAPACITY> {
    inner: Mutex<NamePoolInner<CAPACITY>>,
}

struct NamePoolInner<const CAPACITY: usize> {
    filter: FxHashSet<&'static str>,
    lines: Vec<CacheLine<CAPACITY>>,
}

impl std::fmt::Debug for NamePool<CACHE_LINE_CAPACITY> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamePool")
            .field("len", &self.len())
            .field("lines", &self.inner.lock().filter)
            .finish()
    }
}


impl<const CAPACITY: usize> Default for NamePool<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAPACITY: usize> NamePool<CAPACITY> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(NamePoolInner {
                filter: FxHashSet::default(),
                lines: vec![CacheLine::new()],
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().filter.len()
    }

    /// This function add a name into last cache line, if the last cache line is
    /// full, a new cache line will be added.
    ///
    /// # Panic
    ///
    /// This function will panic if a new CacheLine cannot hold the given name.
    ///
    /// Returns (line_num, str_offset)
    ///
    /// One important feature of NamePool is that the returned offset is stable
    /// and won't be overwritten.
    pub fn push<'c>(&'c self, name: &str) -> &'c str {
        let mut inner = self.inner.lock();
        if let Some(existing) = inner.filter.get(name) {
            return existing;
        }
        let lines = &mut inner.lines;
        // There is at least one cache line
        let new_s = if let Some(s) = lines.last_mut().unwrap().push(name) {
            unsafe { str::from_raw_parts(s.0, s.1) }
        } else {
            let mut cache_line = CacheLine::new();
            let s = cache_line
                .push(name)
                .expect("Cache line is not large enough to hold he given name");
            lines.push(cache_line);
            unsafe { str::from_raw_parts(s.0, s.1) }
        };
        inner.filter.insert(new_s);
        new_s
    }

    pub fn search_substr<'search, 'pool: 'search>(
        &'pool self,
        substr: &'search str,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .lines
            .iter()
            .flat_map(|x| {
                x.search_substr(substr)
                    .map(|s| unsafe { str::from_raw_parts(s.0, s.1) })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn search_subslice<'search, 'pool: 'search>(
        &'pool self,
        subslice: &'search [u8],
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .lines
            .iter()
            .flat_map(|x| {
                x.search_subslice(subslice)
                    .map(|s| unsafe { str::from_raw_parts(s.0, s.1) })
            })
            .collect()
    }

    pub fn search_suffix<'search, 'pool: 'search>(
        &'pool self,
        suffix: &'search CStr,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .lines
            .iter()
            .flat_map(|x| {
                x.search_suffix(suffix)
                    .map(|s| unsafe { str::from_raw_parts(s.0, s.1) })
            })
            .collect()
    }

    // prefix should starts with a \0, e.g. b"\0hello"
    pub fn search_prefix<'search, 'pool: 'search>(
        &'pool self,
        prefix: &'search [u8],
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .lines
            .iter()
            .flat_map(|x| {
                x.search_prefix(prefix)
                    .map(|s| unsafe { str::from_raw_parts(s.0, s.1) })
            })
            .collect()
    }

    // `exact` should starts with a '\0', and ends with a '\0',
    // e.g. b"\0hello\0"
    pub fn search_exact<'search, 'pool: 'search>(
        &'pool self,
        exact: &'search [u8],
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .lines
            .iter()
            .flat_map(|x| {
                x.search_exact(exact)
                    .map(|s| unsafe { str::from_raw_parts(s.0, s.1) })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let pool = NamePool::<1024>::new();
        assert_eq!(pool.inner.lock().lines.len(), 1);
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_push_basic() {
        let pool = NamePool::<1024>::new();
        let s = pool.push("hello");
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_push_multiple() {
        let pool = NamePool::<1024>::new();
        let s1 = pool.push("foo");
        let s2 = pool.push("bar");
        let s3 = pool.push("baz");
        assert_eq!(s1, "foo");
        assert_eq!(s2, "bar");
        assert_eq!(s3, "baz");
    }

    #[test]
    fn test_push_empty_string() {
        let pool = NamePool::<1024>::new();
        let s = pool.push("");
        assert_eq!(s, "");
    }

    #[test]
    fn test_push_unicode() {
        let pool = NamePool::<1024>::new();
        let s = pool.push("こんにちは");
        assert_eq!(s, "こんにちは");
    }

    #[test]
    fn test_push_deduplication() {
        let pool = NamePool::<1024>::new();
        let s1 = pool.push("hello");
        let s2 = pool.push("hello");
        assert_eq!(s1, s2);
        assert_eq!(s1, "hello");
    }

    #[test]
    fn test_search_substr() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");
        pool.push("hello world hello");

        let result = pool.search_substr("hello");
        assert_eq!(result.len(), 3);
        assert!(result.contains("hello"));
        assert!(result.contains("hello world"));
        assert!(result.contains("hello world hello"));
    }

    #[test]
    fn test_search_subslice() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let result = pool.search_subslice(b"world");
        assert_eq!(result.len(), 2);
        assert!(result.contains("world"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_suffix() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let suffix = c"world";
        let result = pool.search_suffix(suffix);
        assert_eq!(result.len(), 2);
        assert!(result.contains("world"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_prefix() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let prefix = b"\0hello";
        let result = pool.search_prefix(prefix);
        assert_eq!(result.len(), 2);
        assert!(result.contains("hello"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_exact() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let exact = b"\0hello\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("hello"));

        let exact = b"\0world\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("world"));
    }

    #[test]
    fn test_search_nonexistent() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");

        let result = pool.search_substr("nonexistent");
        assert!(result.is_empty());

        let result = pool.search_subslice(b"nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_partial_match() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hell");

        let result = pool.search_substr("hell");
        assert_eq!(result.len(), 2);
        assert!(result.contains("hello"));
        assert!(result.contains("hell"));
    }

    #[test]
    fn test_search_exact_unicode() {
        let pool = NamePool::<1024>::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let exact = "\0こんにちは\0".as_bytes();
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("こんにちは"));

        let exact = "\0世界\0".as_bytes();
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("世界"));

        let exact = "\0こんにちは世界\0".as_bytes();
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("こんにちは世界"));
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed\n  left: 104\n right: 0")]
    fn test_search_exact_should_panic_no_leading_null_namepool() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");

        // This should panic because the exact string does not start with \0
        let exact = b"hello\0";
        let _result = pool.search_exact(exact);
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed\n  left: 111\n right: 0")]
    fn test_search_exact_should_panic_no_trailing_null_namepool() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");

        // This should panic because the exact string does not end with '\0'
        let exact = b"\0hello";
        let _result = pool.search_exact(exact);
    }

    #[test]
    fn test_search_exact_no_overlap() {
        let pool = NamePool::<1024>::new();
        pool.push("test");
        pool.push("testtest");
        pool.push("testtesttest");

        let exact = b"\0test\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test"));

        let exact = b"\0testtest\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testtest"));

        let exact = b"\0testtesttest\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testtesttest"));
    }

    #[test]
    fn test_search_exact_with_embedded_nulls() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");

        let exact = b"\0\0hello\0";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());

        let exact = b"\0hello\0\0";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_boundary_cases() {
        let pool = NamePool::<1024>::new();
        pool.push("");
        pool.push("a");
        pool.push("ab");

        let exact = b"\0\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));

        let exact = b"\0a\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("a"));

        let exact = b"\0ab\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("ab"));
    }

    #[test]
    fn test_search_exact_similar_strings() {
        let pool = NamePool::<1024>::new();
        pool.push("test");
        pool.push("testing");
        pool.push("tester");
        pool.push("test123");

        let exact = b"\0test\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test"));

        let exact = b"\0testing\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testing"));

        let exact = b"\0tester\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("tester"));

        let exact = b"\0test123\0";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test123"));
    }

    #[test]
    fn test_search_unicode() {
        let pool = NamePool::<1024>::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let result = pool.search_substr("世界");
        assert_eq!(result.len(), 2);
        assert!(result.contains("世界"));
        assert!(result.contains("こんにちは世界"));
    }

    #[test]
    fn test_search_prefix_nonexistent() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");

        let prefix = b"\0nonexistent";
        let result = pool.search_prefix(prefix);
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_nonexistent() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("world");

        let exact = b"\0nonexistent\0";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());
    }

    #[test]
    #[should_panic(expected = "assertion `left == right` failed")]
    fn test_search_prefix_should_panic_namepool() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");

        let prefix = b"hello";
        let _result = pool.search_prefix(prefix);
    }

    #[test]
    fn test_dedup_behavior_comparison() {
        let pool = NamePool::<1024>::new();
        pool.push("hello");
        pool.push("hello world");
        pool.push("hello world hello");

        let substr_result: Vec<_> = pool.search_substr("hello").into_iter().collect();
        assert_eq!(substr_result.len(), 3);

        let exact_result: Vec<_> = pool.search_exact(b"\0hello\0").into_iter().collect();
        assert_eq!(exact_result.len(), 1);
        assert_eq!(exact_result[0], "hello");

        let mut unique_results = substr_result.clone();
        unique_results.sort();
        unique_results.dedup();
        assert_eq!(substr_result.len(), unique_results.len());
    }

    #[test]
    fn test_search_exact_performance_assumption() {
        let pool = NamePool::<1024>::new();
        pool.push("abc");
        pool.push("abcabc");

        let exact = b"\0abc\0";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abc");

        let exact = b"\0abcabc\0";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abcabc");

        let exact = b"\0ab\0";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_boundary_single_char() {
        let pool = NamePool::<1024>::new();
        pool.push("a");
        let result: Vec<_> = pool.search_substr("a").into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        let result: Vec<_> = pool.search_subslice(b"a").into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        pool.push("abc");
        let result: Vec<_> = pool.search_substr("a").into_iter().collect();
        assert_eq!(result.len(), 2);

        let result: Vec<_> = pool.search_substr("b").into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abc");

        let result: Vec<_> = pool.search_substr("c").into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abc");
    }

    #[test]
    fn test_boundary_very_long_strings() {
        let pool = NamePool::<1024>::new();
        let long_string = "a".repeat(500);
        let medium_string = "b".repeat(250);

        pool.push(&long_string);
        pool.push(&medium_string);

        let result: Vec<_> = pool.search_substr("a").into_iter().collect();
        assert_eq!(result.len(), 1);

        let result: Vec<_> = pool.search_substr("b").into_iter().collect();
        assert_eq!(result.len(), 1);

        let middle_substr = "a".repeat(100);
        let result: Vec<_> = pool.search_substr(&middle_substr).into_iter().collect();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_boundary_special_characters() {
        let pool = NamePool::<1024>::new();
        pool.push("hello\nworld");
        pool.push("tab\there");
        pool.push("quote\"here");
        pool.push("backslash\\here");
        pool.push("unicode🚀test");

        let result: Vec<_> = pool.search_substr("hello\nworld").into_iter().collect();
        assert_eq!(result.len(), 1);

        let result: Vec<_> = pool.search_substr("tab\there").into_iter().collect();
        assert_eq!(result.len(), 1);

        let result: Vec<_> = pool.search_substr("quote\"here").into_iter().collect();
        assert_eq!(result.len(), 1);

        let result: Vec<_> = pool.search_substr("backslash\\here").into_iter().collect();
        assert_eq!(result.len(), 1);

        let result: Vec<_> = pool.search_substr("unicode🚀test").into_iter().collect();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_boundary_overlapping_patterns() {
        let pool = NamePool::<1024>::new();
        pool.push("aaa");
        pool.push("aaaa");
        pool.push("aaaaa");

        let result: Vec<_> = pool.search_substr("aa").into_iter().collect();
        assert_eq!(result.len(), 3);

        let mut unique_results = result.clone();
        unique_results.sort();
        unique_results.dedup();
        assert_eq!(result.len(), unique_results.len());
    }

    #[test]
    fn test_corner_many_duplicates() {
        let pool = NamePool::<1024>::new();
        // Push the same string many times
        for _ in 0..100 {
            pool.push("duplicate");
        }
        assert_eq!(pool.len(), 1); // Should only store one unique string

        let result = pool.search_exact(b"\0duplicate\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains("duplicate"));
    }

    #[test]
    fn test_corner_capacity_overflow() {
        let pool = NamePool::<1024>::new();
        // Fill with small strings first
        for i in 0..50 {
            pool.push(&format!("str{i}"));
        }

        // Try to add a very long string that might not fit
        let long_str = "x".repeat(800);
        pool.push(&long_str); // This should succeed as it goes to a new cache line

        let result: Vec<_> = pool.search_substr(&long_str).into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], long_str);
    }

    #[test]
    fn test_corner_exact_boundary_strings() {
        let pool = NamePool::<1024>::new();
        // Test strings that are exactly at various boundaries
        pool.push(""); // Empty
        pool.push("x"); // Single char
        pool.push("xy"); // Two chars
        pool.push("xyz"); // Three chars

        let result = pool.search_exact(b"\0\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));

        let result = pool.search_exact(b"\0x\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains("x"));

        let result = pool.search_exact(b"\0xy\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains("xy"));

        let result = pool.search_exact(b"\0xyz\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains("xyz"));
    }

    #[test]
    fn test_corner_search_longer_than_strings() {
        let pool = NamePool::<1024>::new();
        pool.push("hi");
        pool.push("hello");

        // Search for pattern longer than any string
        let result = pool.search_substr("helloworld");
        assert!(result.is_empty());

        let result = pool.search_subslice(b"helloworld");
        assert!(result.is_empty());
    }

    #[test]
    fn test_corner_multiple_cache_lines() {
        let pool = NamePool::<1024>::new();
        // Fill first cache line
        for i in 0..100 {
            pool.push(&format!("line1_{i}"));
        }

        // Add to second cache line
        for i in 0..100 {
            pool.push(&format!("line2_{i}"));
        }

        // Search should work across all cache lines
        let result = pool.search_substr("line1_");
        assert_eq!(result.len(), 100);

        let result = pool.search_substr("line2_");
        assert_eq!(result.len(), 100);

        // Total unique strings
        assert_eq!(pool.len(), 200);
    }

    #[test]
    fn test_corner_prefix_suffix_relationships() {
        let pool = NamePool::<1024>::new();
        pool.push("a");
        pool.push("ab");
        pool.push("abc");
        pool.push("abcd");

        // Test prefix searches
        let result = pool.search_prefix(b"\0a");
        assert_eq!(result.len(), 4); // All strings start with "a"

        let result = pool.search_prefix(b"\0ab");
        assert_eq!(result.len(), 3); // "ab", "abc", "abcd"

        let result = pool.search_prefix(b"\0abc");
        assert_eq!(result.len(), 2); // "abc", "abcd"

        let result = pool.search_prefix(b"\0abcd");
        assert_eq!(result.len(), 1); // "abcd"

        // Test suffix searches
        let result = pool.search_suffix(c"d");
        assert_eq!(result.len(), 1); // "abcd"

        let result = pool.search_suffix(c"c");
        assert_eq!(result.len(), 1); // "abc"

        let result = pool.search_suffix(c"bc");
        assert_eq!(result.len(), 1); // "abc"
    }

    #[test]
    fn test_corner_control_characters() {
        let pool = NamePool::<1024>::new();
        pool.push("line1\nline2");
        pool.push("tab\there");
        pool.push("null\x00byte");
        pool.push("bell\x07sound");

        let result = pool.search_substr("line1\nline2");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("tab\there");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("null\x00byte");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("bell\x07sound");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_corner_unicode_edge_cases() {
        let pool = NamePool::<1024>::new();
        pool.push("café");
        pool.push("naïve");
        pool.push("Москва"); // Cyrillic
        pool.push("東京"); // Japanese
        pool.push("🚀🌟"); // Emojis
        pool.push("e\u{0301}"); // Combining character

        let result = pool.search_substr("café");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("naïve");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("Москва");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("東京");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("🚀🌟");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("e\u{0301}");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_corner_search_result_deduplication() {
        let pool = NamePool::<1024>::new();
        pool.push("abab");
        pool.push("ababa");

        // "ab" appears in both strings, but should only be returned once per string
        let result: Vec<_> = pool.search_substr("ab").into_iter().collect();
        assert_eq!(result.len(), 2); // Two strings contain "ab"
        assert!(result.contains(&"abab"));
        assert!(result.contains(&"ababa"));
    }

    #[test]
    fn test_corner_exact_vs_substring() {
        let pool = NamePool::<1024>::new();
        pool.push("test");
        pool.push("testing");
        pool.push("atestb");

        // Exact search for "test"
        let exact_result = pool.search_exact(b"\0test\0");
        assert_eq!(exact_result.len(), 1);
        assert!(exact_result.contains("test"));

        // Substring search for "test"
        let substr_result = pool.search_substr("test");
        assert_eq!(substr_result.len(), 3); // "test", "testing", "atestb"
        assert!(substr_result.contains("test"));
        assert!(substr_result.contains("testing"));
        assert!(substr_result.contains("atestb"));
    }

    #[test]
    fn test_corner_zero_width_strings() {
        let pool = NamePool::<1024>::new();
        pool.push("");
        pool.push("a");
        pool.push("");

        // Should only have one empty string due to deduplication
        assert_eq!(pool.len(), 2);

        let result = pool.search_exact(b"\0\0");
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));
    }

    #[test]
    fn test_corner_large_number_of_small_strings() {
        let pool = NamePool::<1024>::new();
        // Add many small strings
        for i in 0..1000 {
            pool.push(&i.to_string());
        }

        assert_eq!(pool.len(), 1000);

        // Search for a specific number
        let result = pool.search_exact(format!("\0{}\0", 42).as_bytes());
        assert_eq!(result.len(), 1);
        assert!(result.contains("42"));

        // Search for a pattern that appears in many strings
        let result = pool.search_substr("1");
        assert_eq!(result.len(), 271); // Numbers containing "1": 1,10-19,21,31,41,51,...991
    }
}
