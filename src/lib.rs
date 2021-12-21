#![deny(unsafe_op_in_unsafe_fn)]
mod fsevent;
mod fsevent_flags;
mod pb;

use fsevent::FsEvent;

use anyhow::{bail, Result};
use core_foundation::{
    array::CFArray,
    base::TCFType,
    runloop::{kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRun},
    string::CFString,
};
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer,
    kFSEventStreamEventIdSinceNow, FSEventStreamContext, FSEventStreamCreate,
    FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamRef,
    FSEventStreamScheduleWithRunLoop, FSEventStreamStart,
};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::{runtime, sync::mpsc, task};

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

fn listen(paths: Vec<String>, callback: EventsCallback) -> Result<()> {
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
            kFSEventStreamEventIdSinceNow,
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

async fn init_worker() -> Result<()> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let paths = vec!["/".into()];
    task::spawn_blocking(move || {
        let callback = Box::new(move |events| {
            sender.send(events).unwrap();
        });
        listen(paths, callback).unwrap();
    });
    while let Some(events) = receiver.recv().await {
        EVENTS_LIST.lock().extend(events.into_iter());
    }
    Ok(())
}

static RUNTIME: Lazy<runtime::Runtime> = Lazy::new(|| {
    runtime::Builder::new_multi_thread()
        .thread_name("cardinal")
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
});

static EVENTS_LIST: Mutex<Vec<FsEvent>> = parking_lot::const_mutex(Vec::new());

#[no_mangle]
pub extern "C" fn init_sdk() {
    RUNTIME.spawn(init_worker());
}

#[no_mangle]
pub extern "C" fn get_events(
    context: *const i8,
    callback: Option<unsafe extern "C" fn(*const i8, *const i8)>,
) {
    let bytes = fsevent::write_events_to_bytes(&EVENTS_LIST.lock());
    callback.map(|c| unsafe { c(context, bytes.as_ptr() as _) });
}
