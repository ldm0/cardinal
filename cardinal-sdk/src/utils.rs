use chrono::Utc;
use libc::dev_t;
use objc2_core_services::{FSEventsGetCurrentEventId, FSEventsGetLastEventIdForDeviceBeforeTime};
use std::collections::HashMap;

pub fn current_timestamp() -> i64 {
    Utc::now().timestamp()
}

pub fn current_event_id() -> u64 {
    unsafe { FSEventsGetCurrentEventId() }
}

pub fn last_event_id_before_time(dev: dev_t, timestamp: i64) -> u64 {
    unsafe { FSEventsGetLastEventIdForDeviceBeforeTime(dev, timestamp as f64) }
}

/// Given a device id, an event id, and a cache mapping timestamps to last event ids before them,
/// perform a binary search to find the timestamp corresponding to the event id.
pub fn event_id_to_timestamp(dev: dev_t, event_id: u64, cache: &mut HashMap<i64, u64>) -> i64 {
    let mut begin = 0i64;
    let mut end = current_timestamp();
    loop {
        let mid = (begin + end) / 2;
        if mid == begin || mid == end {
            return mid;
        }
        let mid_event_id = *cache.entry(mid).or_insert_with(||
            last_event_id_before_time(dev, mid)
        );
        if mid_event_id < event_id {
            begin = mid
        } else if mid_event_id > event_id {
            end = mid
        } else {
            return mid;
        }
    }
}
