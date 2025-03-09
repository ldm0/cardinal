//! Platform independent fs event processor.
use crate::consts::{self};
use crate::database::{Database, PartialDatabase};
use crate::fsevent::FsEvent;
use anyhow::{Context, Result, bail};
use crossbeam::channel::{self, Receiver, Sender, TryRecvError, TrySendError};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::path::Path;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;

/// The global event processor.
pub static PROCESSOR: Processor = Processor;
/// Bounded fs events FIFO pipe for displaying.
pub static LIMITED_FS_EVENTS: Lazy<(Sender<FsEvent>, Receiver<FsEvent>)> =
    Lazy::new(|| channel::bounded(Processor::FS_EVENTS_CHANNEL_LEN));
/// File system Database .
///
/// It's initialized before event processing.
/// It's dropped on application closed.
pub static DATABASE: Lazy<Mutex<Option<Database>>> = Lazy::new(|| Mutex::new(None));

pub struct Processor;

impl Processor {
    const FS_EVENTS_CHANNEL_LEN: usize = 1024;
    /// Non blocking move fs_event in. If filled, it will drop oldest fs event
    /// repeatedly until a fs_event is pushed.
    fn fill_fs_event(&self, event: FsEvent) -> Result<()> {
        let mut event = Some(event);
        loop {
            match LIMITED_FS_EVENTS.0.try_send(event.take().unwrap()) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => bail!("fs events channel closed!"),
                Err(TrySendError::Full(give_back)) => {
                    match LIMITED_FS_EVENTS.1.try_recv() {
                        Ok(x) => drop(x),
                        Err(TryRecvError::Disconnected) => bail!("fs events channel disconnected"),
                        Err(TryRecvError::Empty) => {}
                    };
                    event = Some(give_back);
                }
            }
        }
        Ok(())
    }

    /// Take out fs_event cache of current processor.
    fn take_fs_events(&self) -> Vec<FsEvent> {
        // Due to non atomic channel recv, double the size of possible receiving vec.
        let max_take_num = 2 * LIMITED_FS_EVENTS.0.len();
        let mut fs_events = Vec::with_capacity(max_take_num);
        while let Ok(event) = LIMITED_FS_EVENTS.1.try_recv() {
            if fs_events.len() >= max_take_num {
                break;
            }
            fs_events.push(event);
        }
        fs_events
    }

    /// Non-blocking process a event.
    pub async fn process_event(
        &self,
        events_receiver: &mut UnboundedReceiver<Vec<FsEvent>>,
    ) -> Result<()> {
        let events = events_receiver
            .recv()
            .await
            .context("System events channel closed.")?;
        for event in events {
            self.on_event(event).context("process fs event failed.")?;
        }
        Ok(())
    }

    /// On new fs event.
    fn on_event(&self, event: FsEvent) -> Result<()> {
        info!(FSEvent = ?event);
        DATABASE
            .lock()
            .as_mut()
            .context("Fs database closed")?
            .merge(&event);
        // Provide raw fs event.
        self.fill_fs_event(event).context("fill fs event failed.")?;
        Ok(())
    }

    pub async fn get_db_from_fs(
        &self,
        events_receiver: &mut UnboundedReceiver<Vec<FsEvent>>,
    ) -> Result<Database> {
        info!("fs scanning starts.");
        let mut partial_db = PartialDatabase::scan_fs();
        info!("fs scanning completes.");
        while let Ok(events) = events_receiver.try_recv() {
            for event in events {
                partial_db.merge(&event);
            }
        }
        info!("Database construction completes.");
        let db = partial_db.complete_merge();
        Ok(db)
    }

    pub async fn block_on(
        &self,
        database: Option<Database>,
        events_receiver: &mut UnboundedReceiver<Vec<FsEvent>>,
    ) -> Result<()> {
        let database = if let Some(x) = database {
            x
        } else {
            self.get_db_from_fs(events_receiver)
                .await
                .context("Get db failed.")?
        };
        *DATABASE.lock() = Some(database);
        loop {
            self.process_event(events_receiver)
                .await
                .context("processor is down.")?;
        }
    }

    pub fn close(&self) -> Result<()> {
        // Save and drop the database
        let database = DATABASE.lock().take().context("Close uninit processor.")?;
        info!("Start saving database");
        database
            .into_fs(Path::new(consts::DB_PATH))
            .context("Save database failed.")?;
        Ok(())
    }
}

/// Get raw fs events from global processor. Capacity is limited due to the
/// memory pressure, so only the first few(currently 1024) events will be provided.
pub fn take_fs_events() -> Vec<FsEvent> {
    PROCESSOR.take_fs_events()
}
