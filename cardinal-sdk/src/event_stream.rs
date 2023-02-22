use crate::fsevent::FsEvent;
use anyhow::{bail, Result};
use core_foundation::base::TCFType;
use core_foundation::{array::CFArray, string::CFString};
use fsevent_sys::core_foundation::{kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRun};
use fsevent_sys::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer, FSEventStreamContext,
    FSEventStreamCreate, FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamRef,
    FSEventStreamScheduleWithRunLoop, FSEventStreamStart,
};
use std::ptr;
use std::{ffi::c_void, slice};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tracing::warn;

type EventsCallback = Box<dyn FnMut(Vec<FsEvent>) + Send>;

struct EventStream {
    stream: FSEventStreamRef,
}

impl EventStream {
    pub fn new(
        paths: Vec<String>,
        raw_event_id: FSEventStreamEventId,
        callback: EventsCallback,
    ) -> Self {
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
                raw_event_id,
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

pub fn spawn_event_watcher(raw_event_id: FSEventStreamEventId) -> UnboundedReceiver<FsEvent> {
    let (sender, receiver) = unbounded_channel();
    std::thread::spawn(move || {
        EventStream::new(
            vec!["/".into()],
            raw_event_id,
            Box::new(move |mut events| {
                // Fun fact, events here are not sorted by event id.
                events.sort_by_key(|x| x.id);
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
