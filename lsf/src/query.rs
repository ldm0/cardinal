// `elloworl` => Substr("elloworl")
// `/root` => Prefix("root")
// `root/` => Suffix("root")
// `/root/` => Exact("root")
// `/root/bar` => Exact("root"), Prefix("bar")
// `/root/bar/kksk` => Exact("root"), Exact("bar"), Prefix("kksk")
// `foo/bar/kks` => Suffix("foo"), Exact("bar"), Prefix("kks")
// `gaea/lil/bee/` => Suffix("gaea"), Exact("lil"), Exact("bee")
// `bab/bob/` => Suffix("bab"), Exact("bob")
// `/byb/huh/good/` => Exact("byb"), Exact("huh"), Exact("good")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'s> {
    Substr(&'s str),
    Prefix(&'s str),
    Suffix(&'s str),
    Exact(&'s str),
}

pub fn query_segmentation(query: &str) -> Vec<Segment<'_>> {
    #[derive(Clone, Copy)]
    enum State {
        Substr,
        Prefix,
        Suffix,
        Exact,
    }
    let left_close = query.starts_with('/');
    let right_close = query.ends_with('/');
    let query = query.trim_start_matches('/').trim_end_matches('/');
    // Filter out ["", "/", "///", ..]
    if query.is_empty() {
        return vec![];
    }
    let segments: Vec<_> = query.split('/').collect();
    // After trimming leading and trailing slashes, if segments contains empty string,
    // it means there are multiple consecutive slashes inserted in the original query.
    // In this case, we should return an empty vector.
    // e.g. "/a//b/" => ["a", "", "b"]
    if segments.contains(&"") {
        return vec![];
    }
    let len = segments.len();
    let states = {
        let mut states: Vec<_> = vec![State::Exact; len];
        assert_ne!(len, 0);
        if len == 1 {
            if !left_close || !right_close {
                if !left_close && !right_close {
                    states[0] = State::Substr;
                } else if !left_close {
                    states[0] = State::Suffix;
                } else if !right_close {
                    states[0] = State::Prefix;
                }
            }
        } else {
            if !left_close {
                states[0] = State::Suffix;
            }
            if !right_close {
                states[len - 1] = State::Prefix;
            }
        }
        states
    };
    states
        .into_iter()
        .zip(segments.into_iter())
        .map(|(state, segment)| match state {
            State::Substr => Segment::Substr(segment),
            State::Prefix => Segment::Prefix(segment),
            State::Suffix => Segment::Suffix(segment),
            State::Exact => Segment::Exact(segment),
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_segmentation() {
        assert_eq!(
            query_segmentation("elloworl"),
            vec![Segment::Substr("elloworl")]
        );
        assert_eq!(query_segmentation("/root"), vec![Segment::Prefix("root")]);
        assert_eq!(query_segmentation("root/"), vec![Segment::Suffix("root")]);
        assert_eq!(query_segmentation("/root/"), vec![Segment::Exact("root")]);
        assert_eq!(
            query_segmentation("/root/bar"),
            vec![Segment::Exact("root"), Segment::Prefix("bar")]
        );
        assert_eq!(
            query_segmentation("/root/bar/kksk"),
            vec![
                Segment::Exact("root"),
                Segment::Exact("bar"),
                Segment::Prefix("kksk")
            ]
        );
        assert_eq!(
            query_segmentation("foo/bar/kks"),
            vec![
                Segment::Suffix("foo"),
                Segment::Exact("bar"),
                Segment::Prefix("kks")
            ]
        );
        assert_eq!(
            query_segmentation("gaea/lil/bee/"),
            vec![
                Segment::Suffix("gaea"),
                Segment::Exact("lil"),
                Segment::Exact("bee")
            ]
        );
        assert_eq!(
            query_segmentation("bab/bob/"),
            vec![Segment::Suffix("bab"), Segment::Exact("bob")]
        );
        assert_eq!(
            query_segmentation("/byb/huh/good/"),
            vec![
                Segment::Exact("byb"),
                Segment::Exact("huh"),
                Segment::Exact("good")
            ]
        );
    }

    #[test]
    fn test_query_segmentation_edge_cases() {
        // Empty string
        assert_eq!(query_segmentation(""), vec![]);

        // Single slash
        assert_eq!(query_segmentation("/"), vec![]);

        // Multiple slashes
        assert_eq!(query_segmentation("///"), vec![]);

        // Leading and trailing slashes
        assert_eq!(query_segmentation("/a/"), vec![Segment::Exact("a")]);

        // Single character
        assert_eq!(query_segmentation("a"), vec![Segment::Substr("a")]);

        // Single character with slash
        assert_eq!(query_segmentation("/a"), vec![Segment::Prefix("a")]);
        assert_eq!(query_segmentation("a/"), vec![Segment::Suffix("a")]);

        // Mixed slashes and empty segments
        assert_eq!(query_segmentation("/a//b/"), vec![]);

        // Long string without slashes
        assert_eq!(
            query_segmentation("thisisaverylongstringwithoutslashes"),
            vec![Segment::Substr("thisisaverylongstringwithoutslashes")]
        );

        // Long string with slashes
        assert_eq!(
            query_segmentation("/this/is/a/very/long/string/"),
            vec![
                Segment::Exact("this"),
                Segment::Exact("is"),
                Segment::Exact("a"),
                Segment::Exact("very"),
                Segment::Exact("long"),
                Segment::Exact("string")
            ]
        );
    }
}
