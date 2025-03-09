#![deny(unsafe_op_in_unsafe_fn)]
mod c;
mod consts;
mod database;
mod fs_entry;
mod processor;
mod runtime;
mod utils;

use anyhow::{Context, Result};
pub use c::*;
use cardinal_sdk::fsevent;
use cardinal_sdk::fsevent::spawn_event_watcher;
use consts::DB_PATH;
pub use database::Database;
use fsevent::FsEvent;
pub use processor::take_fs_events;
use runtime::runtime;
use std::path::Path;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info};

fn spawn_event_processor(
    database: Option<Database>,
    mut receiver: UnboundedReceiver<Vec<FsEvent>>,
) -> Result<()> {
    // unwrap is legal here since processor is always init.
    runtime().spawn(async move {
        let result = processor::PROCESSOR.block_on(database, &mut receiver).await;
        info!("event processor done: {:?}", result);
    });
    Ok(())
}

pub fn close_event_processor() -> Result<()> {
    processor::PROCESSOR
        .close()
        .context("Close global processor failed.")
}

pub fn init_sdk_facade() {
    if let Err(error) = init_sdk() {
        error!(?error, "init sdk failed");
    }
}

pub fn close_sdk_facade() {
    if let Err(error) = close_sdk() {
        error!(?error, "close sdk failed")
    }
}

fn init_sdk() -> Result<()> {
    let database = {
        let database = Database::from_fs(Path::new(DB_PATH));
        if let Err(error) = &database {
            info!(?error, "database not found");
        }
        database.ok()
    };

    let watch_event_since = match database.as_ref() {
        Some(x) => x.last_event_id(),
        None => utils::current_event_id(),
    };

    info!("Watching event since: {}", watch_event_since);
    // A global event watcher spawned on a dedicated thread.
    let receiver = spawn_event_watcher(watch_event_since);
    // A global event processor spawned on a dedicated thread.
    spawn_event_processor(database, receiver).context("spawn event processor failed")?;
    Ok(())
}

fn close_sdk() -> Result<()> {
    close_event_processor().context("close event processor failed")?;
    Ok(())
}
