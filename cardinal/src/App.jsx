import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { once, listen } from '@tauri-apps/api/event';
import { InfiniteLoader, List, AutoSizer } from 'react-virtualized';
import 'react-virtualized/styles.css';
import "./App.css";

class LRUCache {
  constructor(capacity) {
    this.capacity = capacity;
    this.cache = new Map();
  }

  get(key) {
    if (!this.cache.has(key)) {
      return undefined;
    }
    const value = this.cache.get(key);
    this.cache.delete(key);
    this.cache.set(key, value);
    return value;
  }

  put(key, value) {
    if (this.cache.has(key)) {
      this.cache.delete(key);
    } else if (this.cache.size >= this.capacity) {
      const oldestKey = this.cache.keys().next().value;
      this.cache.delete(oldestKey);
    }
    this.cache.set(key, value);
  }

  has(key) {
    return this.cache.has(key);
  }

  clear() {
    this.cache.clear();
  }
}

// Format bytes into KB with one decimal place, e.g., 12.3 KB
function formatKB(bytes) {
  if (bytes == null) return null;
  const kb = bytes / 1024;
  if (!isFinite(kb)) return null;
  return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
}

function App() {
  const [results, setResults] = useState([]);
  const [colWidths, setColWidths] = useState({ path: 600, modified: 180, created: 180, size: 120 });
  const resizingRef = useRef(null);
  const lruCache = useRef(new LRUCache(1000));
  const infiniteLoaderRef = useRef(null);
  const debounceTimerRef = useRef(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [isStatusBarVisible, setIsStatusBarVisible] = useState(true);
  const [statusText, setStatusText] = useState("Walking filesystem...");
  // 新增refs用于同步竖直滚动条
  const scrollAreaRef = useRef(null);
  const listRef = useRef(null);
  const [verticalBar, setVerticalBar] = useState({ top: 0, height: 0, visible: false });

  useEffect(() => {
    listen('status_update', (event) => {
      setStatusText(event.payload);
    });
    once('init_completed', () => {
      setIsInitialized(true);
    });
  }, []);

  useEffect(() => {
    if (isInitialized) {
      const timer = setTimeout(() => {
        setIsStatusBarVisible(false);
      }, 2000);
      return () => clearTimeout(timer);
    }
  }, [isInitialized]);

  useEffect(() => {
    if (infiniteLoaderRef.current) {
      infiniteLoaderRef.current.resetLoadMoreRowsCache(true);
    }
  }, [results]);

  // 竖直滚动条同步逻辑
  useEffect(() => {
    function updateVerticalBar() {
      if (!listRef.current || !scrollAreaRef.current) return;
      const grid = listRef.current.Grid || listRef.current;
      const totalRows = results.length;
      const rowHeight = 24;
      const visibleHeight = grid.props.height;
      const totalHeight = totalRows * rowHeight;
      const scrollTop = grid.state ? grid.state.scrollTop : 0;
      if (totalHeight <= visibleHeight) {
        setVerticalBar({ top: 0, height: 0, visible: false });
        return;
      }
      const barHeight = Math.max(32, visibleHeight * visibleHeight / totalHeight);
      const barTop = (scrollTop / totalHeight) * visibleHeight;
      setVerticalBar({ top: barTop, height: barHeight, visible: true });
    }
    updateVerticalBar();
    if (!listRef.current) return;
    const grid = listRef.current.Grid || listRef.current;
    const onScroll = () => updateVerticalBar();
    grid && grid._scrollingContainer && grid._scrollingContainer.addEventListener('scroll', onScroll);
    return () => {
      grid && grid._scrollingContainer && grid._scrollingContainer.removeEventListener('scroll', onScroll);
    };
  }, [results]);

  // 拖动竖直滚动条
  const onVerticalBarMouseDown = (e) => {
    e.preventDefault();
    e.stopPropagation();
    const startY = e.clientY;
    const startTop = verticalBar.top;
    const grid = listRef.current?.Grid || listRef.current;
    const visibleHeight = grid?.props.height || 1;
    const totalRows = results.length;
    const rowHeight = 24;
    const totalHeight = totalRows * rowHeight;
    function onMove(ev) {
      const deltaY = ev.clientY - startY;
      let newTop = Math.max(0, Math.min(visibleHeight - verticalBar.height, startTop + deltaY));
      const scrollTop = (newTop / visibleHeight) * totalHeight;
      if (grid && grid._scrollingContainer) {
        grid._scrollingContainer.scrollTop = scrollTop;
      }
    }
    function onUp() {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp, { once: true });
  };

  const handleSearch = async (query) => {
    let searchResults = [];
    if (query.trim() !== '') {
      searchResults = await invoke("search", { query });
    }
    lruCache.current.clear();
    setResults(searchResults);
  };

  const onQueryChange = (e) => {
    const currentQuery = e.target.value;
    clearTimeout(debounceTimerRef.current);
    debounceTimerRef.current = setTimeout(() => {
      handleSearch(currentQuery);
    }, 300);
  };

  const onResizeStart = (key) => (e) => {
    e.preventDefault();
    e.stopPropagation();
    resizingRef.current = { key, startX: e.clientX, startW: colWidths[key] };
    window.addEventListener('mousemove', onResizing);
    window.addEventListener('mouseup', onResizeEnd, { once: true });
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'col-resize';
  };
  const onResizing = (e) => {
    const ctx = resizingRef.current;
    if (!ctx) return;
    const delta = e.clientX - ctx.startX;
    const nextW = Math.max(80, Math.min(1200, ctx.startW + delta));
    setColWidths((w) => ({ ...w, [ctx.key]: nextW }));
  };
  const onResizeEnd = () => {
    resizingRef.current = null;
    window.removeEventListener('mousemove', onResizing);
    document.body.style.userSelect = '';
    document.body.style.cursor = '';
  };

  const isRowLoaded = ({ index }) => {
    let loaded = lruCache.current.has(index);
    return loaded;
  };

  const loadMoreRows = async ({ startIndex, stopIndex }) => {
    let rows = results.slice(startIndex, stopIndex + 1);
    const searchResults = await invoke("get_nodes_info", { results: rows });
    for (let i = startIndex; i <= stopIndex; i++) {
      lruCache.current.put(i, searchResults[i - startIndex]);
    }
  };

  const rowRenderer = ({ key, index, style }) => {
    const item = lruCache.current.get(index);
    const path = typeof item === 'string' ? item : item?.path;
    const mtimeSec =
      typeof item !== 'string'
        ? (item?.metadata?.mtime ?? item?.mtime)
        : undefined;
    const mtimeText =
      mtimeSec != null ? new Date(mtimeSec * 1000).toLocaleString() : null;
    const ctimeSec =
      typeof item !== 'string'
        ? (item?.metadata?.ctime ?? item?.ctime)
        : undefined;
    const ctimeText =
      ctimeSec != null ? new Date(ctimeSec * 1000).toLocaleString() : null;
    const sizeBytes =
      typeof item !== 'string'
        ? (item?.metadata?.size ?? item?.size)
        : undefined;
    const sizeText = formatKB(sizeBytes);
    return (
      <div
        key={key}
        style={style}
        className={`row ${index % 2 === 0 ? 'row-even' : 'row-odd'}`}
      >
        {item ? (
          <div
            className="columns row-inner"
            title={path}
          >
            <span className="path-text">{path}</span>
            {mtimeText ? (
              <span className="mtime-text">{mtimeText}</span>
            ) : (
              <span className="mtime-text muted">—</span>
            )}
            {ctimeText ? (
              <span className="ctime-text">{ctimeText}</span>
            ) : (
              <span className="ctime-text muted">—</span>
            )}
            {sizeText ? (
              <span className="size-text">{sizeText}</span>
            ) : (
              <span className="size-text muted">—</span>
            )}
          </div>
        ) : (
          <div />
        )}
      </div>
    );
  };

  return (
    <main className="container">
      <div className="search-container">
        <input
          id="search-input"
          onChange={onQueryChange}
          placeholder="Search for files and folders..."
          spellCheck={false}
          autoCorrect="off"
          autoComplete="off"
          autoCapitalize="off"
        />
      </div>
      <div
        className="results-container"
        style={{
          ['--w-path']: `${colWidths.path}px`,
          ['--w-modified']: `${colWidths.modified}px`,
          ['--w-created']: `${colWidths.created}px`,
          ['--w-size']: `${colWidths.size}px`,
        }}
      >
        {/* 横向滚动区域 */}
        <div
          className="scroll-area"
          ref={scrollAreaRef}
        >
          <div className="header-row columns">
            <span className="path-text header header-cell">
              Path
              <span className="col-resizer" onMouseDown={onResizeStart('path')} />
            </span>
            <span className="mtime-text header header-cell">
              Modified
              <span className="col-resizer" onMouseDown={onResizeStart('modified')} />
            </span>
            <span className="ctime-text header header-cell">
              Created
              <span className="col-resizer" onMouseDown={onResizeStart('created')} />
            </span>
            <span className="size-text header header-cell">
              Size
              <span className="col-resizer" onMouseDown={onResizeStart('size')} />
            </span>
          </div>
          <div style={{ flex: 1, minHeight: 0 }}>
            <InfiniteLoader
              ref={infiniteLoaderRef}
              isRowLoaded={isRowLoaded}
              loadMoreRows={loadMoreRows}
              rowCount={results.length}
            >
              {({ onRowsRendered, registerChild }) => (
                <AutoSizer>
                  {({ height, width }) => {
                    const colGap = 12;
                    const columnsTotal =
                      colWidths.path + colWidths.modified + colWidths.created + colWidths.size + (3 * colGap) + 20;
                    return (
                      <List
                        ref={el => {
                          registerChild(el);
                          listRef.current = el;
                        }}
                        onRowsRendered={onRowsRendered}
                        width={Math.max(width, columnsTotal)}
                        height={height}
                        rowCount={results.length}
                        rowHeight={24}
                        rowRenderer={rowRenderer}
                      />
                    );
                  }}
                </AutoSizer>
              )}
            </InfiniteLoader>
          </div>
        </div>
        {/* 悬浮竖直滚动条 */}
        {verticalBar.visible && (
          <div className="vertical-scrollbar">
            <div
              className="vertical-scrollbar-inner"
              style={{
                height: verticalBar.height,
                top: verticalBar.top,
                position: 'absolute',
                right: 0,
              }}
              onMouseDown={onVerticalBarMouseDown}
            />
          </div>
        )}
      </div>
      {isStatusBarVisible && (
        <div className={`status-bar ${isInitialized ? 'fade-out' : ''}`}>
          {isInitialized ? 'Initialized' :
            <div className="initializing-container">
              <div className="spinner"></div>
              <span>{statusText}</span>
            </div>
          }
        </div>
      )}
    </main>
  );
}

export default App;
