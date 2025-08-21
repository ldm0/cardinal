// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use anyhow::{Context, Result};
use cardinal_sdk::{EventFlag, EventWatcher};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use search_cache::{HandleFSEError, SearchCache, SearchNode, SlabNodeMetadata, WalkData};
use serde::Serialize;
use std::{
    cell::LazyCell,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Once,
    },
    time::Duration,
};
use tauri::{Emitter, RunEvent, State};
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;

static CACHE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    directories::ProjectDirs::from("", "", "Cardinal")
        .expect(
            "Failed to get ProjectDirs: no valid home directory \
                path could be retrieved from the operating system",
        )
        .config_dir()
        .join("cardinal.db")
});

struct SearchState {
    search_tx: Sender<String>,
    result_rx: Receiver<Result<Vec<usize>>>,

    node_info_tx: Sender<Vec<usize>>,
    node_info_results_rx: Receiver<Vec<SearchNode>>,
}

#[tauri::command]
async fn search(query: String, state: State<'_, SearchState>) -> Result<Vec<usize>, String> {
    // 发送搜索请求到后台线程
    state
        .search_tx
        .send(query)
        .map_err(|e| format!("Failed to send search request: {:?}", e))?;

    // 等待搜索结果
    let search_result = state
        .result_rx
        .recv()
        .map_err(|e| format!("Failed to receive search result: {:?}", e))?;

    // 处理搜索结果
    search_result.map_err(|e| format!("Failed to process search result: {:?}", e))
}

#[derive(Serialize)]
struct NodeInfo {
    path: String,
    metadata: Option<SlabNodeMetadata>,
}

#[tauri::command]
async fn get_nodes_info(
    results: Vec<usize>,
    state: State<'_, SearchState>,
) -> Result<Vec<NodeInfo>, String> {
    state
        .node_info_tx
        .send(results)
        .map_err(|e| format!("Failed to send node info request: {:?}", e))?;

    let node_info_results = state
        .node_info_results_rx
        .recv()
        .map(|x| {
            x.into_iter()
                .map(|SearchNode { path, metadata }| NodeInfo {
                    path: path.to_string_lossy().into_owned(),
                    metadata,
                })
                .collect()
        })
        .map_err(|e| format!("Failed to receive node info results: {:?}", e))?;

    Ok(node_info_results)
}

#[tauri::command]
fn open_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let p = Path::new(&path);
        // On macOS, `open -R` reveals the file or directory in Finder.
        std::process::Command::new("open")
            .arg("-R")
            .arg(p)
            .spawn()
            .map_err(|e| format!("Failed to reveal path in Finder: {}", e))?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<()> {
    // Initialize the tracing subscriber to print logs to the command line
    let builder = tracing_subscriber::fmt();
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        builder.with_env_filter(filter).init();
    } else {
        builder.with_max_level(LevelFilter::INFO).init();
    }

    // Create communication channels
    let (finish_tx, finish_rx) = bounded::<Sender<SearchCache>>(1);
    let (search_tx, search_rx) = unbounded::<String>();
    let (result_tx, result_rx) = unbounded::<Result<Vec<usize>>>();
    let (node_info_tx, node_info_rx) = unbounded::<Vec<usize>>();
    let (node_info_results_tx, node_info_results_rx) = unbounded::<Vec<SearchNode>>();

    // 运行Tauri应用
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SearchState {
            search_tx,
            result_rx,
            node_info_tx,
            node_info_results_rx,
        })
        .invoke_handler(tauri::generate_handler![
            search,
            get_nodes_info,
            open_in_finder
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    let app_handle = app.handle().to_owned();
    // 启动后台处理线程
    std::thread::spawn(move || {
        const WATCH_ROOT: &str = "/";
        const FSE_LATENCY_SECS: f64 = 0.1;
        let path = PathBuf::from(WATCH_ROOT);
        let mut processed_events = 0;
        let emit_init = {
            let app_handle_clone = app_handle.clone();
            LazyCell::new(move || app_handle_clone.emit("init_completed", ()).unwrap())
        };
        // 初始化搜索缓存
        let mut cache = if let Ok(cached) =
            SearchCache::try_read_persistent_cache(&path, &CACHE_PATH)
        {
            info!("Loaded existing cache");
            // If using cache, defer the emit init process to HistoryDone event processing
            cached
        } else {
            info!("Walking filesystem...");
            let walk_data = Arc::new(WalkData::new(PathBuf::from("/System/Volumes/Data"), false));
            let walk_data_clone = walk_data.clone();
            let app_handle_clone = app_handle.clone();
            let walking_done = Arc::new(AtomicBool::new(false));
            let walking_done_clone = walking_done.clone();

            std::thread::spawn(move || {
                while !walking_done_clone.load(Ordering::Relaxed) {
                    let dirs = walk_data_clone.num_dirs.load(Ordering::Relaxed);
                    let files = walk_data_clone.num_files.load(Ordering::Relaxed);
                    app_handle_clone
                        .emit(
                            "status_update",
                            format!(
                                "Walking filesystem... {} directories, {} files...",
                                dirs, files
                            ),
                        )
                        .unwrap();
                    std::thread::sleep(Duration::from_millis(100));
                }
            });

            let cache = SearchCache::walk_fs_with_walk_data(path.clone(), &walk_data);
            walking_done.store(true, Ordering::Relaxed);
            // If full file system scan, emit initialized instantly.
            *emit_init;
            cache
        };

        // 启动事件监听器
        let mut event_watcher = EventWatcher::spawn(
            WATCH_ROOT.to_string(),
            cache.last_event_id(),
            FSE_LATENCY_SECS,
        );
        info!("Started background processing thread");
        loop {
            crossbeam_channel::select! {
                recv(finish_rx) -> tx => {
                    let tx = tx.expect("Finish channel closed");
                    tx.send(cache).expect("Failed to send cache");
                    break;
                }
                recv(search_rx) -> query => {
                    let query = query.expect("Search channel closed");
                    let result = cache.search(&query);
                    result_tx.send(result).expect("Failed to send result");
                }
                recv(node_info_rx) -> results => {
                    let results = results.expect("Node info channel closed");
                    let node_info_results = cache.expand_file_nodes(results);
                    node_info_results_tx.send(node_info_results).expect("Failed to send node info results");
                }
                recv(event_watcher.receiver) -> events => {
                    let events = events.expect("Event stream closed");
                    processed_events += events.len();
                    app_handle.emit("status_update", format!("Processing {} events...", processed_events)).unwrap();
                    // Emit HistoryDone inform frontend that cache is ready.
                    if events.iter().any(|x| x.flag == EventFlag::HistoryDone) {
                        *emit_init;
                    }
                    if let Err(HandleFSEError::Rescan) = cache.handle_fs_events(events) {
                        info!("!!!!!!!!!! Rescan triggered !!!!!!!!");
                        // Here we clear event_watcher first as rescan may take a lot of time
                        event_watcher.clear();
                        cache.rescan();
                        event_watcher = EventWatcher::spawn(WATCH_ROOT.to_string(), cache.last_event_id(), FSE_LATENCY_SECS);
                    }
                }
            }
        }
        info!("Background thread exited");
    });

    app.run(move |app_handle, event| {
        match &event {
            RunEvent::Exit => {
                // 右键关闭的时候会被调用
                // TODO(ldm0): 未来这里可以优化成不时保存一下，然后关闭的时候如果10秒内之前存过就不再存了

                // Write cache to file before app exit
                flush_cache_to_file_once(&finish_tx);
            }
            RunEvent::ExitRequested { api, code, .. } => {
                // 点击红色关闭气泡的时候会被调用，这时候窗口先关闭，然后托盘图标再慢慢退

                // Keep the event loop running even if all windows are closed
                // This allow us to catch tray icon events when there is no window
                // if we manually requested an exit (code is Some(_)) we will let it go through
                if code.is_none() {
                    info!("Tauri application exited, flushing cache...");

                    // TODO(ldm0): is this necessary?
                    api.prevent_exit();

                    // TODO(ldm0): change the tray icon to "saving"

                    flush_cache_to_file_once(&finish_tx);

                    app_handle.exit(0);
                }
            }
            _ => (),
        }
    });
    Ok(())
}

/// Write cache to file before app exit
fn flush_cache_to_file_once(finish_tx: &Sender<Sender<SearchCache>>) {
    static FLUSH_ONCE: Once = Once::new();
    FLUSH_ONCE.call_once(move || {
        let (cache_tx, cache_rx) = bounded::<SearchCache>(1);
        finish_tx
            .send(cache_tx)
            .context("cache_tx is closed")
            .unwrap();
        let cache = cache_rx.recv().context("cache_tx is closed").unwrap();
        cache
            .flush_to_file(&CACHE_PATH)
            .context("Failed to write cache to file")
            .unwrap();

        info!("Cache flushed successfully to {:?}", CACHE_PATH);
    });
}
