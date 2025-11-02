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
    path::{Path, PathBuf},
    sync::{
        LazyLock, Once,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, RunEvent, State};
use tracing::{error, info, level_filters::LevelFilter};
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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppLifecycleState {
    Initializing = 0,
    Ready = 1,
    Closing = 2,
}

impl AppLifecycleState {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Ready,
            2 => Self::Closing,
            _ => Self::Initializing,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Initializing => "Initializing",
            Self::Ready => "Ready",
            Self::Closing => "Closing",
        }
    }
}

static APP_LIFECYCLE_STATE: AtomicU8 = AtomicU8::new(AppLifecycleState::Initializing as u8);

fn load_app_state() -> AppLifecycleState {
    AppLifecycleState::from_u8(APP_LIFECYCLE_STATE.load(Ordering::Acquire))
}

fn store_app_state(state: AppLifecycleState) {
    APP_LIFECYCLE_STATE.store(state as u8, Ordering::Release);
}

fn emit_app_state(app_handle: &tauri::AppHandle) {
    if let Err(err) = app_handle.emit("app_lifecycle_state", load_app_state().as_str()) {
        error!("Failed to emit app_lifecycle_state event: {:?}", err);
    }
}

fn update_app_state(app_handle: &tauri::AppHandle, state: AppLifecycleState) {
    if load_app_state() == state {
        return;
    }
    store_app_state(state);
    emit_app_state(app_handle);
}

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
    // Send the search request to the background worker.
    state
        .search_tx
        .send(SearchJob { query, options })
        .map_err(|e| format!("Failed to send search request: {:?}", e))?;

    // Wait for the search result.
    let search_result = state
        .result_rx
        .recv()
        .map_err(|e| format!("Failed to receive search result: {:?}", e))?;

    // Propagate any search errors back to the caller.
    search_result.map_err(|e| format!("Failed to process search result: {:?}", e))
}

#[derive(Serialize)]
struct NodeInfo {
    path: String,
    metadata: Option<NodeInfoMetadata>,
    icon: Option<String>,
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

fn unix_timestamp_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

struct EventSnapshot {
    path: PathBuf,
    event_id: u64,
    flag: EventFlag,
    timestamp: i64,
}

// Forward new events to frontend without storing them in Rust
fn forward_new_events(app_handle: &tauri::AppHandle, snapshots: &[EventSnapshot]) {
    if snapshots.is_empty() {
        return;
    }

    let mut ordered_events: Vec<&EventSnapshot> = snapshots.iter().collect();
    ordered_events.sort_unstable_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.event_id.cmp(&b.event_id))
    });
    let new_events: Vec<RecentEvent> = ordered_events
        .into_iter()
        .map(|event| RecentEvent {
            path: event.path.to_string_lossy().into_owned(),
            flag_bits: event.flag.bits(),
            event_id: event.event_id,
            timestamp: event.timestamp,
        })
        .collect();

    // Emit new events to frontend - frontend will maintain the list
    let _ = app_handle.emit("fs_events_batch", new_events);
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecentEvent {
    path: String,
    flag_bits: u32,
    event_id: u64,
    timestamp: i64,
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
    icon: String,
}

fn run_background_event_loop(
    app_handle: &tauri::AppHandle,
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
) {
    let mut processed_events = 0usize;
    let mut history_ready = matches!(load_app_state(), AppLifecycleState::Ready);
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
                let node_info_results = cache.expand_file_nodes(&results);
                node_info_results_tx.send(node_info_results).expect("Failed to send node info results");
            }
            recv(icon_viewport_rx) -> update => {
                let (_request_id, viewport) = update.expect("Icon viewport channel closed");

                let nodes = cache.expand_file_nodes(&viewport);
                let icon_jobs: Vec<_> = viewport
                    .into_iter()
                    .zip(nodes.into_iter())
                    .map(|(slab_index, SearchResultNode { path, .. })| (slab_index, path))
                    .collect();

                if icon_jobs.is_empty() {
                    continue;
                }

                icon_jobs.into_iter().for_each(|(slab_index, path)| {
                    let icon_update_tx = icon_update_tx.clone();
                    spawn(move || {
                        if let Some(icon) = path
                            .to_str()
                            .and_then(fs_icon::icon_of_path_ql)
                            .map(|data| format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(&data)
                            )) {
                            let _ = icon_update_tx.send(IconPayload { slab_index, icon });
                        }
                    });
                });
            }
            recv(event_watcher.receiver) -> events => {
                let events = events.expect("Event stream closed");
                processed_events += events.len();

                // Emit a status-bar update with the current scan and event counts.
                app_handle.emit("status_bar_update", StatusBarUpdate {
                    scanned_files: cache.get_total_files(),
                    processed_events
                }).unwrap();

                let mut snapshots = Vec::with_capacity(events.len());
                for event in events.iter() {
                    if event.flag == EventFlag::HistoryDone {
                        history_ready = true;
                        update_app_state(app_handle, AppLifecycleState::Ready);
                    } else if history_ready {
                        snapshots.push(EventSnapshot {
                            path: event.path.clone(),
                            event_id: event.id,
                            flag: event.flag,
                            timestamp: unix_timestamp_now(),
                        });
                    }
                }

                let handle_result = cache.handle_fs_events(events);
                if let Err(HandleFSEError::Rescan) = handle_result {
                    info!("!!!!!!!!!! Rescan triggered !!!!!!!!");
                    event_watcher = EventWatcher::noop();
                    cache.rescan();
                    event_watcher = EventWatcher::spawn(watch_root.to_string(), cache.last_event_id(), fse_latency_secs).1;
                    update_app_state(app_handle, AppLifecycleState::Initializing);
                    history_ready = false;
                }

                if history_ready && !snapshots.is_empty() {
                    forward_new_events(app_handle, &snapshots);
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
        .map(|SearchResultNode { path, metadata }| {
            let path = path.to_string_lossy().into_owned();
            // icon_of_path_ns is fast enough, we can synchronously get basic
            // icon here, and then get QuickLook icon later(with push
            // icon_update).
            let icon = fs_icon::icon_of_path_ns(&path).map(|data| {
                format!(
                    "data:image/png;base64,{}",
                    general_purpose::STANDARD.encode(data)
                )
            });
            NodeInfo {
                path,
                icon,
                metadata: metadata.as_ref().map(NodeInfoMetadata::from_metadata),
            }
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
async fn get_app_status() -> Result<String, String> {
    Ok(load_app_state().as_str().to_string())
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
    let mut builder = tauri::Builder::default();
    #[cfg(not(feature = "dev"))]
    {
        builder = builder.plugin(tauri_plugin_prevent_default::init());
    }
    builder = builder.plugin(tauri_plugin_opener::init());
    // Run the Tauri application.
    let app = builder
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
            get_app_status,
            open_in_finder
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    let app_handle = &app.handle().to_owned();
    emit_app_state(app_handle);
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
            // Initialize the search cache.
            let mut cache = match SearchCache::try_read_persistent_cache(
                &path,
                &CACHE_PATH,
                Some(Path::new("/System/Volumes/Data")),
                Some(&APP_QUIT),
            ) {
                Ok(cached) => {
                    info!("Loaded existing cache");
                    // If using cache, defer the emit init process to HistoryDone event processing

                    // Emit the initial status-bar payload.
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

                    // Emit the initial status-bar payload.
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
                    update_app_state(app_handle, AppLifecycleState::Ready);
                    cache
                }
            };

            // Start the filesystem event watcher.
            let event_watcher = EventWatcher::spawn(
                WATCH_ROOT.to_string(),
                cache.last_event_id(),
                FSE_LATENCY_SECS,
            ).1;
            if !matches!(load_app_state(), AppLifecycleState::Ready) {
                update_app_state(app_handle, AppLifecycleState::Initializing);
            }
            info!("Started background processing thread");
            run_background_event_loop(
                &app_handle,
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
                    update_app_state(app_handle, AppLifecycleState::Closing);
                    APP_QUIT.store(true, Ordering::Relaxed);
                    // Triggered when the tray context menu requests an exit.
                    // TODO(ldm0): Periodically save the cache so a close event within ~10s can skip another flush.

                    // Write cache to file before app exit
                    flush_cache_to_file_once(&finish_tx);
                }
                RunEvent::ExitRequested { api, code, .. } => {
                    // Triggered by the window close button; the window exits first and the tray follows.

                    // Keep the event loop running even if all windows are closed
                    // This allow us to catch tray icon events when there is no window
                    // if we manually requested an exit (code is Some(_)) we will let it go through
                    if code.is_none() {
                        update_app_state(app_handle, AppLifecycleState::Closing);
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
