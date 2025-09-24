# PENDING
- fs-icon 仍差强人意，目前显示的和 finder 仍然不一致，怀疑Finder用的别的预览 API(QLThumbnailGenerator?)
    + 改成 QLThumbnailGenerator + NSWorkspace 之后仍然和Finder实现不一样，尽力了尽力了
- 需要在内容没有返回时阻塞滚动条
    - 标准的 scroll bar 没有实现这个，只能用假 scroll bar 了 https://github.com/yairEO/fakescroll/blob/master/react.fakescroll.js
    - AI 写半天写不好，先缓缓
- 搜索结果path中query部分高亮(优化成Rust实现返回matched part)
    - 不高优
- metadata 启动后自动 fetching
    - 目前 metadata replenish 流程不太明朗，先搁置
- metadata 索引
    - ctime, mtime, size
    - 目前 metadata 启动后自动 fetching 流程不太明朗(需要更细粒度的锁)，先搁置
- 内存优化
    + APFS 每个目录下面最多只能有 21 亿个文件 https://superuser.com/questions/845143/any-limitation-for-having-many-files-in-a-directory-in-macos
        + Vec with u32 capacity + length
            + capacity + length 都用 u32 表示 24 byte -> 16 byte
        + 最后使用了 ThinVec，反正大多数Node都是文件，ThinVec::new() 也不会产生额外的卫星内存
            + 24 byte -> 8 byte
- Windows/NTFS 支持

# TODO
- 文件空格预览支持
- 支持普通搜索，正则搜索, glob search
    - 不同的格式有开关按钮，类似于 vscode
    - 在输入不同的内容的时候自动推断是 glob 还是正则还是普通 substr 搜索，然后对应的按钮变成浅色
        - 用户可以手动勾选对应的模式按钮，变成深色
- 搜索结果自动更新
    - FSEvent 更新之后要重新拉取搜索结果
- 搜索结果排序
- native 内存占用高(mmap?)
    - namepool 和索引内存共享
+ shareded-slab + parking_lot::Mutex(+1 byte，内存体积友好)
    + 问题在于并行读写的正确处理，如 parent 消失场景
        + fsevent 改 slab 结构， metadata fetching 只增添 metadata 不改 slab 结构
+ namepool 里面想要去重就用一个附属的hashset就行了（fnv hashset 不存储内容，只有hash）
    + 只 insert 去重，不 remove 不覆盖，否则之前的 index 会失效（坏处就是如果一直有不重名的文件创建，name pool 会越来越大
+ 加一个页面放 fsevent 列表
    + 知道文件系统有哪些更新的需求
+ 加一个页面搜索一个文件夹曾经有过哪些文件
    + 误删除确认需求
        - 目前 fsevent 处理慢:
            - 并行化 fsevent 的fs遍历部分，写数据部分抢锁
            - 只要有足够快速的 fsevent 处理，我们就可以做到搜索曾经曾经存在过的文件
            - 好像速度也能接受
+ 加一个页面渲染文件体积 treemap, 支持钻取（点击文件夹展开子项）。
    + 磁盘整理需求
+ 申请 macos 全盘访问权限: https://github.com/ayangweb/tauri-plugin-macos-permissions
+ 内存优化:
    + 更 Compact 的 SlabNode
    + APFS 文件名长度最大只有 255 个字节(Linux 文件系统也是)
        + 改成用 namepool 变成偏移可以 24 byte -> (usize + u16)10 byte
        + https://superuser.com/questions/1561484/what-is-the-maximum-length-of-a-filename-apfs
        + NamePool 需要增加全局单例(运行过程中不变，对去重有更高的要求了)，且内部结构需要改造（改造成链表 + 内存块）（类似于 allocator）
+ 为什么扫描 /Library/Developer/CoreSimulator/Volumes/iOS_23A343 是单线程的？
+ 考虑类LSM/WAL设计?
+ icon 抓取之后异步 push，有些icon取得挺慢的
+ 重启+reopen window之后会很慢
+ 一个全局 string池，生命周期类似于 allocator，
    + 不适合用 allocator 形式，因为分配内存之后什么时候写入是不保证的

```bash
npm run tauri dev -- --release --features dev
npm run tauri build
```