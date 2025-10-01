// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose};
use cardinal_sdk::{EventFlag, EventWatcher};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use rayon::spawn;
use search_cache::{
    HandleFSEError, SearchCache, SearchOptions, SearchResultNode, SlabIndex, SlabNodeMetadata,
    WalkData,
};
use serde::{Deserialize, Serialize};
use std::{
    cell::LazyCell,
    path::{Path, PathBuf},
    sync::{
        LazyLock, Once,
        atomic::{AtomicBool, Ordering},
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
static APP_QUIT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SearchOptionsPayload {
    #[serde(default)]
    use_regex: bool,
    #[serde(default)]
    case_insensitive: bool,
}

impl From<SearchOptionsPayload> for SearchOptions {
    fn from(
        SearchOptionsPayload {
            use_regex,
            case_insensitive,
        }: SearchOptionsPayload,
    ) -> Self {
        SearchOptions {
            use_regex,
            case_insensitive,
        }
    }
}

#[derive(Debug, Clone)]
struct SearchJob {
    query: String,
    options: SearchOptionsPayload,
}

struct SearchState {
    search_tx: Sender<SearchJob>,
    result_rx: Receiver<Result<Vec<SlabIndex>>>,

    node_info_tx: Sender<Vec<SlabIndex>>,
    node_info_results_rx: Receiver<Vec<SearchResultNode>>,

    icon_viewport_tx: Sender<(u64, Vec<SlabIndex>)>,
}

#[tauri::command]
async fn search(
    query: String,
    options: Option<SearchOptionsPayload>,
    state: State<'_, SearchState>,
) -> Result<Vec<SlabIndex>, String> {
    let options = options.unwrap_or_default();
    // 发送搜索请求到后台线程
    state
        .search_tx
        .send(SearchJob { query, options })
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
    metadata: Option<NodeInfoMetadata>,
}

#[derive(Serialize)]
struct NodeInfoMetadata {
    r#type: u8,
    size: u64,
    ctime: u32,
    mtime: u32,
}

impl NodeInfoMetadata {
    pub fn from_metadata(metadata: SlabNodeMetadata<'_>) -> Self {
        Self {
            r#type: metadata.r#type() as u8,
            size: metadata.size(),
            ctime: metadata.ctime().map(|x| x.get()).unwrap_or_default(),
            mtime: metadata.mtime().map(|x| x.get()).unwrap_or_default(),
        }
    }
}

#[derive(Serialize, Clone)]
struct StatusBarUpdate {
    scanned_files: usize,
    processed_events: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct IconPayload {
    slab_index: SlabIndex,
    icon: Option<String>,
}

fn run_background_event_loop<F>(
    app_handle: &tauri::AppHandle,
    emit_init: &LazyCell<(), F>,
    mut cache: SearchCache,
    mut event_watcher: EventWatcher,
    finish_rx: Receiver<Sender<Option<SearchCache>>>,
    search_rx: Receiver<SearchJob>,
    result_tx: Sender<Result<Vec<SlabIndex>>>,
    node_info_rx: Receiver<Vec<SlabIndex>>,
    node_info_results_tx: Sender<Vec<SearchResultNode>>,
    icon_viewport_rx: Receiver<(u64, Vec<SlabIndex>)>,
    icon_update_tx: Sender<IconPayload>,
    watch_root: &str,
    fse_latency_secs: f64,
) where
    F: FnOnce() -> (),
{
    let mut processed_events = 0usize;
    loop {
        crossbeam_channel::select! {
            recv(finish_rx) -> tx => {
                let tx = tx.expect("Finish channel closed");
                tx.send(Some(cache)).expect("Failed to send cache");
                return;
            }
            recv(search_rx) -> job => {
                let SearchJob { query, options } = job.expect("Search channel closed");
                let opts: SearchOptions = options.into();
                let result = if query.is_empty() {
                    Ok(cache.search_empty())
                } else {
                    cache.search_with_options(&query, opts)
                };
                result_tx.send(result).expect("Failed to send result");
            }
            recv(node_info_rx) -> results => {
                let results = results.expect("Node info channel closed");
                let node_info_results = cache.expand_file_nodes(results);
                node_info_results_tx.send(node_info_results).expect("Failed to send node info results");
            }
            recv(icon_viewport_rx) -> update => {
                let (_request_id, viewport) = update.expect("Icon viewport channel closed");

                let nodes = cache.expand_file_nodes(viewport.clone());
                let icon_jobs: Vec<_> = viewport
                    .into_iter()
                    .zip(nodes.into_iter())
                    .map(|(slab_index, SearchResultNode { path, .. })| (slab_index, path))
                    .collect();

                if icon_jobs.is_empty() {
                    continue;
                }

                let icon_update_tx = icon_update_tx.clone();
                spawn(move || {
                    icon_jobs.into_iter().for_each(|(slab_index, path)| {
                        let icon = path
                            .to_str()
                            .and_then(fs_icon::icon_of_path)
                            .map(|data| format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(&data)
                            ));

                        let _ = icon_update_tx.send(IconPayload { slab_index, icon });
                    });
                });
            }
            recv(event_watcher.receiver) -> events => {
                let events = events.expect("Event stream closed");
                processed_events += events.len();

                // 发送状态栏更新事件，包含文件扫描数和处理的事件数
                app_handle.emit("status_bar_update", StatusBarUpdate {
                    scanned_files: cache.get_total_files(),
                    processed_events
                }).unwrap();

                // Emit HistoryDone inform frontend that cache is ready.
                if events.iter().any(|x| x.flag == EventFlag::HistoryDone) {
                    LazyCell::force(emit_init);
                }
                if let Err(HandleFSEError::Rescan) = cache.handle_fs_events(events) {
                    info!("!!!!!!!!!! Rescan triggered !!!!!!!!");
                    // Here we clear event_watcher first as rescan may take a lot of time
                    event_watcher.clear();
                    cache.rescan();
                    event_watcher = EventWatcher::spawn(watch_root.to_string(), cache.last_event_id(), fse_latency_secs);
                }
            }
        }
    }
}

#[tauri::command]
async fn get_nodes_info(
    results: Vec<SlabIndex>,
    state: State<'_, SearchState>,
) -> Result<Vec<NodeInfo>, String> {
    if results.is_empty() {
        return Ok(Vec::new());
    }

    state
        .node_info_tx
        .send(results.clone())
        .map_err(|e| format!("Failed to send node info request: {:?}", e))?;

    let nodes = state
        .node_info_results_rx
        .recv()
        .map_err(|e| format!("Failed to receive node info results: {:?}", e))?;

    let node_infos = nodes
        .into_iter()
        .map(|SearchResultNode { path, metadata }| NodeInfo {
            path: path.to_string_lossy().into_owned(),
            metadata: metadata.as_ref().map(NodeInfoMetadata::from_metadata),
        })
        .collect();

    Ok(node_infos)
}

#[tauri::command]
async fn update_icon_viewport(
    id: u64,
    viewport: Vec<SlabIndex>,
    state: State<'_, SearchState>,
) -> Result<(), String> {
    state
        .icon_viewport_tx
        .send((id, viewport))
        .map_err(|e| format!("Failed to send icon viewport update: {:?}", e))
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
    #[cfg(target_os = "windows")]
    {
        let p = Path::new(&path);
        // On Windows, use explorer.exe /select to reveal the file.
        std::process::Command::new("explorer.exe")
            .arg("/select,")
            .arg(p)
            .spawn()
            .map_err(|e| format!("Failed to reveal path in Explorer: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        let p = Path::new(&path);
        // On Linux, use xdg-open to open the parent directory.
        if let Some(parent) = p.parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| format!("Failed to open parent directory: {}", e))?;
        } else {
            std::process::Command::new("xdg-open")
                .arg(p)
                .spawn()
                .map_err(|e| format!("Failed to open path: {}", e))?;
        }
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
    let (finish_tx, finish_rx) = bounded::<Sender<Option<SearchCache>>>(1);
    let (search_tx, search_rx) = unbounded::<SearchJob>();
    let (result_tx, result_rx) = unbounded();
    let (node_info_tx, node_info_rx) = unbounded();
    let (node_info_results_tx, node_info_results_rx) = unbounded::<Vec<SearchResultNode>>();
    let (icon_viewport_tx, icon_viewport_rx) = unbounded::<(u64, Vec<SlabIndex>)>();
    let (icon_update_tx, icon_update_rx) = unbounded::<IconPayload>();

    // 运行Tauri应用
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SearchState {
            search_tx,
            result_rx,
            node_info_tx,
            node_info_results_rx,
            icon_viewport_tx: icon_viewport_tx.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            search,
            get_nodes_info,
            update_icon_viewport,
            open_in_finder
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    let app_handle = &app.handle().to_owned();
    let icon_update_rx = &icon_update_rx;
    std::thread::scope(move |s| {
        // Init icon update thread
        s.spawn(|| {
            while let Ok(icon) = icon_update_rx.recv() {
                let mut icons = vec![icon];
                // Batch icon updates within 100ms
                std::thread::sleep(Duration::from_millis(100));
                icons.extend(icon_update_rx.try_iter());
                info!("emitting {} icons", icons.len());
                app_handle.emit("icon_update", icons).unwrap();
            }
            info!("icon update thread exited");
        });
        // Init background event processing thread
        s.spawn(move || {
            const WATCH_ROOT: &str = "/";
            const FSE_LATENCY_SECS: f64 = 0.1;
            let path = PathBuf::from(WATCH_ROOT);
            let emit_init = LazyCell::new(|| app_handle.emit("init_completed", ()).unwrap());
            // 初始化搜索缓存
            let mut cache = match SearchCache::try_read_persistent_cache(
                &path,
                &CACHE_PATH,
                Some(Path::new("/System/Volumes/Data")),
                Some(&APP_QUIT),
            ) {
                Ok(cached) => {
                    info!("Loaded existing cache");
                    // If using cache, defer the emit init process to HistoryDone event processing

                    // 发送初始状态栏信息
                    app_handle
                        .emit(
                            "status_bar_update",
                            StatusBarUpdate {
                                scanned_files: cached.get_total_files(),
                                processed_events: 0,
                            },
                        )
                        .unwrap();

                    cached
                }
                Err(e) => {
                    info!("Walking filesystem: {:?}", e);
                    let walk_data = WalkData::new(
                        Some(Path::new("/System/Volumes/Data")),
                        false,
                        Some(&APP_QUIT),
                    );
                    let walking_done = AtomicBool::new(false);
                    let cache = std::thread::scope(|s| {
                        s.spawn(|| {
                            while !walking_done.load(Ordering::Relaxed) {
                                let dirs = walk_data.num_dirs.load(Ordering::Relaxed);
                                let files = walk_data.num_files.load(Ordering::Relaxed);
                                let total = dirs + files;
                                app_handle
                                    .emit(
                                        "status_bar_update",
                                        StatusBarUpdate {
                                            scanned_files: total,
                                            processed_events: 0,
                                        },
                                    )
                                    .unwrap();
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        });
                        let cache = SearchCache::walk_fs_with_walk_data(
                            path.clone(),
                            &walk_data,
                            Some(Path::new("/System/Volumes/Data")),
                            Some(&APP_QUIT),
                        );

                        walking_done.store(true, Ordering::Relaxed);

                        cache
                    });

                    let Some(cache) = cache else {
                        info!("Walk filesystem cancelled, app quitting");
                        finish_rx
                            .recv()
                            .expect("Failed to receive finish signal")
                            .send(None)
                            .expect("Failed to send None cache");
                        return;
                    };

                    // 发送初始状态栏信息
                    app_handle
                        .emit(
                            "status_bar_update",
                            StatusBarUpdate {
                                scanned_files: cache.get_total_files(),
                                processed_events: 0,
                            },
                        )
                        .unwrap();

                    // If full file system scan, emit initialized instantly.
                    *emit_init;
                    cache
                }
            };

            // 启动事件监听器
            let event_watcher = EventWatcher::spawn(
                WATCH_ROOT.to_string(),
                cache.last_event_id(),
                FSE_LATENCY_SECS,
            );
            info!("Started background processing thread");
            run_background_event_loop(
                &app_handle,
                &emit_init,
                cache,
                event_watcher,
                finish_rx,
                search_rx,
                result_tx,
                node_info_rx,
                node_info_results_tx,
                icon_viewport_rx,
                icon_update_tx,
                WATCH_ROOT,
                FSE_LATENCY_SECS,
            );

            info!("Background thread exited");
        });

        app.run(move |app_handle, event| {
            match &event {
                RunEvent::Exit => {
                    APP_QUIT.store(true, Ordering::Relaxed);
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
                        APP_QUIT.store(true, Ordering::Relaxed);
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
    });
    Ok(())
}

/// Write cache to file before app exit
fn flush_cache_to_file_once(finish_tx: &Sender<Sender<Option<SearchCache>>>) {
    static FLUSH_ONCE: Once = Once::new();
    FLUSH_ONCE.call_once(move || {
        let (cache_tx, cache_rx) = bounded::<Option<SearchCache>>(1);
        finish_tx
            .send(cache_tx)
            .context("cache_tx is closed")
            .unwrap();
        if let Some(cache) = cache_rx.recv().context("cache_tx is closed").unwrap() {
            cache
                .flush_to_file(&CACHE_PATH)
                .context("Failed to write cache to file")
                .unwrap();

            info!("Cache flushed successfully to {:?}", &*CACHE_PATH);
        } else {
            info!("Canncelled during data construction, no cache to flush");
        }
    });
}
