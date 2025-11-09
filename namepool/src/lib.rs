#![feature(str_from_raw_parts)]
use core::str;
use parking_lot::Mutex;
use regex::Regex;
use std::collections::BTreeSet;

pub struct NamePool {
    inner: Mutex<BTreeSet<Box<str>>>,
}

impl std::fmt::Debug for NamePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NamePool")
            .field("len", &self.len())
            .finish()
    }
}

impl Default for NamePool {
    fn default() -> Self {
        Self::new()
    }
}

impl NamePool {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
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
        let Some(mut inner) = self.inner.try_lock_for(std::time::Duration::from_millis(3000)) else {
            tracing::warn!("NamePool push lock timeout: name={}", name);
            panic!("NamePool push lock timeout: name={}", name);
        };
        if !inner.contains(name) {
            inner.insert(name.into());
        }
        let existing = inner.get(name).unwrap();
        unsafe { str::from_raw_parts(existing.as_ptr(), existing.len()) }
    }

    pub fn search_substr<'search, 'pool: 'search>(
        &'pool self,
        substr: &'search str,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .iter()
            .filter(|&x| x.contains(substr))
            .map(|x| unsafe { str::from_raw_parts(x.as_ptr(), x.len()) })
            .collect()
    }

    pub fn search_suffix<'search, 'pool: 'search>(
        &'pool self,
        suffix: &'search str,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .iter()
            .filter(|&x| x.ends_with(suffix))
            .map(|x| unsafe { str::from_raw_parts(x.as_ptr(), x.len()) })
            .collect()
    }

    pub fn search_prefix<'search, 'pool: 'search>(
        &'pool self,
        prefix: &'search str,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .iter()
            .filter(|&x| x.starts_with(prefix))
            .map(|x| unsafe { str::from_raw_parts(x.as_ptr(), x.len()) })
            .collect()
    }

    pub fn search_regex<'search, 'pool: 'search>(
        &'pool self,
        pattern: &Regex,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .iter()
            .filter_map(|x| {
                let existing = unsafe { str::from_raw_parts(x.as_ptr(), x.len()) };
                pattern.is_match(existing).then_some(existing)
            })
            .collect()
    }

    // `exact` should starts with a '\0', and ends with a '\0',
    // e.g. b"\0hello\0"
    pub fn search_exact<'search, 'pool: 'search>(
        &'pool self,
        exact: &'search str,
    ) -> BTreeSet<&'pool str> {
        self.inner
            .lock()
            .iter()
            .filter(|&x| (&**x == exact))
            .map(|x| unsafe { str::from_raw_parts(x.as_ptr(), x.len()) })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let pool = NamePool::new();
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_push_basic() {
        let pool = NamePool::new();
        let s = pool.push("hello");
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_push_multiple() {
        let pool = NamePool::new();
        let s1 = pool.push("foo");
        let s2 = pool.push("bar");
        let s3 = pool.push("baz");
        assert_eq!(s1, "foo");
        assert_eq!(s2, "bar");
        assert_eq!(s3, "baz");
    }

    #[test]
    fn test_push_empty_string() {
        let pool = NamePool::new();
        let s = pool.push("");
        assert_eq!(s, "");
    }

    #[test]
    fn test_push_unicode() {
        let pool = NamePool::new();
        let s = pool.push("こんにちは");
        assert_eq!(s, "こんにちは");
    }

    #[test]
    fn test_push_deduplication() {
        let pool = NamePool::new();
        let s1 = pool.push("hello");
        let s2 = pool.push("hello");
        assert_eq!(s1, s2);
        assert_eq!(s1, "hello");
    }

    #[test]
    fn test_search_substr() {
        let pool = NamePool::new();
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
    fn test_search_substr_2() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let result = pool.search_substr("world");
        assert_eq!(result.len(), 2);
        assert!(result.contains("world"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_suffix() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let suffix = "world";
        let result = pool.search_suffix(suffix);
        assert_eq!(result.len(), 2);
        assert!(result.contains("world"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_prefix() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let prefix = "hello";
        let result = pool.search_prefix(prefix);
        assert_eq!(result.len(), 2);
        assert!(result.contains("hello"));
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_search_exact() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("hello world");

        let exact = "hello";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("hello"));

        let exact = "world";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("world"));
    }

    #[test]
    fn test_search_regex_basic() {
        use regex::Regex;

        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");
        pool.push("helloworld");

        let regex = Regex::new("hell.*").unwrap();
        let result = pool.search_regex(&regex);
        assert_eq!(result.len(), 2);
        assert!(result.contains("hello"));
        assert!(result.contains("helloworld"));
    }

    #[test]
    fn test_search_regex_case_insensitive() {
        use regex::RegexBuilder;

        let pool = NamePool::new();
        pool.push("Alpha");
        pool.push("beta");

        let regex = RegexBuilder::new("alpha")
            .case_insensitive(true)
            .build()
            .unwrap();
        let result = pool.search_regex(&regex);
        assert_eq!(result.len(), 1);
        assert!(result.contains("Alpha"));
    }

    #[test]
    fn test_search_nonexistent() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let result = pool.search_substr("nonexistent");
        assert!(result.is_empty());

        let result = pool.search_substr("nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_partial_match() {
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        pool.push("こんにちは");
        pool.push("世界");
        pool.push("こんにちは世界");

        let exact = "こんにちは";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("こんにちは"));

        let exact = "世界";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("世界"));

        let exact = "こんにちは世界";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("こんにちは世界"));
    }

    #[test]
    fn test_search_exact_no_overlap() {
        let pool = NamePool::new();
        pool.push("test");
        pool.push("testtest");
        pool.push("testtesttest");

        let exact = "test";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test"));

        let exact = "testtest";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testtest"));

        let exact = "testtesttest";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testtesttest"));
    }

    #[test]
    fn test_search_exact_with_embedded_nulls() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let exact = "\0\0hello\0";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());

        let exact = "\0hello\0\0";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_boundary_cases() {
        let pool = NamePool::new();
        pool.push("");
        pool.push("a");
        pool.push("ab");

        let exact = "";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));

        let exact = "a";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("a"));

        let exact = "ab";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("ab"));
    }

    #[test]
    fn test_search_exact_similar_strings() {
        let pool = NamePool::new();
        pool.push("test");
        pool.push("testing");
        pool.push("tester");
        pool.push("test123");

        let exact = "test";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test"));

        let exact = "testing";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("testing"));

        let exact = "tester";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("tester"));

        let exact = "test123";
        let result = pool.search_exact(exact);
        assert_eq!(result.len(), 1);
        assert!(result.contains("test123"));
    }

    #[test]
    fn test_search_unicode() {
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let prefix = "nonexistent";
        let result = pool.search_prefix(prefix);
        assert!(result.is_empty());
    }

    #[test]
    fn test_search_exact_nonexistent() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("world");

        let exact = "nonexistent";
        let result = pool.search_exact(exact);
        assert!(result.is_empty());
    }

    #[test]
    fn test_dedup_behavior_comparison() {
        let pool = NamePool::new();
        pool.push("hello");
        pool.push("hello world");
        pool.push("hello world hello");

        let substr_result: Vec<_> = pool.search_substr("hello").into_iter().collect();
        assert_eq!(substr_result.len(), 3);

        let exact_result: Vec<_> = pool.search_exact("hello").into_iter().collect();
        assert_eq!(exact_result.len(), 1);
        assert_eq!(exact_result[0], "hello");

        let mut unique_results = substr_result.clone();
        unique_results.sort();
        unique_results.dedup();
        assert_eq!(substr_result.len(), unique_results.len());
    }

    #[test]
    fn test_search_exact_performance_assumption() {
        let pool = NamePool::new();
        pool.push("abc");
        pool.push("abcabc");

        let exact = "abc";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abc");

        let exact = "abcabc";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "abcabc");

        let exact = "ab";
        let result: Vec<_> = pool.search_exact(exact).into_iter().collect();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_boundary_single_char() {
        let pool = NamePool::new();
        pool.push("a");
        let result: Vec<_> = pool.search_substr("a").into_iter().collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "a");

        let result: Vec<_> = pool.search_substr("a").into_iter().collect();
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
        let pool = NamePool::new();
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
        let pool = NamePool::new();
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
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        // Push the same string many times
        for _ in 0..100 {
            pool.push("duplicate");
        }
        assert_eq!(pool.len(), 1); // Should only store one unique string

        let result = pool.search_exact("duplicate");
        assert_eq!(result.len(), 1);
        assert!(result.contains("duplicate"));
    }

    #[test]
    fn test_corner_capacity_overflow() {
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        // Test strings that are exactly at various boundaries
        pool.push(""); // Empty
        pool.push("x"); // Single char
        pool.push("xy"); // Two chars
        pool.push("xyz"); // Three chars

        let result = pool.search_exact("");
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));

        let result = pool.search_exact("x");
        assert_eq!(result.len(), 1);
        assert!(result.contains("x"));

        let result = pool.search_exact("xy");
        assert_eq!(result.len(), 1);
        assert!(result.contains("xy"));

        let result = pool.search_exact("xyz");
        assert_eq!(result.len(), 1);
        assert!(result.contains("xyz"));
    }

    #[test]
    fn test_corner_search_longer_than_strings() {
        let pool = NamePool::new();
        pool.push("hi");
        pool.push("hello");

        // Search for pattern longer than any string
        let result = pool.search_substr("helloworld");
        assert!(result.is_empty());

        let result = pool.search_substr("helloworld");
        assert!(result.is_empty());
    }

    #[test]
    fn test_corner_multiple_cache_lines() {
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        pool.push("a");
        pool.push("ab");
        pool.push("abc");
        pool.push("abcd");

        // Test prefix searches
        let result = pool.search_prefix("a");
        assert_eq!(result.len(), 4); // All strings start with "a"

        let result = pool.search_prefix("ab");
        assert_eq!(result.len(), 3); // "ab", "abc", "abcd"

        let result = pool.search_prefix("abc");
        assert_eq!(result.len(), 2); // "abc", "abcd"

        let result = pool.search_prefix("abcd");
        assert_eq!(result.len(), 1); // "abcd"

        // Test suffix searches
        let result = pool.search_suffix("d");
        assert_eq!(result.len(), 1); // "abcd"

        let result = pool.search_suffix("cd");
        assert_eq!(result.len(), 1); // "abcd"

        let result = pool.search_suffix("bcd");
        assert_eq!(result.len(), 1); // "abcd"
    }

    #[test]
    fn test_corner_control_characters() {
        let pool = NamePool::new();
        pool.push("line1\nline2");
        pool.push("tab\there");
        pool.push("null\0byte");
        pool.push("bell\x07sound");

        let result = pool.search_substr("line1\nline2");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("tab\there");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("null\0byte");
        assert_eq!(result.len(), 1);

        let result = pool.search_substr("bell\x07sound");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_corner_unicode_edge_cases() {
        let pool = NamePool::new();
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
        let pool = NamePool::new();
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
        let pool = NamePool::new();
        pool.push("test");
        pool.push("testing");
        pool.push("atestb");

        // Exact search for "test"
        let exact_result = pool.search_exact("test");
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
        let pool = NamePool::new();
        pool.push("");
        pool.push("a");
        pool.push("");

        // Should only have one empty string due to deduplication
        assert_eq!(pool.len(), 2);

        let result = pool.search_exact("");
        assert_eq!(result.len(), 1);
        assert!(result.contains(""));
    }

    #[test]
    fn test_corner_large_number_of_small_strings() {
        let pool = NamePool::new();
        // Add many small strings
        for i in 0..1000 {
            pool.push(&i.to_string());
        }

        assert_eq!(pool.len(), 1000);

        // Search for a specific number
        let result = pool.search_exact("42");
        assert_eq!(result.len(), 1);
        assert!(result.contains("42"));

        // Search for a pattern that appears in many strings
        let result = pool.search_substr("1");
        assert_eq!(result.len(), 271);
    }
}
