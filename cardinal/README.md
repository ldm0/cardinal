# TODO
- 需要实现流式列表（从 rust 层拿整个 btreemap 的 subslice） btreemap 做不到subslice，可能要换成 IndexMap
    - 理想的是一个插入O(logn)，查找O(logn)，用数字索引O(logn)的有序的数据结构
        - indexmap插入是 O(n) 的
        - skiplist?
            - crossbeam-skiplist? 不过不支持 mutable access
        - BTreeMap 看起来性能也行
- 需要在内容没有返回时阻塞滚动条
    - 标准的 scroll bar 没有实现这个，只能用假 scroll bar 了 https://github.com/yairEO/fakescroll/blob/master/react.fakescroll.js
    - AI 写半天写不好，先缓缓
- 目前 fsevent 处理慢:
    - 并行化 fsevent 的fs遍历部分，写数据部分抢锁
    - 只要有足够快速的 fsevent 处理，我们就可以做到搜索曾经曾经存在过的文件
    - 好像速度也能接受
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
- remove bincode, use postcard
    - SlabNode::children 用 IndexSet
    - name_index & MetadataCache::* 用 IndexSet 看看
    - 性能不是特别好
- metadata 启动后自动 fetching
- Windows/NTFS 支持
+ File icon
- native 内存泄漏排查
- native 内存占用高(mmap?)
    - namepool 和索引内存共享


```bash
npm run tauri dev -- --release
```