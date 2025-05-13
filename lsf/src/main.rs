mod cache;
mod cli;
mod persistent;
mod query;

use anyhow::{Context, Result};
use cache::SearchCache;
use cardinal_sdk::{EventStream, FSEventStreamEventId, FsEvent};
use clap::Parser;
use cli::Cli;
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use std::io::Write;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.path;
    let mut cache = if cli.refresh {
        SearchCache::walk_fs(path)
    } else {
        println!("Try reading cache...");
        SearchCache::try_read_persistent_cache(&path).unwrap_or_else(|e| {
            println!("Failed to read cache: {e:?}. Re-walking filesystem...");
            SearchCache::walk_fs(path)
        })
    };

    let (finish_tx, finish_rx) = bounded::<Sender<SearchCache>>(1);
    let (search_tx, search_rx) = unbounded::<String>();
    let (search_result_tx, search_result_rx) = unbounded::<Result<Vec<String>>>();

    std::thread::spawn(move || {
        let event_stream = spawn_event_watcher("/".to_string(), cache.last_event_id());
        println!("Processing changes during processing");
        loop {
            crossbeam_channel::select! {
                recv(finish_rx) -> tx => {
                    let tx = tx.expect("finish_tx is closed");
                    tx.send(cache).expect("finish_tx is closed");
                    break;
                }
                recv(search_rx) -> query => {
                    let query = query.expect("search_tx is closed");
                    let files = cache.query_files(query);
                    search_result_tx
                        .send(files)
                        .expect("search_result_tx is closed");
                }
                recv(event_stream) -> events => {
                    let events = events.expect("event_stream is closed");
                    cache.handle_fs_events(events);
                }
            }
        }
        println!("fsevent processing is done");
    });

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("> ");
        stdout.flush().unwrap();
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();
        let line = line.trim();
        if line.is_empty() {
            continue;
        } else if line == "/bye" {
            break;
        }

        search_tx
            .send(line.to_string())
            .context("search_tx is closed")?;
        let search_result = search_result_rx
            .recv()
            .context("search_result_rx is closed")?;
        match search_result {
            Ok(path_set) => {
                for (i, path) in path_set.into_iter().enumerate() {
                    println!("[{i}] {path}");
                }
            }
            Err(e) => {
                eprintln!("Failed to search: {e:?}");
            }
        }
    }

    let (cache_tx, cache_rx) = bounded::<SearchCache>(1);
    finish_tx.send(cache_tx).context("cache_tx is closed")?;
    let cache = cache_rx.recv().context("cache_tx is closed")?;
    cache
        .flush_to_file()
        .context("Failed to write cache to file")?;

    Ok(())
}

fn spawn_event_watcher(
    path: String,
    since_event_id: FSEventStreamEventId,
) -> Receiver<Vec<FsEvent>> {
    let (sender, receiver) = unbounded();
    std::thread::spawn(move || {
        EventStream::new(
            &[&path],
            since_event_id,
            0.1,
            Box::new(move |events| {
                let _ = sender.send(events);
            }),
        )
        .block_on()
        .unwrap();
    });
    receiver
}

// TODO(ldm0):
// - segment search cache(same search routine will be triggered while user is typing, should cache exact[..], suffix, suffix/exact[..])
// [] tui?
// - lazy metadata design
// 或许最后可以在首次扫描过程中就把中间结果 在索引逻辑和搜索逻辑之间抛来抛去，做到边索引边搜索
