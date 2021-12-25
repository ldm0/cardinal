use crate::fsevent::FsEvent;
use anyhow::{bail, Result};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{BTreeSet, VecDeque};
use tokio::sync::mpsc::{
    self,
    error::{TryRecvError, TrySendError},
    Receiver, Sender, UnboundedReceiver, UnboundedSender,
};

const FS_EVENTS_CHANNEL_LEN: usize = 1024;
static FS_EVENTS_CHANNEL: Lazy<(Sender<FsEvent>, Mutex<Receiver<FsEvent>>)> = Lazy::new(|| {
    let (sender, receiver) = mpsc::channel(FS_EVENTS_CHANNEL_LEN);
    (sender, Mutex::new(receiver))
});

/// Non blocking move fs_event in. If filled, it will drop oldest fs event repeatedly until a fs_event is pushed.
fn fill_fs_event(event: FsEvent) -> Result<()> {
    let permit = loop {
        match FS_EVENTS_CHANNEL.0.try_reserve() {
            Ok(x) => break x,
            Err(TrySendError::Closed(_)) => bail!("fs events channel closed!"),
            Err(TrySendError::Full(_)) => {
                match FS_EVENTS_CHANNEL.1.lock().try_recv() {
                    Ok(x) => drop(x),
                    Err(TryRecvError::Disconnected) => bail!("fs events channel disconnected"),
                    Err(TryRecvError::Empty) => {}
                };
            }
        }
    };
    permit.send(event);
    Ok(())
}

pub fn take_fs_events() -> Vec<FsEvent> {
    let current_num = FS_EVENTS_CHANNEL_LEN - FS_EVENTS_CHANNEL.0.capacity();
    // Due to non atomic channel recv, double the size of possible receiving vec.
    let max_take_num = 2 * current_num;
    let mut fs_events = Vec::with_capacity(max_take_num);
    while let Ok(event) = FS_EVENTS_CHANNEL.1.lock().try_recv() {
        if fs_events.len() >= max_take_num {
            break;
        }
        fs_events.push(event);
    }
    fs_events
}

pub async fn processor(mut receiver: UnboundedReceiver<Vec<FsEvent>>) -> Result<()> {
    let mut core_paths = BTreeSet::new();
    for events in receiver.recv().await {
        for event in events {
            core_paths.insert(event.path.clone());
            fill_fs_event(event)?;
        }
    }
    Ok(())
}
