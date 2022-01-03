#![deny(unsafe_op_in_unsafe_fn)]
mod c;
mod fsevent;
mod fsevent_flags;
mod fsevent_pb;
mod processor;
mod runtime;

pub use c::*;
use fsevent::FsEvent;
pub use processor::take_fs_events;

use anyhow::{bail, Result};
use core_foundation::{
    array::CFArray,
    base::TCFType,
    runloop::{kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRun},
    string::CFString,
};
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer, FSEventStreamContext,
    FSEventStreamCreate, FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamRef,
    FSEventStreamScheduleWithRunLoop, FSEventStreamStart, FSEventsGetCurrentEventId,
};
use runtime::runtime;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use std::{ffi::c_void, ptr, slice};

type EventsCallback = Box<dyn FnMut(Vec<FsEvent>) + Send>;

extern "C" fn raw_callback(
    _stream: FSEventStreamRef,  // ConstFSEventStreamRef streamRef
    callback_info: *mut c_void, // void *clientCallBackInfo
    num_events: usize,          // size_t numEvents
    event_paths: *mut c_void,   // void *eventPaths
    event_flags: *const FSEventStreamEventFlags, // const FSEventStreamEventFlags eventFlags[]
    event_ids: *const FSEventStreamEventId, // const FSEventStreamEventId eventIds[]
) {
    let event_paths = unsafe { slice::from_raw_parts(event_paths as *const *const i8, num_events) };
    let event_flags =
        unsafe { slice::from_raw_parts(event_flags as *const FSEventStreamEventFlags, num_events) };
    let event_ids =
        unsafe { slice::from_raw_parts(event_ids as *const FSEventStreamEventId, num_events) };
    let events: Vec<_> = event_paths
        .iter()
        .zip(event_flags)
        .zip(event_ids)
        .map(|((&path, &flag), &id)| FsEvent::from_raw(path, flag, id))
        .collect();

    let callback = unsafe { (callback_info as *mut EventsCallback).as_mut() }.unwrap();
    callback(events);
}

fn watch_fs_events(
    paths: Vec<String>,
    since: FSEventStreamEventId,
    callback: EventsCallback,
) -> Result<()> {
    extern "C" fn drop_callback(info: *const c_void) {
        let _cb: Box<EventsCallback> = unsafe { Box::from_raw(info as _) };
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
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe { FSEventStreamScheduleWithRunLoop(stream, run_loop as _, kCFRunLoopDefaultMode as _) };
    let result = unsafe { FSEventStreamStart(stream) };
    if result == 0 {
        bail!("fs event stream start failed.");
    }
    unsafe { CFRunLoopRun() };
    Ok(())
}

fn spawn_watcher(since: FSEventStreamEventId) -> UnboundedReceiver<Vec<FsEvent>> {
    let (sender, receiver) = mpsc::unbounded_channel();
    runtime().spawn_blocking(move || {
        watch_fs_events(
            vec!["/".into()],
            since,
            Box::new(move |events| {
                sender.send(events).unwrap();
            }),
        )
        .unwrap();
    });
    receiver
}

fn spawn_processor(since: FSEventStreamEventId, receiver: UnboundedReceiver<Vec<FsEvent>>) {
    runtime().spawn(processor::processor(since, receiver));
}

pub fn init_sdk() {
    let since = unsafe { FSEventsGetCurrentEventId() };
    let receiver = spawn_watcher(since);
    spawn_processor(since, receiver);
}
