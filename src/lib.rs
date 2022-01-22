#![deny(unsafe_op_in_unsafe_fn)]
mod c;
pub mod fs_entry;
mod fsevent;
mod fsevent_flags;
mod fsevent_pb;
mod processor;
mod runtime;

pub use c::*;
use fsevent::FsEvent;
pub use processor::take_fs_events;
use processor::Processor;

use anyhow::{bail, Result};
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
    FSEventStreamScheduleWithRunLoop, FSEventStreamStart, FSEventsGetCurrentEventId,
};
use runtime::runtime;

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
                .map(|((&path, &flag), &id)| FsEvent::from_raw(path, flag, id))
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

    fn watch(self) -> Result<()> {
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

struct EventId {
    since: u64,
    timestamp: i64,
}

impl EventId {
    // Return latest event id and timestamp.
    fn now() -> Self {
        let since = unsafe { FSEventsGetCurrentEventId() };
        let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
        Self { since, timestamp }
    }
}

fn spawn_watcher(since: FSEventStreamEventId) -> Receiver<FsEvent> {
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
        .watch()
        .unwrap();
    });
    receiver
}

fn spawn_processor(since: FSEventStreamEventId, receiver: Receiver<FsEvent>) {
    if let Err(_) = processor::PROCESSOR.set(Processor::new(since, receiver)) {
        panic!("Multiple initialization");
    }
    runtime().spawn_blocking(|| loop {
        if let Err(e) = processor::PROCESSOR.get().unwrap().process() {
            panic!("processor is down. {}", e);
        }
    });
}

pub fn init_sdk() {
    let event_id = EventId::now();
    let receiver = spawn_watcher(event_id.since);
    spawn_processor(event_id.since, receiver);
}
