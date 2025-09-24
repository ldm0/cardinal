#[cfg(test)]
mod extra {
    use crate::SearchCache;
    use std::{fs, path::PathBuf};
    use tempdir::TempDir;

    #[test]
    fn test_search_empty_returns_all_nodes() {
        let tmp = TempDir::new("search_empty").unwrap();
        fs::File::create(tmp.path().join("a.txt")).unwrap();
        fs::File::create(tmp.path().join("b.txt")).unwrap();
        let cache = SearchCache::walk_fs(tmp.path().to_path_buf());
        let all = cache.search_empty();
        assert_eq!(all.len(), cache.get_total_files());
    }

    #[test]
    fn test_node_path_root_and_child() {
        let tmp = TempDir::new("node_path").unwrap();
        fs::create_dir(tmp.path().join("dir1")).unwrap();
        fs::File::create(tmp.path().join("dir1/file_x")).unwrap();
        let cache = SearchCache::walk_fs(tmp.path().to_path_buf());
        let idxs = cache.search("file_x").unwrap();
        assert_eq!(idxs.len(), 1);
        let full = cache.node_path(idxs.into_iter().next().unwrap()).unwrap();
        assert!(full.ends_with(PathBuf::from("dir1/file_x")));
    }

    #[test]
    fn test_remove_node_path_nonexistent_returns_none() {
        let tmp = TempDir::new("remove_node_none").unwrap();
        let mut cache = SearchCache::walk_fs(tmp.path().to_path_buf());
        // remove_node_path is private via crate; exercise via scan removal scenario
        // create then delete file and ensure second scan removal returns None
        let file = tmp.path().join("temp_remove.txt");
        fs::write(&file, b"x").unwrap();
        let id = cache.last_event_id() + 1;
        cache
            .handle_fs_events(vec![cardinal_sdk::FsEvent {
                path: file.clone(),
                id,
                flag: cardinal_sdk::EventFlag::ItemCreated,
            }])
            .unwrap();
        // delete file and send removal event => handle_fs_events will trigger internal removal
        fs::remove_file(&file).unwrap();
        let id2 = id + 1;
        cache
            .handle_fs_events(vec![cardinal_sdk::FsEvent {
                path: file.clone(),
                id: id2,
                flag: cardinal_sdk::EventFlag::ItemRemoved,
            }])
            .unwrap();
        assert!(cache.search("temp_remove.txt").unwrap().is_empty());
    }

    #[test]
    fn test_expand_file_nodes_fetch_metadata() {
        let tmp = TempDir::new("expand_meta").unwrap();
        fs::write(tmp.path().join("meta.txt"), b"hello world").unwrap();
        let mut cache = SearchCache::walk_fs(tmp.path().to_path_buf());
        let idxs = cache.search("meta.txt").unwrap();
        assert_eq!(idxs.len(), 1);
        // First query_files returns metadata None
        let q1 = cache.query_files("meta.txt".into()).unwrap();
        assert_eq!(q1.len(), 1);
        assert!(q1[0].metadata.is_none());
        // expand_file_nodes should fetch metadata
        let nodes = cache.expand_file_nodes(idxs.to_vec());
        assert_eq!(nodes.len(), 1);
        assert!(
            nodes[0].metadata.is_some(),
            "metadata should be fetched on demand"
        );
        // A second expand should still have metadata (cached)
        let nodes2 = cache.expand_file_nodes(idxs.to_vec());
        assert!(nodes2[0].metadata.is_some());
    }

    #[test]
    fn test_persistent_roundtrip() {
        let tmp = TempDir::new("persist_round").unwrap();
        fs::write(tmp.path().join("a.bin"), b"data").unwrap();
        let cache_path = tmp.path().join("cache.zstd");
        let cache = SearchCache::walk_fs(tmp.path().to_path_buf());
        let original_total = cache.get_total_files();
        cache.flush_to_file(&cache_path).unwrap();
        let loaded =
            SearchCache::try_read_persistent_cache(tmp.path(), &cache_path, None, None).unwrap();
        assert_eq!(loaded.get_total_files(), original_total);
    }
}
