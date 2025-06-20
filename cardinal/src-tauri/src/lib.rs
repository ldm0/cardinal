// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use anyhow::{Context, Result};
use cardinal_sdk::{EventStream, FSEventStreamEventId, FsEvent};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use search_cache::{SearchCache, SearchNode};
use std::path::PathBuf;
use tauri::{Emitter, RunEvent, State};

struct SearchState {
    search_tx: Sender<String>,
    result_rx: Receiver<Result<Vec<SearchNode>>>,
}

#[tauri::command]
async fn search(query: &str, state: State<'_, SearchState>) -> Result<Vec<String>, String> {
    // 发送搜索请求到后台线程
    state
        .search_tx
        .send(query.to_string())
        .map_err(|e| format!("Failed to send search request: {e}"))?;

    // 等待搜索结果
    let search_result = state
        .result_rx
        .recv()
        .map_err(|e| format!("Failed to receive search result: {e}"))?;

    // 处理搜索结果
    search_result
        .map(|nodes| {
            nodes
                .into_iter()
                .map(|n| n.path.to_string_lossy().into_owned())
                .collect()
        })
        .map_err(|e| e.to_string())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<()> {
    // 创建通信通道
    let (finish_tx, finish_rx) = bounded::<Sender<SearchCache>>(1);
    let (search_tx, search_rx) = unbounded::<String>();
    let (result_tx, result_rx) = unbounded::<Result<Vec<SearchNode>>>();
    // 运行Tauri应用
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(SearchState {
            search_tx,
            result_rx,
        })
        .invoke_handler(tauri::generate_handler![search])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    let app_handle = app.handle().to_owned();
    // 启动后台处理线程
    std::thread::spawn(move || {
        // 初始化搜索缓存
        let path = PathBuf::from("/");
        let mut cache = if let Ok(cached) = SearchCache::try_read_persistent_cache(&path) {
            println!("Loaded existing cache");
            cached
        } else {
            println!("Walking filesystem...");
            SearchCache::walk_fs(path.clone())
        };

        app_handle.emit("init_completed", ()).unwrap();

        // 启动事件监听器
        let event_stream = spawn_event_watcher("/".to_string(), cache.last_event_id());
        println!("Started background processing thread");

        loop {
            crossbeam_channel::select! {
                recv(finish_rx) -> tx => {
                    let tx = tx.expect("Finish channel closed");
                    tx.send(cache).expect("Failed to send cache");
                    break;
                }
                recv(search_rx) -> query => {
                    let query = query.expect("Search channel closed");
                    let result = cache.query_files(query);
                    result_tx.send(result).expect("Failed to send result");
                }
                recv(event_stream) -> events => {
                    let events = events.expect("Event stream closed");
                    cache.handle_fs_events(events);
                }
            }
        }
        println!("Background thread exited");
    });

    app.run(move |app_handle, event| {
        match &event {
            RunEvent::ExitRequested { api, code, .. } => {
                // Keep the event loop running even if all windows are closed
                // This allow us to catch tray icon events when there is no window
                // if we manually requested an exit (code is Some(_)) we will let it go through
                if code.is_none() {
                    println!("Tauri application exited, flushing cache...");

                    // TODO(ldm0): is this necessary?
                    api.prevent_exit();

                    // TODO(ldm0): change the tray icon

                    let (cache_tx, cache_rx) = bounded::<SearchCache>(1);
                    finish_tx
                        .send(cache_tx)
                        .context("cache_tx is closed")
                        .unwrap();
                    let cache = cache_rx.recv().context("cache_tx is closed").unwrap();
                    cache
                        .flush_to_file()
                        .context("Failed to write cache to file")
                        .unwrap();

                    println!("Cache flushed successfully");

                    app_handle.exit(66);
                }
            }
            _ => (),
        }
    });
    Ok(())
}
