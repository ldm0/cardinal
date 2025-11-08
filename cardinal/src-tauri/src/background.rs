use crate::{
    commands::SearchJob,
    lifecycle::{AppLifecycleState, load_app_state, update_app_state},
};
use anyhow::Result as AnyhowResult;
use base64::{Engine as _, engine::general_purpose};
use cardinal_sdk::{EventFlag, EventWatcher};
use crossbeam_channel::{Receiver, Sender};
use rayon::spawn;
use search_cache::{HandleFSEError, SearchCache, SearchOptions, SearchResultNode, SlabIndex};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tracing::info;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatusBarUpdate {
    pub scanned_files: usize,
    pub processed_events: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IconPayload {
    pub slab_index: SlabIndex,
    pub icon: String,
}

pub struct BackgroundLoopChannels {
    pub finish_rx: Receiver<Sender<Option<SearchCache>>>,
    pub search_rx: Receiver<SearchJob>,
    pub result_tx: Sender<AnyhowResult<Vec<SlabIndex>>>,
    pub node_info_rx: Receiver<Vec<SlabIndex>>,
    pub node_info_results_tx: Sender<Vec<SearchResultNode>>,
    pub icon_viewport_rx: Receiver<(u64, Vec<SlabIndex>)>,
    pub rescan_rx: Receiver<()>,
    pub icon_update_tx: Sender<IconPayload>,
    pub search_version: Arc<AtomicU64>,
}

pub fn emit_status_bar_update(
    app_handle: &AppHandle,
    scanned_files: usize,
    processed_events: usize,
) {
    app_handle
        .emit(
            "status_bar_update",
            StatusBarUpdate {
                scanned_files,
                processed_events,
            },
        )
        .unwrap();
}

struct EventSnapshot {
    path: PathBuf,
    event_id: u64,
    flag: EventFlag,
    timestamp: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecentEvent {
    path: String,
    flag_bits: u32,
    event_id: u64,
    timestamp: i64,
}

pub fn run_background_event_loop(
    app_handle: &AppHandle,
    mut cache: SearchCache,
    mut event_watcher: EventWatcher,
    channels: BackgroundLoopChannels,
    watch_root: &str,
    fse_latency_secs: f64,
) {
    let BackgroundLoopChannels {
        finish_rx,
        search_rx,
        result_tx,
        node_info_rx,
        node_info_results_tx,
        icon_viewport_rx,
        rescan_rx,
        icon_update_tx,
        search_version,
    } = channels;
    let mut processed_events = 0usize;
    let mut history_ready = load_app_state() == AppLifecycleState::Ready;
    loop {
        crossbeam_channel::select! {
            recv(finish_rx) -> tx => {
                let tx = tx.expect("Finish channel closed");
                tx.send(Some(cache)).expect("Failed to send cache");
                return;
            }
            recv(search_rx) -> job => {
                let SearchJob {
                    query,
                    options,
                    version,
                } = job.expect("Search channel closed");
                let opts = SearchOptions::from(options);
                let result = if query.is_empty() {
                    Ok(cache.search_empty())
                } else {
                    cache.search_with_options(
                        &query,
                        opts,
                        Some((version, search_version.as_ref())),
                    )
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

                icon_jobs
                    .into_iter()
                    .map(|(slab_index, path)| (slab_index, path.to_string_lossy().into_owned()))
                    .filter(|(_, path)| !path.contains("OneDrive") && !path.contains("com~apple~CloudDocs"))
                    .for_each(|(slab_index, path)| {
                        let icon_update_tx = icon_update_tx.clone();
                        spawn(move || {
                            if let Some(icon) = fs_icon::icon_of_path_ql(&path).map(|data| format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(&data)
                            )) {
                                let _ = icon_update_tx.send(IconPayload { slab_index, icon });
                            }
                        });
                    });
            }
            recv(rescan_rx) -> request => {
                request.expect("Rescan channel closed");
                info!("Manual rescan requested");
                perform_rescan(
                    app_handle,
                    &mut cache,
                    &mut event_watcher,
                    watch_root,
                    fse_latency_secs,
                    &mut history_ready,
                );
            }
            recv(event_watcher) -> events => {
                let events = events.expect("Event stream closed");
                processed_events += events.len();

                emit_status_bar_update(app_handle, cache.get_total_files(), processed_events);

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
                    perform_rescan(
                        app_handle,
                        &mut cache,
                        &mut event_watcher,
                        watch_root,
                        fse_latency_secs,
                        &mut history_ready,
                    );
                }

                if history_ready && !snapshots.is_empty() {
                    forward_new_events(app_handle, &snapshots);
                }
            }
        }
    }
}

fn perform_rescan(
    app_handle: &AppHandle,
    cache: &mut SearchCache,
    event_watcher: &mut EventWatcher,
    watch_root: &str,
    fse_latency_secs: f64,
    history_ready: &mut bool,
) {
    *event_watcher = EventWatcher::noop();
    update_app_state(app_handle, AppLifecycleState::Initializing);
    emit_status_bar_update(app_handle, 0, 0);
    *history_ready = false;

    let walk_data = cache.walk_data();
    let walking_done = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !walking_done.load(Ordering::Relaxed) {
                let dirs = walk_data.num_dirs.load(Ordering::Relaxed);
                let files = walk_data.num_files.load(Ordering::Relaxed);
                let total = dirs + files;
                emit_status_bar_update(app_handle, total, 0);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        cache.rescan_with_walk_data(&walk_data);
        walking_done.store(true, Ordering::Relaxed);
    });

    *event_watcher = EventWatcher::spawn(
        watch_root.to_string(),
        cache.last_event_id(),
        fse_latency_secs,
    )
    .1;
    update_app_state(app_handle, AppLifecycleState::Updating);
}

fn unix_timestamp_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn forward_new_events(app_handle: &AppHandle, snapshots: &[EventSnapshot]) {
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

    let _ = app_handle.emit("fs_events_batch", new_events);
}
