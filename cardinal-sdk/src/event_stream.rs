use crate::FsEvent;
use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use libc::dev_t;
use objc2_core_foundation::{CFArray, CFString, CFTimeInterval};
use objc2_core_services::{
    ConstFSEventStreamRef, FSEventStreamContext, FSEventStreamCreate, FSEventStreamEventFlags,
    FSEventStreamEventId, FSEventStreamGetDeviceBeingWatched, FSEventStreamInvalidate,
    FSEventStreamRef, FSEventStreamRelease, FSEventStreamSetDispatchQueue, FSEventStreamStart,
    FSEventStreamStop, kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer,
    kFSEventStreamCreateFlagWatchRoot,
};
use std::{ffi::c_void, ptr::NonNull, slice};

type EventsCallback = Box<dyn FnMut(Vec<FsEvent>) + Send>;

pub struct EventStream {
    stream: FSEventStreamRef,
}

unsafe impl Send for EventStream {}

impl Drop for EventStream {
    fn drop(&mut self) {
        unsafe {
            FSEventStreamRelease(self.stream);
        }
    }
}

impl EventStream {
    pub fn new(
        paths: &[&str],
        since_event_id: FSEventStreamEventId,
        latency: CFTimeInterval,
        callback: EventsCallback,
    ) -> Self {
        unsafe extern "C-unwind" fn drop_callback(info: *const c_void) {
            let _cb: Box<EventsCallback> = unsafe { Box::from_raw(info as _) };
        }

        unsafe extern "C-unwind" fn raw_callback(
            _stream: ConstFSEventStreamRef, // ConstFSEventStreamRef streamRef
            callback_info: *mut c_void,     // void *clientCallBackInfo
            num_events: usize,              // size_t numEvents
            event_paths: NonNull<c_void>,   // void *eventPaths
            event_flags: NonNull<FSEventStreamEventFlags>, // const FSEventStreamEventFlags eventFlags[]
            event_ids: NonNull<FSEventStreamEventId>,      // const FSEventStreamEventId eventIds[]
        ) {
            let event_paths = unsafe {
                slice::from_raw_parts(event_paths.as_ptr() as *const *const i8, num_events)
            };
            let event_flags = unsafe { slice::from_raw_parts(event_flags.as_ptr(), num_events) };
            let event_ids = unsafe { slice::from_raw_parts(event_ids.as_ptr(), num_events) };
            let events: Vec<_> = event_paths
                .iter()
                .zip(event_flags)
                .zip(event_ids)
                .map(|((&path, &flag), &id)| unsafe { FsEvent::from_raw(path, flag, id) })
                .collect();

            let callback = unsafe { (callback_info as *mut EventsCallback).as_mut() }.unwrap();
            callback(events);
        }

        let paths: Vec<_> = paths.iter().map(|&x| CFString::from_str(x)).collect();
        let paths = CFArray::from_retained_objects(&paths);
        let mut context = FSEventStreamContext {
            version: 0,
            info: Box::leak(Box::new(callback)) as *mut _ as *mut _,
            retain: None,
            release: Some(drop_callback),
            copyDescription: None,
        };

        let stream: FSEventStreamRef = unsafe {
            FSEventStreamCreate(
                None,
                Some(raw_callback),
                &mut context,
                paths.as_opaque(),
                since_event_id,
                latency,
                kFSEventStreamCreateFlagNoDefer
                    | kFSEventStreamCreateFlagFileEvents
                    | kFSEventStreamCreateFlagWatchRoot,
            )
        };
        Self { stream }
    }

    // Start the FSEventStream with a dispatch queue.
    pub fn spawn(self) -> Result<EventStreamWithQueue> {
        let queue = DispatchQueue::new("cardinal-sdk-queue", DispatchQueueAttr::SERIAL);
        unsafe { FSEventStreamSetDispatchQueue(self.stream, Some(&queue)) };
        let result = unsafe { FSEventStreamStart(self.stream) };
        if !result {
            unsafe { FSEventStreamStop(self.stream) };
            unsafe { FSEventStreamInvalidate(self.stream) };
            bail!("fs event stream start failed.");
        }
        let stream = self.stream;
        Ok(EventStreamWithQueue { stream, queue })
    }

    // Get device id being watched by this event stream.
    pub fn dev(&self) -> dev_t {
        unsafe { FSEventStreamGetDeviceBeingWatched(self.stream.cast_const()) }
    }
}

/// FSEventStream with dispatch queue.
///
/// Dropping this struct will stop the FSEventStream and release the dispatch queue.
pub struct EventStreamWithQueue {
    stream: FSEventStreamRef,
    #[allow(dead_code)]
    queue: DispatchRetained<DispatchQueue>,
}

impl Drop for EventStreamWithQueue {
    fn drop(&mut self) {
        unsafe {
            FSEventStreamStop(self.stream);
            FSEventStreamInvalidate(self.stream);
            FSEventStreamRelease(self.stream);
        }
    }
}

pub struct EventWatcher {
    pub receiver: Receiver<Vec<FsEvent>>,
    _cancellation_token: Sender<()>,
}

impl EventWatcher {
    pub fn noop() -> Self {
        Self {
            receiver: unbounded().1,
            _cancellation_token: bounded::<()>(1).0,
        }
    }

    pub fn spawn(
        path: String,
        since_event_id: FSEventStreamEventId,
        latency: f64,
    ) -> (dev_t, EventWatcher) {
        let (_cancellation_token, cancellation_token_rx) = bounded::<()>(1);
        let (sender, receiver) = unbounded();
        let stream = EventStream::new(
            &[&path],
            since_event_id,
            latency,
            Box::new(move |events| {
                let _ = sender.send(events);
            }),
        );
        let dev = stream.dev();
        std::thread::Builder::new()
            .name("cardinal-sdk-event-watcher".to_string())
            .spawn(move || {
                let _stream_and_queue = stream.spawn().expect("failed to spawn event stream");
                let _ = cancellation_token_rx.recv();
            })
            .unwrap();
        (
            dev,
            EventWatcher {
                receiver,
                _cancellation_token,
            },
        )
    }
}
