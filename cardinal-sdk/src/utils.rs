use chrono::Utc;
use fsevent_sys::{FSEventsGetCurrentEventId, FSEventsGetLastEventIdForDeviceBeforeTime};
use libc::dev_t;
use std::{cell::RefCell, collections::HashMap, ffi::CStr, mem::MaybeUninit};

pub fn current_timestamp() -> i64 {
    Utc::now().timestamp()
}

pub fn current_event_id() -> u64 {
    unsafe { FSEventsGetCurrentEventId() }
}

pub fn dev_of_path(path: &CStr) -> std::io::Result<dev_t> {
    let mut stat = MaybeUninit::uninit();
    let ret = unsafe { libc::lstat(path.as_ptr(), stat.as_mut_ptr()) };
    if ret != 0 {
        return Err(std::io::Error::from_raw_os_error(ret));
    }
    Ok(unsafe { stat.assume_init().st_dev })
}

pub fn last_event_id_before_time(dev: dev_t, timestamp: i64) -> u64 {
    // TODO(ldm0): Vec<dev_t, HashMap>, HashMap -> lru_cache
    thread_local! {
        static DEV: RefCell<Option<dev_t>> = const { RefCell::new(None) };
        static CACHE: RefCell<HashMap<i64, u64>> = RefCell::new(HashMap::new());
    }
    // Ensure that device is the same for the whole thread.
    DEV.with(|dev_cache| {
        let mut dev_cache = dev_cache.borrow_mut();
        if dev_cache.is_none() {
            *dev_cache = Some(dev)
        } else {
            assert_eq!(*dev_cache, Some(dev));
        }
    });
    // Return cached result if exists.
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(&event_id) = cache.get(&timestamp) {
            event_id
        } else {
            let event_id =
                unsafe { FSEventsGetLastEventIdForDeviceBeforeTime(dev, timestamp as f64) };
            cache.insert(timestamp, event_id);
            event_id
        }
    })
}

pub fn event_id_to_timestamp(dev: dev_t, event_id: u64) -> i64 {
    let mut begin = 0i64;
    let mut end = current_timestamp();
    loop {
        let mid = (begin + end) / 2;
        let mid_event_id = last_event_id_before_time(dev, mid);
        if mid == begin || mid == end {
            return mid;
        }
        if mid_event_id < event_id {
            begin = mid
        } else if mid_event_id > event_id {
            end = mid
        } else {
            return mid;
        }
    }
}
