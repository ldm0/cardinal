use cardinal_sdk::{EventFlag, EventWatcher, event_id_to_timestamp};
use clap::Parser;
use std::time::Duration;

#[derive(Parser)]
struct Cli {
    /// Path to watch, default to current directory.
    path: Option<String>,
    /// Start event id, default to 0.
    #[clap(long, default_value_t = 0)]
    since: u64,
}

fn main() {
    let cli = Cli::parse();
    let path = cli.path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string()
    });
    let (dev, event_stream) = EventWatcher::spawn(path, cli.since, 0.1);
    let cache = &mut std::collections::HashMap::new();
    let mut history_done = false;
    let timezone = chrono::Local::now().timezone();
    loop {
        let events = if history_done {
            // If history is done, we try to drain the event stream with a timeout.
            event_stream
                .receiver
                .recv_timeout(Duration::from_secs_f32(0.5))
                .ok()
        } else {
            event_stream.receiver.recv().ok()
        };
        let Some(events) = events else {
            break;
        };
        for event in events {
            if event.flag.contains(EventFlag::HistoryDone) {
                history_done = true;
            }
            let timestamp = event_id_to_timestamp(dev, event.id, cache);
            let time = chrono::DateTime::from_timestamp(timestamp, 0)
                .unwrap()
                .with_timezone(&timezone);
            println!("{}, {}, {:?}, {:?}", time, event.id, event.path, event.flag);
        }
    }
}
