use crate::lifecycle::load_app_state;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use crossbeam_channel::{Receiver, Sender};
use search_cache::{SearchOptions, SearchResultNode, SlabIndex, SlabNodeMetadata};
use serde::{Deserialize, Serialize};
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::State;

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptionsPayload {
    #[serde(default)]
    pub use_regex: bool,
    #[serde(default)]
    pub case_insensitive: bool,
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
pub struct SearchJob {
    pub query: String,
    pub options: SearchOptionsPayload,
    pub version: u64,
}

pub struct SearchState {
    search_tx: Sender<SearchJob>,
    result_rx: Receiver<Result<Vec<SlabIndex>>>,

    node_info_tx: Sender<Vec<SlabIndex>>,
    node_info_results_rx: Receiver<Vec<SearchResultNode>>,

    icon_viewport_tx: Sender<(u64, Vec<SlabIndex>)>,
    rescan_tx: Sender<()>,
    search_version: Arc<AtomicU64>,
}

impl SearchState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        search_tx: Sender<SearchJob>,
        result_rx: Receiver<Result<Vec<SlabIndex>>>,
        node_info_tx: Sender<Vec<SlabIndex>>,
        node_info_results_rx: Receiver<Vec<SearchResultNode>>,
        icon_viewport_tx: Sender<(u64, Vec<SlabIndex>)>,
        rescan_tx: Sender<()>,
        search_version: Arc<AtomicU64>,
    ) -> Self {
        Self {
            search_tx,
            result_rx,
            node_info_tx,
            node_info_results_rx,
            icon_viewport_tx,
            rescan_tx,
            search_version,
        }
    }
}

#[derive(Serialize)]
pub struct NodeInfo {
    pub path: String,
    pub metadata: Option<NodeInfoMetadata>,
    pub icon: Option<String>,
}

#[derive(Serialize)]
pub struct NodeInfoMetadata {
    pub r#type: u8,
    pub size: u64,
    pub ctime: u32,
    pub mtime: u32,
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

#[tauri::command]
pub async fn search(
    query: String,
    options: Option<SearchOptionsPayload>,
    version: u64,
    state: State<'_, SearchState>,
) -> Result<Vec<SlabIndex>, String> {
    let options = options.unwrap_or_default();
    state.search_version.swap(version, Ordering::SeqCst);
    state
        .search_tx
        .send(SearchJob {
            query,
            options,
            version,
        })
        .map_err(|e| format!("Failed to send search request: {e:?}"))?;

    let search_result = state
        .result_rx
        .recv()
        .map_err(|e| format!("Failed to receive search result: {e:?}"))?;

    search_result.map_err(|e| format!("Failed to process search result: {e:?}"))
}

#[tauri::command]
pub async fn get_nodes_info(
    results: Vec<SlabIndex>,
    state: State<'_, SearchState>,
) -> Result<Vec<NodeInfo>, String> {
    if results.is_empty() {
        return Ok(Vec::new());
    }

    state
        .node_info_tx
        .send(results.clone())
        .map_err(|e| format!("Failed to send node info request: {e:?}"))?;

    let nodes = state
        .node_info_results_rx
        .recv()
        .map_err(|e| format!("Failed to receive node info results: {e:?}"))?;

    let node_infos = nodes
        .into_iter()
        .map(|SearchResultNode { path, metadata }| {
            let path = path.to_string_lossy().into_owned();
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
pub async fn update_icon_viewport(
    id: u64,
    viewport: Vec<SlabIndex>,
    state: State<'_, SearchState>,
) -> Result<(), String> {
    state
        .icon_viewport_tx
        .send((id, viewport))
        .map_err(|e| format!("Failed to send icon viewport update: {e:?}"))
}

#[tauri::command]
pub async fn get_app_status() -> Result<String, String> {
    Ok(load_app_state().as_str().to_string())
}

#[tauri::command]
pub async fn trigger_rescan(state: State<'_, SearchState>) -> Result<(), String> {
    state
        .rescan_tx
        .send(())
        .map_err(|e| format!("Failed to request rescan: {e:?}"))?;
    Ok(())
}

#[tauri::command]
pub fn open_in_finder(path: String) -> Result<(), String> {
    Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to reveal path in Finder: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn preview_with_quicklook(path: String) -> Result<(), String> {
    Command::new("qlmanage")
        .arg("-p")
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to launch Quick Look preview: {e}"))?;
    Ok(())
}
