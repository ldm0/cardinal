mod event_flag;
mod event_id;

pub use event_flag::EventFlag;
pub use event_flag::MacEventFlag;
pub use event_id::EventId;

use anyhow::{anyhow, Context, Result};
use fsevent_sys::FSEventStreamEventId;

use std::{
    ffi::{CStr, OsStr},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

#[derive(Debug)]
pub struct FsEvent {
    /// The path of this event.
    pub path: PathBuf,
    /// The event type.
    pub flag: EventFlag,
    /// The event id.
    pub id: FSEventStreamEventId,
}

impl FsEvent {
    pub(crate) fn from_raw(path: *const i8, flag: u32, id: u64) -> Result<Self> {
        let path = unsafe { CStr::from_ptr(path) };
        let path = OsStr::from_bytes(path.to_bytes());
        let path = PathBuf::from(path);
        let flag = MacEventFlag::from_bits_truncate(flag);
        let flag = flag.try_into().map_err(|x| {
            anyhow!(
                "convert mac event flag to abstract event flag failed: {:?}",
                x
            )
        })?;
        Ok(FsEvent { path, flag, id })
    }
}
