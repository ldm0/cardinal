mod background;
mod commands;
mod lifecycle;
mod tray;
mod window_controls;

use anyhow::{Context, Result};
use background::{
    BackgroundLoopChannels, IconPayload, emit_status_bar_update, run_background_event_loop,
};
use cardinal_sdk::EventWatcher;
use commands::{
    NodeInfoRequest, SearchJob, SearchState, get_app_status, get_nodes_info, open_in_finder,
    preview_with_quicklook, search, trigger_rescan, update_icon_viewport,
};
use crossbeam_channel::{Sender, bounded, unbounded};
use lifecycle::{
    APP_QUIT, AppLifecycleState, EXIT_REQUESTED, emit_app_state, load_app_state, update_app_state,
};
use search_cache::{SearchCache, SlabIndex, WalkData};
use std::{
    path::{Path, PathBuf},
    sync::{
        LazyLock, Once,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;
use tracing::{info, level_filters::LevelFilter, warn};
use tracing_subscriber::EnvFilter;
use tray::setup_tray;
use window_controls::{WindowToggle, activate_window, hide_window, toggle_window};

static CACHE_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    directories::ProjectDirs::from("", "", "Cardinal")
        .expect(
            "Failed to get ProjectDirs: no valid home directory \
                path could be retrieved from the operating system",
        )
        .config_dir()
        .join("cardinal.db")
});
const QUICK_LAUNCH_SHORTCUT: &str = "CmdOrCtrl+Shift+Space";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<()> {
    let builder = tracing_subscriber::fmt();
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        builder.with_env_filter(filter).init();
    } else {
        builder.with_max_level(LevelFilter::INFO).init();
    }

    let (finish_tx, finish_rx) = bounded::<Sender<Option<SearchCache>>>(1);
    let (search_tx, search_rx) = unbounded::<SearchJob>();
    let (result_tx, result_rx) = unbounded::<anyhow::Result<Vec<SlabIndex>>>();
    let (node_info_tx, node_info_rx) = unbounded::<NodeInfoRequest>();
    let (icon_viewport_tx, icon_viewport_rx) = unbounded::<(u64, Vec<SlabIndex>)>();
    let (rescan_tx, rescan_rx) = unbounded::<()>();
    let (icon_update_tx, icon_update_rx) = unbounded::<IconPayload>();

    let quick_launch_shortcut_plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_shortcut(QUICK_LAUNCH_SHORTCUT)
        .expect("invalid quick launch shortcut definition")
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                let Some(window) = app.get_webview_window("main") else {
                    warn!("Toggle requested but main window is unavailable");
                    return;
                };

                if matches!(toggle_window(&window), WindowToggle::Hidden) {
                    info!("Global shortcut hid the Cardinal window");
                }
            }
        })
        .build();

    let mut builder = tauri::Builder::default();
    #[cfg(not(feature = "dev"))]
    {
        builder = builder.plugin(tauri_plugin_prevent_default::init());
    }
    builder = builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(quick_launch_shortcut_plugin)
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }

            let WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };

            if EXIT_REQUESTED.load(Ordering::Relaxed) {
                return;
            }

            api.prevent_close();

            let Some(window) = window.get_webview_window("main") else {
                warn!("Close requested but main window is unavailable");
                return;
            };

            if hide_window(&window) {
                info!("Main window hidden; Cardinal keeps running in the background");
            }
        });

    let app = builder
        .manage(SearchState::new(
            search_tx,
            result_rx,
            node_info_tx,
            icon_viewport_tx.clone(),
            rescan_tx.clone(),
        ))
        .invoke_handler(tauri::generate_handler![
            search,
            get_nodes_info,
            update_icon_viewport,
            get_app_status,
            trigger_rescan,
            open_in_finder,
            preview_with_quicklook
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    setup_tray(&app).expect("failed to initialize system tray");

    let app_handle = &app.handle().to_owned();
    emit_app_state(app_handle);
    let icon_update_rx = &icon_update_rx;
    std::thread::scope(move |s| {
        s.spawn(|| {
            while let Ok(icon) = icon_update_rx.recv() {
                let mut icons = vec![icon];
                std::thread::sleep(Duration::from_millis(100));
                icons.extend(icon_update_rx.try_iter());
                info!("emitting {} icons", icons.len());
                app_handle.emit("icon_update", icons).unwrap();
            }
            info!("icon update thread exited");
        });

        s.spawn(move || {
            if !has_full_disk_access(app_handle) {
                info!("App does not have Full Disk Access, sleeping indefinitely");
                while !APP_QUIT.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                }
                info!("Background thread quitting without Full Disk Access");
                return;
            }

            const WATCH_ROOT: &str = "/";
            const FSE_LATENCY_SECS: f64 = 0.1;
            let path = PathBuf::from(WATCH_ROOT);

            let mut cache = match SearchCache::try_read_persistent_cache(
                &path,
                &CACHE_PATH,
                Some(Path::new("/System/Volumes/Data")),
                Some(&APP_QUIT),
            ) {
                Ok(cached) => {
                    info!("Loaded existing cache");
                    emit_status_bar_update(app_handle, cached.get_total_files(), 0);
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
                                emit_status_bar_update(app_handle, total, 0);
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

                    emit_status_bar_update(app_handle, cache.get_total_files(), 0);

                    cache
                }
            };

            let event_watcher = EventWatcher::spawn(
                WATCH_ROOT.to_string(),
                cache.last_event_id(),
                FSE_LATENCY_SECS,
            )
            .1;
            if !matches!(load_app_state(), AppLifecycleState::Ready) {
                update_app_state(app_handle, AppLifecycleState::Initializing);
            }
            info!("Started background processing thread");
            run_background_event_loop(
                app_handle,
                cache,
                event_watcher,
                BackgroundLoopChannels {
                    finish_rx,
                    search_rx,
                    result_tx,
                    node_info_rx,
                    icon_viewport_rx,
                    rescan_rx,
                    icon_update_tx,
                },
                WATCH_ROOT,
                FSE_LATENCY_SECS,
            );

            info!("Background thread exited");
        });

        app.run(move |app_handle, event| match event {
            RunEvent::Exit => {
                APP_QUIT.store(true, Ordering::Relaxed);
                flush_cache_to_file_once(&finish_tx);
            }
            RunEvent::ExitRequested { api, code, .. } => {
                let already_requested = EXIT_REQUESTED.swap(true, Ordering::Relaxed);
                APP_QUIT.store(true, Ordering::Relaxed);
                if !already_requested {
                    info!(
                        "Exit requested (code: {:?}); flushing cache before shutdown",
                        code
                    );
                }

                flush_cache_to_file_once(&finish_tx);

                if code.is_none() {
                    api.prevent_exit();
                    app_handle.exit(0);
                }
            }
            RunEvent::Reopen {
                has_visible_windows,
                ..
            } => {
                if !has_visible_windows {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        activate_window(&window);
                    } else {
                        warn!("Reopen requested but main window is unavailable");
                    }
                }
            }
            _ => {}
        });
    });

    Ok(())
}

fn flush_cache_to_file_once(finish_tx: &Sender<Sender<Option<SearchCache>>>) {
    static FLUSH_ONCE: Once = Once::new();
    if load_app_state() == AppLifecycleState::Initializing {
        info!("App not fully initialized, skipping cache flush");
        return;
    }
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

fn has_full_disk_access<R: Runtime>(app_handle: &AppHandle<R>) -> bool {
    let check_dirs = ["Library/Containers/com.apple.stocks", "Library/Safari"];

    if let Ok(home_dir) = app_handle.path().home_dir() {
        for check_dir in check_dirs.iter() {
            if std::fs::read_dir(home_dir.join(check_dir)).is_ok() {
                return true;
            }
        }
    }

    false
}
