mod consts;
mod database;
mod disk_entry;
mod event_stream;
mod fs_visitor;
mod fsevent;
mod models;
mod schema;
mod utils;

use database::Database;
use fsevent::FsEvent;
use tracing::error;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    // let _ = std::fs::remove_file(DATABASE_URL);
    let mut db = Database::from_fs().unwrap();
    let mut receiver = event_stream::spawn_event_watcher(db.event_id);
    loop {
        tokio::select! {
            fs_event = receiver.recv() => {
                let fs_event = fs_event.unwrap();
                merge_event(&mut db, fs_event);
            }
        }
    }
}

fn merge_event(db: &mut Database, fs_event: FsEvent) {
    // info!(?fs_event, "new event:");
    if let Err(e) = db.merge_event(fs_event) {
        error!(?e, "merge event failed:");
    }
}
