use fswalk::{WalkData, walk_it};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[tokio::main]
async fn main() {
    let done = AtomicBool::new(false);
    let walk_data = WalkData::new(Some(Path::new("/System/Volumes/Data")), true, None);
    std::thread::scope(|s| {
        s.spawn(|| {
            let node = walk_it(
                Path::new("/"),
                &walk_data,
            )
            .unwrap();
            println!("root has {} children", node.children.len());
            done.store(true, Ordering::Relaxed);
        });
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                let files = walk_data.num_files.load(Ordering::Relaxed);
                let dirs = walk_data.num_dirs.load(Ordering::Relaxed);
                println!("so far: {files} files, {dirs} dirs");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    });
}
