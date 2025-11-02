mod cli;

use anyhow::{Context, Result};
use cardinal_sdk::EventWatcher;
use clap::Parser;
use cli::Cli;
use crossbeam_channel::{Sender, bounded, unbounded};
use search_cache::{HandleFSEError, SearchCache, SearchResultNode};
use std::{io::Write, path::Path};
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

const CACHE_PATH: &str = "target/cache.zstd";
const IGNORE_PATH: &str = "/System/Volumes/Data"; // macOS specific ignore path

fn main() -> Result<()> {
    let builder = tracing_subscriber::fmt();
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        builder.with_env_filter(filter).init();
    } else {
        builder.with_max_level(LevelFilter::INFO).init();
    }

    let cli = Cli::parse();
    let path = cli.path;
    let mut cache = if cli.refresh {
        println!("Walking filesystem...");
        SearchCache::walk_fs_with_ignore(path, Path::new(IGNORE_PATH))
    } else {
        println!("Try reading cache...");
        SearchCache::try_read_persistent_cache(
            &path,
            Path::new(CACHE_PATH),
            Some(Path::new(IGNORE_PATH)),
            None,
        )
        .unwrap_or_else(|e| {
            println!("Failed to read cache: {e:?}. Re-walking filesystem...");
            SearchCache::walk_fs_with_ignore(path, Path::new(IGNORE_PATH))
        })
    };

    println!("Cache is: {cache:?}");

    let (finish_tx, finish_rx) = bounded::<Sender<SearchCache>>(1);
    let (search_tx, search_rx) = unbounded::<String>();
    let (search_result_tx, search_result_rx) = unbounded::<Result<Vec<SearchResultNode>>>();

    std::thread::spawn(move || {
        let (dev, mut event_watcher) = EventWatcher::spawn("/".to_string(), cache.last_event_id(), 0.1);
        println!("Processing changes of dev:{dev} during preparation.");
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
                recv(event_watcher.receiver) -> events => {
                    let events = events.expect("event_stream is closed");
                    if let Err(HandleFSEError::Rescan) = cache.handle_fs_events(events) {
                        println!("!!!!!!!!!! Rescan triggered !!!!!!!!");
                        // Here we clear event_watcher first as rescan may take a lot of time
                        #[allow(unused_assignments)]
                        {
                            event_watcher = EventWatcher::noop();
                        }
                        cache.rescan();
                        event_watcher = EventWatcher::spawn("/".to_string(), cache.last_event_id(), 0.1).1;
                    }
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
                    println!("[{i}] {:?} {:?}", path.path, path.metadata);
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
    println!("start writing cache: {cache:?}");
    cache
        .flush_to_file(Path::new(CACHE_PATH))
        .context("Failed to write cache to file")?;

    Ok(())
}
