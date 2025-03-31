use bincode::Encode;
use rayon::{iter::ParallelBridge, prelude::ParallelIterator};
use serde::Serialize;
use std::{
    fs,
    io::Error,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Serialize, Encode, Debug)]
pub struct Node {
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Arc<Node>>,
}

#[derive(Debug)]
pub enum Operator {
    Equal = 0,
    LessThan = 1,
    GreaterThan = 2,
}

#[derive(Default, Debug)]
pub struct WalkData {
    pub num_files: AtomicUsize,
    pub num_dirs: AtomicUsize,
}

pub fn walk_it(dir: PathBuf, walk_data: &WalkData) -> Option<Node> {
    walk(dir, walk_data, 0)
}

fn walk(dir: PathBuf, walk_data: &WalkData, depth: usize) -> Option<Node> {
    let children = if dir.is_dir() {
        walk_data.num_dirs.fetch_add(1, Ordering::Relaxed);
        let read_dir = fs::read_dir(&dir);
        match read_dir {
            Ok(entries) => entries
                .into_iter()
                .par_bridge()
                .filter_map(|entry| {
                    match &entry {
                        Ok(entry) => {
                            if let Ok(data) = entry.file_type() {
                                if data.is_dir() {
                                    return walk(entry.path(), walk_data, depth + 1);
                                } else {
                                    walk_data.num_files.fetch_add(1, Ordering::Relaxed);
                                    return Some(Node {
                                        name: entry
                                            .path()
                                            .file_name()
                                            .map(|x| x.to_string_lossy().into_owned())
                                            .unwrap_or_default(),
                                        children: vec![],
                                    });
                                }
                            }
                        }
                        Err(failed) => {
                            if handle_error_and_retry(failed) {
                                return walk(dir.clone(), walk_data, depth);
                            }
                        }
                    }
                    None
                })
                .map(Arc::new)
                .collect(),
            Err(failed) => {
                if handle_error_and_retry(&failed) {
                    return walk(dir, walk_data, depth);
                } else {
                    vec![]
                }
            }
        }
    } else {
        walk_data.num_files.fetch_add(1, Ordering::Relaxed);
        vec![]
    };
    Some(Node {
        name: dir
            .file_name()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default(),
        children,
    })
}

fn handle_error_and_retry(failed: &Error) -> bool {
    failed.kind() == std::io::ErrorKind::Interrupted
}
