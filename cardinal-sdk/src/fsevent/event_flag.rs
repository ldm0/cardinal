#![allow(non_upper_case_globals)]
use bitflags::bitflags;
bitflags! {
    pub struct MacEventFlag: u32 {
        const None = fsevent_sys::kFSEventStreamEventFlagNone;
        const MustScanSubDirs = fsevent_sys::kFSEventStreamEventFlagMustScanSubDirs;
        const UserDropped = fsevent_sys::kFSEventStreamEventFlagUserDropped;
        const KernelDropped = fsevent_sys::kFSEventStreamEventFlagKernelDropped;
        const EventIdsWrapped = fsevent_sys::kFSEventStreamEventFlagEventIdsWrapped;
        const HistoryDone = fsevent_sys::kFSEventStreamEventFlagHistoryDone;
        const RootChanged = fsevent_sys::kFSEventStreamEventFlagRootChanged;
        const Mount = fsevent_sys::kFSEventStreamEventFlagMount;
        const Unmount = fsevent_sys::kFSEventStreamEventFlagUnmount;
        const ItemCreated = fsevent_sys::kFSEventStreamEventFlagItemCreated;
        const ItemRemoved = fsevent_sys::kFSEventStreamEventFlagItemRemoved;
        const ItemInodeMetaMod = fsevent_sys::kFSEventStreamEventFlagItemInodeMetaMod;
        const ItemRenamed = fsevent_sys::kFSEventStreamEventFlagItemRenamed;
        const ItemModified = fsevent_sys::kFSEventStreamEventFlagItemModified;
        const ItemFinderInfoMod = fsevent_sys::kFSEventStreamEventFlagItemFinderInfoMod;
        const ItemChangeOwner = fsevent_sys::kFSEventStreamEventFlagItemChangeOwner;
        const ItemXattrMod = fsevent_sys::kFSEventStreamEventFlagItemXattrMod;
        const ItemIsFile = fsevent_sys::kFSEventStreamEventFlagItemIsFile;
        const ItemIsDir = fsevent_sys::kFSEventStreamEventFlagItemIsDir;
        const ItemIsSymlink = fsevent_sys::kFSEventStreamEventFlagItemIsSymlink;
        const OwnEvent = fsevent_sys::kFSEventStreamEventFlagOwnEvent;
        const IsHardlink = fsevent_sys::kFSEventStreamEventFlagItemIsHardlink;
        const IsLastHardlink = fsevent_sys::kFSEventStreamEventFlagItemIsLastHardlink;
        const Cloned = fsevent_sys::kFSEventStreamEventFlagItemCloned;
    }
}

pub enum EventType {
    Unknown,
    File,
    Dir,
    Symlink,
    Hardlink,
}

pub enum ScanType {
    SingleNode,
    Folder,
    /// Something wrong happened, do re-indexing.
    ReScan,
    /// Do nothing, since event id is always updated.
    Nop,
}

impl MacEventFlag {
    pub fn event_type(&self) -> EventType {
        if self.contains(MacEventFlag::IsHardlink)
            | self.contains(MacEventFlag::IsLastHardlink)
        {
            EventType::Hardlink
        } else if self.contains(MacEventFlag::ItemIsSymlink) {
            EventType::Symlink
        } else if self.contains(MacEventFlag::ItemIsDir) {
            EventType::Dir
        } else if self.contains(MacEventFlag::ItemIsFile) {
            EventType::File
        } else {
            EventType::Unknown
        }
    }

    pub fn scan_type(&self) -> ScanType {
        let event_type = self.event_type();
        let is_dir = matches!(event_type, EventType::Dir);
        if self.contains(MacEventFlag::None) {
            // Strange event, doesn't know when it happens, processing it using a generic way
            // e.g. new event: fs_event=FsEvent { path: "/.docid/16777229/changed/782/src=0,dst=41985052", flag: kFSEventStreamEventFlagNone, id: 471533015 }
            if is_dir {
                ScanType::Folder
            } else {
                ScanType::SingleNode
            }
        } else if self.contains(MacEventFlag::MustScanSubDirs)
            | self.contains(MacEventFlag::UserDropped)
            | self.contains(MacEventFlag::KernelDropped)
        {
            ScanType::ReScan
        } else if self.contains(MacEventFlag::EventIdsWrapped)
            | self.contains(MacEventFlag::HistoryDone)
        {
            ScanType::Nop
        } else if self.contains(MacEventFlag::RootChanged) {
            // Should never happen since we are watching "/"
            assert!(false);
            ScanType::ReScan
        } else if self.contains(MacEventFlag::Unmount)
            | self.contains(MacEventFlag::Mount)
        {
            assert!(is_dir);
            ScanType::Folder
        } else if self.contains(MacEventFlag::ItemCreated) {
            // creating dir is also single node
            ScanType::SingleNode
        } else if self.contains(MacEventFlag::ItemRemoved) {
            if is_dir {
                ScanType::Folder
            } else {
                ScanType::SingleNode
            }
        } else if self.contains(MacEventFlag::ItemInodeMetaMod) {
            // creating dir is also single node
            ScanType::SingleNode
        } else if self.contains(MacEventFlag::ItemRenamed) {
            if is_dir {
                ScanType::Folder
            } else {
                ScanType::SingleNode
            }
        } else if self.contains(MacEventFlag::ItemModified) {
            assert!(!is_dir);
            ScanType::SingleNode
        } else if self.contains(MacEventFlag::ItemFinderInfoMod)
            | self.contains(MacEventFlag::ItemChangeOwner)
            | self.contains(MacEventFlag::ItemXattrMod)
        {
            // creating dir is also single node
            ScanType::SingleNode
        } else if self.contains(MacEventFlag::OwnEvent) {
            unreachable!()
        } else if self.contains(MacEventFlag::Cloned) {
            if is_dir {
                ScanType::Folder
            } else {
                ScanType::SingleNode
            }
        } else {
            panic!("unexpected event: {:?}", self)
        }
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
        if f.contains(MacEventFlag::ItemCreated) {
            Ok(EventFlag::Create)
        } else if f.contains(MacEventFlag::ItemRemoved)
            | f.contains(MacEventFlag::Unmount)
        {
            Ok(EventFlag::Delete)
        } else if f.contains(MacEventFlag::ItemInodeMetaMod)
            | f.contains(MacEventFlag::ItemXattrMod)
            | f.contains(MacEventFlag::ItemChangeOwner)
            | f.contains(MacEventFlag::ItemFinderInfoMod)
            | f.contains(MacEventFlag::ItemModified)
            // Nowhere to distinguish it's 'from' or 'to'.
            | f.contains(MacEventFlag::ItemRenamed)
            // Nowhere to distinguish it's 'from' or 'to'.
            | f.contains(MacEventFlag::Cloned)
        {
            Ok(EventFlag::Modify)
        } else if f.contains(MacEventFlag::MustScanSubDirs)
            | f.contains(MacEventFlag::UserDropped)
            | f.contains(MacEventFlag::KernelDropped)
            | f.contains(MacEventFlag::EventIdsWrapped)
            // check the FSEvents.h it's implementation will be special
            | f.contains(MacEventFlag::Mount)
        {
            Err(f)
        } else if
        // we are watching root, so this will never happen.
        f.contains(MacEventFlag::RootChanged)
            // MarkSelf is not set on monitoring
            | f.contains(MacEventFlag::OwnEvent)
        {
            unreachable!()
        } else {
            Err(f)
        }
    }
}
