use crate::utils;
use bincode::Decode;
use bincode::Encode;
use fsevent_sys::FSEventStreamEventId;
use serde::Deserialize;
use serde::Serialize;

/// A event id for event ordering.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialOrd,
    PartialEq,
    Eq,
    Ord,
    Decode,
    Encode,
    Serialize,
    Deserialize,
)]
pub struct EventId {
    pub raw_event_id: FSEventStreamEventId,
    pub timestamp: i64,
}

impl EventId {
    // Return current event id and timestamp.
    pub fn now() -> Self {
        let raw_event_id = utils::current_event_id();
        let timestamp = utils::current_timestamp();
        Self {
            raw_event_id,
            timestamp,
        }
    }

    pub fn now_with_id(raw_event_id: u64) -> Self {
        let timestamp = utils::current_timestamp();
        Self {
            raw_event_id,
            timestamp,
        }
    }
}
