#![allow(non_upper_case_globals)]
use bitflags::bitflags;
bitflags! {
    pub struct MacEventFlag: u32 {
        const kFSEventStreamEventFlagNone = fsevent_sys::kFSEventStreamEventFlagNone;
        const kFSEventStreamEventFlagMustScanSubDirs = fsevent_sys::kFSEventStreamEventFlagMustScanSubDirs;
        const kFSEventStreamEventFlagUserDropped = fsevent_sys::kFSEventStreamEventFlagUserDropped;
        const kFSEventStreamEventFlagKernelDropped = fsevent_sys::kFSEventStreamEventFlagKernelDropped;
        const kFSEventStreamEventFlagEventIdsWrapped = fsevent_sys::kFSEventStreamEventFlagEventIdsWrapped;
        const kFSEventStreamEventFlagHistoryDone = fsevent_sys::kFSEventStreamEventFlagHistoryDone;
        const kFSEventStreamEventFlagRootChanged = fsevent_sys::kFSEventStreamEventFlagRootChanged;
        const kFSEventStreamEventFlagMount = fsevent_sys::kFSEventStreamEventFlagMount;
        const kFSEventStreamEventFlagUnmount = fsevent_sys::kFSEventStreamEventFlagUnmount;
        const kFSEventStreamEventFlagItemCreated = fsevent_sys::kFSEventStreamEventFlagItemCreated;
        const kFSEventStreamEventFlagItemRemoved = fsevent_sys::kFSEventStreamEventFlagItemRemoved;
        const kFSEventStreamEventFlagItemInodeMetaMod = fsevent_sys::kFSEventStreamEventFlagItemInodeMetaMod;
        const kFSEventStreamEventFlagItemRenamed = fsevent_sys::kFSEventStreamEventFlagItemRenamed;
        const kFSEventStreamEventFlagItemModified = fsevent_sys::kFSEventStreamEventFlagItemModified;
        const kFSEventStreamEventFlagItemFinderInfoMod = fsevent_sys::kFSEventStreamEventFlagItemFinderInfoMod;
        const kFSEventStreamEventFlagItemChangeOwner = fsevent_sys::kFSEventStreamEventFlagItemChangeOwner;
        const kFSEventStreamEventFlagItemXattrMod = fsevent_sys::kFSEventStreamEventFlagItemXattrMod;
        const kFSEventStreamEventFlagItemIsFile = fsevent_sys::kFSEventStreamEventFlagItemIsFile;
        const kFSEventStreamEventFlagItemIsDir = fsevent_sys::kFSEventStreamEventFlagItemIsDir;
        const kFSEventStreamEventFlagItemIsSymlink = fsevent_sys::kFSEventStreamEventFlagItemIsSymlink;
        const kFSEventStreamEventFlagOwnEvent = fsevent_sys::kFSEventStreamEventFlagOwnEvent;
        const kFSEventStreamEventFlagItemIsHardlink = fsevent_sys::kFSEventStreamEventFlagItemIsHardlink;
        const kFSEventStreamEventFlagItemIsLastHardlink = fsevent_sys::kFSEventStreamEventFlagItemIsLastHardlink;
        const kFSEventStreamEventFlagItemCloned = fsevent_sys::kFSEventStreamEventFlagItemCloned;
    }
}

/// Abstract action of a file system event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventFlag {
    Create,
    Delete,
    Modify,
}

impl TryFrom<MacEventFlag> for EventFlag {
    type Error = MacEventFlag;
    fn try_from(f: MacEventFlag) -> Result<Self, MacEventFlag> {
        if f.contains(MacEventFlag::kFSEventStreamEventFlagItemCreated) {
            Ok(EventFlag::Create)
        } else if f.contains(MacEventFlag::kFSEventStreamEventFlagItemRemoved)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagUnmount)
        {
            Ok(EventFlag::Delete)
        } else if f.contains(MacEventFlag::kFSEventStreamEventFlagItemInodeMetaMod)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemXattrMod)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemChangeOwner)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemFinderInfoMod)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemModified)
            // Nowhere to distinguish it's 'from' or 'to'.
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemRenamed)
            // Nowhere to distinguish it's 'from' or 'to'.
            | f.contains(MacEventFlag::kFSEventStreamEventFlagItemCloned)
        {
            Ok(EventFlag::Modify)
        } else if f.contains(MacEventFlag::kFSEventStreamEventFlagMustScanSubDirs)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagUserDropped)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagKernelDropped)
            | f.contains(MacEventFlag::kFSEventStreamEventFlagEventIdsWrapped)
            // check the FSEvents.h it's implementation will be special
            | f.contains(MacEventFlag::kFSEventStreamEventFlagMount)
        {
            todo!("TODO: need to rescan specific directory: {:?}", f);
        } else if
        // we are watching root, so this will never happen.
        f.contains(MacEventFlag::kFSEventStreamEventFlagRootChanged)
            // MarkSelf is not set on monitoring
            | f.contains(MacEventFlag::kFSEventStreamEventFlagOwnEvent)
        {
            unreachable!()
        } else {
            Err(f)
        }
    }
}
