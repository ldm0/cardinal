use bitflags::bitflags;
bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EventFlag: u32 {
        const None = objc2_core_services::kFSEventStreamEventFlagNone;
        const MustScanSubDirs = objc2_core_services::kFSEventStreamEventFlagMustScanSubDirs;
        const UserDropped = objc2_core_services::kFSEventStreamEventFlagUserDropped;
        const KernelDropped = objc2_core_services::kFSEventStreamEventFlagKernelDropped;
        const EventIdsWrapped = objc2_core_services::kFSEventStreamEventFlagEventIdsWrapped;
        const HistoryDone = objc2_core_services::kFSEventStreamEventFlagHistoryDone;
        const RootChanged = objc2_core_services::kFSEventStreamEventFlagRootChanged;
        const Mount = objc2_core_services::kFSEventStreamEventFlagMount;
        const Unmount = objc2_core_services::kFSEventStreamEventFlagUnmount;
        const ItemCreated = objc2_core_services::kFSEventStreamEventFlagItemCreated;
        const ItemRemoved = objc2_core_services::kFSEventStreamEventFlagItemRemoved;
        const ItemInodeMetaMod = objc2_core_services::kFSEventStreamEventFlagItemInodeMetaMod;
        const ItemRenamed = objc2_core_services::kFSEventStreamEventFlagItemRenamed;
        const ItemModified = objc2_core_services::kFSEventStreamEventFlagItemModified;
        const ItemFinderInfoMod = objc2_core_services::kFSEventStreamEventFlagItemFinderInfoMod;
        const ItemChangeOwner = objc2_core_services::kFSEventStreamEventFlagItemChangeOwner;
        const ItemXattrMod = objc2_core_services::kFSEventStreamEventFlagItemXattrMod;
        const ItemIsFile = objc2_core_services::kFSEventStreamEventFlagItemIsFile;
        const ItemIsDir = objc2_core_services::kFSEventStreamEventFlagItemIsDir;
        const ItemIsSymlink = objc2_core_services::kFSEventStreamEventFlagItemIsSymlink;
        const OwnEvent = objc2_core_services::kFSEventStreamEventFlagOwnEvent;
        const IsHardlink = objc2_core_services::kFSEventStreamEventFlagItemIsHardlink;
        const IsLastHardlink = objc2_core_services::kFSEventStreamEventFlagItemIsLastHardlink;
        const Cloned = objc2_core_services::kFSEventStreamEventFlagItemCloned;
    }
}

pub enum EventType {
    Unknown,
    File,
    Dir,
    Symlink,
    Hardlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanType {
    /// Scan a single node
    SingleNode,
    /// Scan the whole folder, including sub-folders.
    Folder,
    /// Something wrong happened, do re-indexing.
    /// Should only happen with `kFSEventStreamCreateFlagWatchRoot` set in EventStream::new().
    ReScan,
    /// Do nothing, since event id is always updated.
    Nop,
}

impl EventFlag {
    pub fn event_type(&self) -> EventType {
        if self.contains(EventFlag::IsHardlink) | self.contains(EventFlag::IsLastHardlink) {
            EventType::Hardlink
        } else if self.contains(EventFlag::ItemIsSymlink) {
            EventType::Symlink
        } else if self.contains(EventFlag::ItemIsDir) {
            EventType::Dir
        } else if self.contains(EventFlag::ItemIsFile) {
            EventType::File
        } else {
            EventType::Unknown
        }
    }

    pub fn scan_type(&self) -> ScanType {
        let event_type = self.event_type();
        let is_dir = matches!(event_type, EventType::Dir);
        if self.contains(EventFlag::HistoryDone) | self.contains(EventFlag::EventIdsWrapped) {
            ScanType::Nop
        } else if self.contains(EventFlag::RootChanged) {
            ScanType::ReScan
        } else {
            // Strange event, doesn't know when it happens, processing it using a generic way
            // e.g. new event: fs_event=FsEvent { path: "/.docid/16777229/changed/782/src=0,dst=41985052", flag: kFSEventStreamEventFlagNone, id: 471533015 }
            if is_dir {
                ScanType::Folder
            } else {
                ScanType::SingleNode
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_deduction() {
        assert!(matches!(
            (EventFlag::ItemIsFile).event_type(),
            EventType::File
        ));
        assert!(matches!(
            (EventFlag::ItemIsDir).event_type(),
            EventType::Dir
        ));
        assert!(matches!(
            (EventFlag::ItemIsSymlink).event_type(),
            EventType::Symlink
        ));
        assert!(matches!(
            (EventFlag::IsHardlink).event_type(),
            EventType::Hardlink
        ));
        // Unknown when no type bits set
        assert!(matches!((EventFlag::None).event_type(), EventType::Unknown));
    }

    #[test]
    fn test_scan_type_root_changed_and_history_done() {
        // RootChanged should always trigger ReScan regardless of type bits; test with RootChanged only.
        assert_eq!(EventFlag::RootChanged.scan_type(), ScanType::ReScan);
        assert_eq!(EventFlag::HistoryDone.scan_type(), ScanType::Nop);
    }

    #[test]
    fn test_scan_type_created_removed_modified() {
        // File create => SingleNode
        assert!(matches!(
            (EventFlag::ItemCreated | EventFlag::ItemIsFile).scan_type(),
            ScanType::SingleNode
        ));
        // Dir removal => Folder
        assert!(matches!(
            (EventFlag::ItemRemoved | EventFlag::ItemIsDir).scan_type(),
            ScanType::Folder
        ));
        // File removal => SingleNode
        assert!(matches!(
            (EventFlag::ItemRemoved | EventFlag::ItemIsFile).scan_type(),
            ScanType::SingleNode
        ));
        // File modified => SingleNode
        assert!(matches!(
            (EventFlag::ItemModified | EventFlag::ItemIsFile).scan_type(),
            ScanType::SingleNode
        ));
    }

    #[test]
    fn test_scan_type_must_scan_subdirs() {
        // MustScanSubDirs => Folder
        assert!(matches!(
            (EventFlag::MustScanSubDirs | EventFlag::ItemIsDir).scan_type(),
            ScanType::Folder
        ));
    }
}
