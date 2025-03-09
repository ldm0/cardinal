use cardinal_sdk::database::Database;
use cardinal_sdk::fsevent::FsEvent;
use tracing::debug;
use tracing::error;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    // let _ = std::fs::remove_file(DATABASE_URL);
    let mut db = Database::from_fs().unwrap();
    let mut receiver = cardinal_sdk::fsevent::spawn_event_watcher(db.event_id);
    let (filter_tx, mut filter_rx) = tokio::sync::mpsc::unbounded_channel();
    let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let stdin = std::io::stdin();
        let mut filter = String::new();
        loop {
            stdin.read_line(&mut filter).unwrap();
            filter_tx
                .send(std::mem::take(&mut filter).trim().to_string())
                .unwrap();
            let result = result_rx.recv().await;
            dbg!(result);
        }
    });
    loop {
        tokio::select! {
            fs_events = receiver.recv() => {
                let fs_events = fs_events.unwrap();
                for fs_event in fs_events {
                    merge_event(&mut db, fs_event);
                }
            }
            filter = filter_rx.recv() => {
                let filter = filter.unwrap();
                match db.search(&filter) {
                    Ok(results) => result_tx.send(results).unwrap(),
                    Err(e) => error!(?e, "search failed:"),
                }
            }
        }
    }
}

fn merge_event(db: &mut Database, fs_event: FsEvent) {
    debug!(?fs_event, "new event:");
    if let Err(e) = db.merge_event(fs_event) {
        error!(?e, "merge event failed:");
    }
}
