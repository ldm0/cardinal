```
cbindgen --config cbindgen.toml --crate cardinal --output cardinal.h
```

// touch tmp.md:
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/tmp.md",
        flag: kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemIsFile,
        id: 281055642,
    },
]

// rm tmp.md
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/tmp.md",
        flag: kFSEventStreamEventFlagItemRemoved | kFSEventStreamEventFlagItemIsFile,
        id: 281056153,
    },
]

// mv tmp.md emm.md
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/tmp.md",
        flag: kFSEventStreamEventFlagItemRenamed | kFSEventStreamEventFlagItemIsFile,
        id: 281060640,
    },
    FsEvent {
        path: "/Users/bytedance/code/cardinal/emm.md",
        flag: kFSEventStreamEventFlagItemRenamed | kFSEventStreamEventFlagItemIsFile,
        id: 281060641,
    },
]

// chmod +x emm.md
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/emm.md",
        flag: kFSEventStreamEventFlagItemChangeOwner | kFSEventStreamEventFlagItemIsFile,
        id: 281063105,
    },
]

// Change README.md and save
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/README.md",
        flag: kFSEventStreamEventFlagItemInodeMetaMod | kFSEventStreamEventFlagItemModified | kFSEventStreamEventFlagItemIsFile,
        id: 286980245,
    },
]

// ln -s README.md readme
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/readme",
        flag: kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemIsSymlink,
        id: 286990858,
    },
]

// rm readme (symlink)
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/readme",
        flag: kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemRemoved | kFSEventStreamEventFlagItemIsSymlink,
        id: 286993883,
    },
]

// ln README.md readme
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/readme",
        flag: kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemIsFile | kFSEventStreamEventFlagItemIsHardlink,
        id: 286993132,
    },
    FsEvent {
        path: "/Users/bytedance/code/cardinal",
        flag: kFSEventStreamEventFlagItemInodeMetaMod | kFSEventStreamEventFlagItemIsDir,
        id: 286993135,
    },
]

// rm readme (hardlink)
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/readme",
        flag: kFSEventStreamEventFlagItemRemoved | kFSEventStreamEventFlagItemIsFile | kFSEventStreamEventFlagItemIsHardlink,
        id: 286993483,
    },
]






// mkdir target/tmp
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/target/tmp",
        flag: kFSEventStreamEventFlagItemCreated | kFSEventStreamEventFlagItemIsDir,
        id: 281055860,
    },
]

// rm -r target/tmp
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/target/tmp",
        flag: kFSEventStreamEventFlagItemRemoved | kFSEventStreamEventFlagItemIsDir,
        id: 281060345,
    },
]

// mv target/tmp target/emm
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/target/tmp",
        flag: kFSEventStreamEventFlagItemRenamed | kFSEventStreamEventFlagItemIsDir,
        id: 281061482,
    },
    FsEvent {
        path: "/Users/bytedance/code/cardinal/target/emm",
        flag: kFSEventStreamEventFlagItemRenamed | kFSEventStreamEventFlagItemIsDir,
        id: 281061483,
    },
]

// chmod 600 target/emm
[
    FsEvent {
        path: "/Users/bytedance/code/cardinal/target/emm",
        flag: kFSEventStreamEventFlagItemChangeOwner | kFSEventStreamEventFlagItemIsDir,
        id: 286155411,
    },
]
