#![deny(unsafe_op_in_unsafe_fn)]
mod c;
mod consts;
mod database;
pub mod fs_entry;
mod fsevent;
mod processor;
mod runtime;
mod utils;

pub use c::*;
use fsevent::EventId;
use fsevent::FsEvent;
pub use processor::take_fs_events;
use processor::Processor;

use anyhow::{anyhow, bail, Context, Result};
use core_foundation::{
    array::CFArray,
    base::TCFType,
    runloop::{kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRun},
    string::CFString,
};
use crossbeam::channel::{self, Receiver};
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer, FSEventStreamContext,
    FSEventStreamCreate, FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamRef,
    FSEventStreamScheduleWithRunLoop, FSEventStreamStart,
};
use runtime::runtime;
use tracing::{error, warn};

use std::{ffi::c_void, ptr, slice};

type EventsCallback = Box<dyn FnMut(Vec<FsEvent>) + Send>;

struct EventStream {
    stream: FSEventStreamRef,
}

impl EventStream {
    pub fn new(paths: Vec<String>, since: FSEventStreamEventId, callback: EventsCallback) -> Self {
        extern "C" fn drop_callback(info: *const c_void) {
            let _cb: Box<EventsCallback> = unsafe { Box::from_raw(info as _) };
        }

        extern "C" fn raw_callback(
            _stream: FSEventStreamRef,  // ConstFSEventStreamRef streamRef
            callback_info: *mut c_void, // void *clientCallBackInfo
            num_events: usize,          // size_t numEvents
            event_paths: *mut c_void,   // void *eventPaths
            event_flags: *const FSEventStreamEventFlags, // const FSEventStreamEventFlags eventFlags[]
            event_ids: *const FSEventStreamEventId,      // const FSEventStreamEventId eventIds[]
        ) {
            let event_paths =
                unsafe { slice::from_raw_parts(event_paths as *const *const i8, num_events) };
            let event_flags = unsafe {
                slice::from_raw_parts(event_flags as *const FSEventStreamEventFlags, num_events)
            };
            let event_ids = unsafe {
                slice::from_raw_parts(event_ids as *const FSEventStreamEventId, num_events)
            };
            let events: Vec<_> = event_paths
                .iter()
                .zip(event_flags)
                .zip(event_ids)
                .filter_map(
                    |((&path, &flag), &id)| match FsEvent::from_raw(path, flag, id) {
                        Ok(x) => Some(x),
                        Err(error) => {
                            warn!(?error, "bad fs event:");
                            None
                        }
                    },
                )
                .collect();

            let callback = unsafe { (callback_info as *mut EventsCallback).as_mut() }.unwrap();
            callback(events);
        }

        let paths: Vec<_> = paths.into_iter().map(|x| CFString::new(&x)).collect();
        let paths = CFArray::from_CFTypes(&paths);
        let context = Box::leak(Box::new(FSEventStreamContext {
            version: 0,
            info: Box::leak(Box::new(callback)) as *mut _ as _,
            retain: None,
            release: Some(drop_callback),
            copy_description: None,
        }));

        let stream: FSEventStreamRef = unsafe {
            FSEventStreamCreate(
                ptr::null_mut(),
                raw_callback,
                context,
                paths.as_concrete_TypeRef() as _,
                since,
                0.1,
                kFSEventStreamCreateFlagNoDefer | kFSEventStreamCreateFlagFileEvents,
            )
        };
        Self { stream }
    }

    fn block_watch(self) -> Result<()> {
        let run_loop = unsafe { CFRunLoopGetCurrent() };
        unsafe {
            FSEventStreamScheduleWithRunLoop(self.stream, run_loop as _, kCFRunLoopDefaultMode as _)
        };
        let result = unsafe { FSEventStreamStart(self.stream) };
        if result == 0 {
            bail!("fs event stream start failed.");
        }
        unsafe { CFRunLoopRun() };
        Ok(())
    }
}

fn spawn_event_watcher(since: FSEventStreamEventId) -> Receiver<FsEvent> {
    let (sender, receiver) = channel::unbounded();
    runtime().spawn_blocking(move || {
        EventStream::new(
            vec!["/".into()],
            since,
            Box::new(move |events| {
                for event in events {
                    sender.send(event).unwrap();
                }
            }),
        )
        .block_watch()
        .unwrap();
    });
    receiver
}

fn spawn_event_processor(event_id: EventId, receiver: Receiver<FsEvent>) -> Result<()> {
    processor::PROCESSOR
        .set(Processor::new(event_id, receiver))
        .map_err(|_| anyhow!("Multiple initialization"))?;
    // unwrap is legal here since processor is always init.
    runtime().spawn_blocking(|| processor::PROCESSOR.get().unwrap().block_on());
    Ok(())
}

pub fn close_event_processor() -> Result<()> {
    processor::PROCESSOR
        .get()
        .context("Global processor is not present")?
        .close()
        .context("Close global processor failed.")?;
    Ok(())
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
    let event_id = EventId::now();
    // A global event watcher spawned on a dedicated thread.
    let receiver = spawn_event_watcher(event_id.since);
    // A global event processor spawned on a dedicated thread.
    spawn_event_processor(event_id, receiver).context("spawn event processor failed")?;
    Ok(())
}

fn close_sdk() -> Result<()> {
    close_event_processor().context("close event processor failed")?;
    Ok(())
}
