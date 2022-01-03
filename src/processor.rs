//! Platform independent fs event processor.
use crate::fsevent::FsEvent;
use anyhow::{bail, Context, Result};
use crossbeam::channel::{self, Receiver, Sender, TryRecvError, TrySendError};
use fsevent_sys::FSEventStreamEventId;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{collections::BTreeSet, path::PathBuf};

pub static PROCESSOR: OnceCell<Processor> = OnceCell::new();

pub struct Processor {
    /// Bounded fs events FIFO pipe for displaying.
    limited_fs_events: (Sender<FsEvent>, Receiver<FsEvent>),
    /// Raw fs events receiver channel from system.
    events_receiver: Receiver<Vec<FsEvent>>,
    /// The event id all the events begins with.
    since: FSEventStreamEventId,
    /// Paths of current file system.
    core_paths: Mutex<BTreeSet<PathBuf>>,
}

impl Processor {
    const FS_EVENTS_CHANNEL_LEN: usize = 1024;
    pub fn new(since: FSEventStreamEventId, events_receiver: Receiver<Vec<FsEvent>>) -> Self {
        let (sender, receiver) = channel::bounded(Self::FS_EVENTS_CHANNEL_LEN);
        Self {
            limited_fs_events: (sender, receiver),
            events_receiver,
            since,
            core_paths: Mutex::new(BTreeSet::new()),
        }
    }

    /// Non blocking move fs_event in. If filled, it will drop oldest fs event
    /// repeatedly until a fs_event is pushed.
    fn fill_fs_event(&self, event: FsEvent) -> Result<()> {
        let mut event = Some(event);
        loop {
            match self.limited_fs_events.0.try_send(event.take().unwrap()) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => bail!("fs events channel closed!"),
                Err(TrySendError::Full(give_back)) => {
                    match self.limited_fs_events.1.try_recv() {
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

    fn take_fs_events(&self) -> Vec<FsEvent> {
        // Due to non atomic channel recv, double the size of possible receiving vec.
        let max_take_num = 2 * self.limited_fs_events.0.len();
        let mut fs_events = Vec::with_capacity(max_take_num);
        while let Ok(event) = self.limited_fs_events.1.try_recv() {
            if fs_events.len() >= max_take_num {
                break;
            }
            fs_events.push(event);
        }
        fs_events
    }

    pub fn process(&self) -> Result<()> {
        let events = self
            .events_receiver
            .recv()
            .context("System events channel closed.")?;
        for event in events {
            self.core_paths.lock().insert(event.path.clone());
            // Provide raw fs event.
            self.fill_fs_event(event).context("fill fs event failed.")?;
        }
        Ok(())
    }
}

/// Get raw fs events from processor. Capacity is limited due to the memory
/// pressure. So only the first few events are provided.
pub fn take_fs_events() -> Vec<FsEvent> {
    PROCESSOR
        .get()
        .map(|x| x.take_fs_events())
        .unwrap_or_default()
}
