use crate::FsEvent;
use anyhow::{Result, bail};
use core_foundation::{array::CFArray, base::TCFType, date::CFTimeInterval, string::CFString};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use fsevent_sys::{
    FSEventStreamContext, FSEventStreamCreate, FSEventStreamEventFlags, FSEventStreamEventId,
    FSEventStreamInvalidate, FSEventStreamRef, FSEventStreamRelease, FSEventStreamSetDispatchQueue,
    FSEventStreamStart, FSEventStreamStop, kFSEventStreamCreateFlagFileEvents,
    kFSEventStreamCreateFlagNoDefer, kFSEventStreamCreateFlagWatchRoot,
};
use std::{ffi::c_void, ptr, slice};

type EventsCallback = Box<dyn FnMut(Vec<FsEvent>) + Send>;

pub struct EventStream {
    stream: FSEventStreamRef,
}

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
        extern "C" fn drop_callback(info: *mut c_void) {
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
                .map(|((&path, &flag), &id)| unsafe { FsEvent::from_raw(path, flag, id) })
                .collect();

            let callback = unsafe { (callback_info as *mut EventsCallback).as_mut() }.unwrap();
            callback(events);
        }

        let paths: Vec<_> = paths.iter().map(|&x| CFString::new(x)).collect();
        let paths = CFArray::from_CFTypes(&paths);
        let context = FSEventStreamContext {
            version: 0,
            info: Box::leak(Box::new(callback)) as *mut _ as *mut _,
            retain: None,
            release: Some(drop_callback),
            copy_description: None,
        };

        let stream: FSEventStreamRef = unsafe {
            FSEventStreamCreate(
                ptr::null_mut(),
                raw_callback,
                &context,
                paths.as_concrete_TypeRef() as _,
                since_event_id,
                latency,
                kFSEventStreamCreateFlagNoDefer
                    | kFSEventStreamCreateFlagFileEvents
                    | kFSEventStreamCreateFlagWatchRoot,
            )
        };
        Self { stream }
    }

    pub fn spawn(self) -> Result<EventStreamWithQueue> {
        let queue = DispatchQueue::new("cardinal-sdk-queue", DispatchQueueAttr::SERIAL);
        unsafe { FSEventStreamSetDispatchQueue(self.stream, &queue) };
        let result = unsafe { FSEventStreamStart(self.stream) };
        if result == 0 {
            // TODO(ldm0): RAII
            unsafe { FSEventStreamStop(self.stream) };
            unsafe { FSEventStreamInvalidate(self.stream) };
            bail!("fs event stream start failed.");
        }
        let stream = self.stream;
        std::mem::forget(self);
        Ok(EventStreamWithQueue { stream, queue })
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

    pub fn clear(&mut self) {
        let _ = std::mem::replace(self, Self::noop());
    }

    pub fn spawn(path: String, since_event_id: FSEventStreamEventId, latency: f64) -> EventWatcher {
        let (_cancellation_token, cancellation_token_rx) = bounded::<()>(1);
        let (sender, receiver) = unbounded();
        std::thread::spawn(move || {
            let _stream_and_queue = EventStream::new(
                &[&path],
                since_event_id,
                latency,
                Box::new(move |events| {
                    let _ = sender.send(events);
                }),
            )
            .spawn()
            .unwrap();
            let _ = cancellation_token_rx.recv();
        });
        EventWatcher {
            receiver,
            _cancellation_token,
        }
    }
}
