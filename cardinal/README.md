# TODO
- 需要在内容没有返回时阻塞滚动条
    - 标准的 scroll bar 没有实现这个，只能用假 scroll bar 了 https://github.com/yairEO/fakescroll/blob/master/react.fakescroll.js
    - AI 写半天写不好，先缓缓
- 支持普通搜索，正则搜索, glob search
    - 不同的格式有开关按钮，类似于 vscode
    - 在输入不同的内容的时候自动推断是 glob 还是正则还是普通 substr 搜索，然后对应的按钮变成浅色
        - 用户可以手动勾选对应的模式按钮，变成深色
- 搜索结果自动更新
    - FSEvent 更新之后要重新拉取搜索结果
- 搜索结果path中query部分高亮(优化成Rust实现返回matched part)
- metadata 索引
    - ctime
    - mtime
    - size
- metadata 启动后自动 fetching
- Windows/NTFS 支持
- native 内存泄漏排查
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
+ 右键菜单提供“打开”、“复制路径”、“在 Finder/Explorer 中显示”等选项。
- 文件夹不显示文件大小

```bash
npm run tauri dev -- --release --features dev
npm run tauri build
```