import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { once, listen } from '@tauri-apps/api/event';
import { InfiniteLoader, Grid, AutoSizer } from 'react-virtualized';
import 'react-virtualized/styles.css';
import "./App.css";
import { LRUCache } from "./utils/LRUCache";
import { formatKB } from "./utils/format";
import { MiddleEllipsis } from "./components/MiddleEllipsis";
import { ContextMenu } from "./components/ContextMenu";
import { ColumnHeader } from "./components/ColumnHeader";

// 默认列宽
const DEFAULT_COL_WIDTHS = { filename: 240, path: 600, modified: 180, created: 180, size: 120 };
// 简化后的常量：列间距与额外补偿（用于横向滚动宽度计算）
const COL_GAP = 12;
const COLUMNS_EXTRA = 20;
const ROW_HEIGHT = 24;

function App() {
  const [results, setResults] = useState([]);
  const [colWidths, setColWidths] = useState(DEFAULT_COL_WIDTHS);
  const resizingRef = useRef(null);
  const lruCache = useRef(new LRUCache(1000));
  const infiniteLoaderRef = useRef(null);
  const debounceTimerRef = useRef(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [isStatusBarVisible, setIsStatusBarVisible] = useState(true);
  const [statusText, setStatusText] = useState("Walking filesystem...");
  const scrollAreaRef = useRef(null);
  const listRef = useRef(null);
  const [contextMenu, setContextMenu] = useState({ visible: false, x: 0, y: 0, path: null });

  // 状态事件
  useEffect(() => {
    listen('status_update', (event) => setStatusText(event.payload));
    once('init_completed', () => setIsInitialized(true));
  }, []);

  // 状态栏淡出
  useEffect(() => {
    if (isInitialized) {
      const timer = setTimeout(() => setIsStatusBarVisible(false), 2000);
      return () => clearTimeout(timer);
    }
  }, [isInitialized]);

  // 结果变更时重置加载缓存
  useEffect(() => {
    if (infiniteLoaderRef.current) {
      infiniteLoaderRef.current.resetLoadMoreRowsCache(true);
    }
  }, [results]);

  // 搜索
  const handleSearch = async (query) => {
    let searchResults = [];
    if (query.trim() !== '') {
      searchResults = await invoke("search", { query });
    }
    lruCache.current.clear();
    setResults(searchResults);
  };

  // 防抖
  const onQueryChange = (e) => {
    const currentQuery = e.target.value;
    clearTimeout(debounceTimerRef.current);
    debounceTimerRef.current = setTimeout(() => {
      handleSearch(currentQuery);
    }, 300);
  };

  // 列宽拖拽
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
    const rootStyle = getComputedStyle(document.documentElement);
    const minW = parseInt(rootStyle.getPropertyValue('--col-min-width')) || 80;
    const maxW = parseInt(rootStyle.getPropertyValue('--col-max-width')) || 1200;
    const nextW = Math.max(minW, Math.min(maxW, ctx.startW + delta));
    setColWidths((w) => ({ ...w, [ctx.key]: nextW }));
  };
  const onResizeEnd = () => {
    resizingRef.current = null;
    window.removeEventListener('mousemove', onResizing);
    document.body.style.userSelect = '';
    document.body.style.cursor = '';
  };

  // 虚拟列表加载
  const isCellLoaded = ({ rowIndex }) => lruCache.current.has(rowIndex);
  const loadMoreRows = async ({ startIndex, stopIndex }) => {
    let rows = results.slice(startIndex, stopIndex + 1);
    const searchResults = await invoke("get_nodes_info", { results: rows });
    for (let i = startIndex; i <= stopIndex; i++) {
      lruCache.current.put(i, searchResults[i - startIndex]);
    }
  };

  // 单元格渲染
  const cellRenderer = ({ columnIndex, key, rowIndex, style }) => {
    // Grid只渲染一列，但我们把整行内容放在第一列
    if (columnIndex !== 0) return null;
    
    const item = lruCache.current.get(rowIndex);
    const path = typeof item === 'string' ? item : item?.path;
    const filename = path ? path.split(/[\\/]/).pop() : '';
    const mtimeSec = typeof item !== 'string' ? (item?.metadata?.mtime ?? item?.mtime) : undefined;
    const mtimeText = mtimeSec != null ? new Date(mtimeSec * 1000).toLocaleString() : null;
    const ctimeSec = typeof item !== 'string' ? (item?.metadata?.ctime ?? item?.ctime) : undefined;
    const ctimeText = ctimeSec != null ? new Date(ctimeSec * 1000).toLocaleString() : null;
    const sizeBytes = typeof item !== 'string' ? (item?.metadata?.size ?? item?.size) : undefined;
    const sizeText = formatKB(sizeBytes);

    const handleContextMenu = (e) => {
      e.preventDefault();
      if (path) {
        setContextMenu({ visible: true, x: e.clientX, y: e.clientY, path });
      }
    };

    return (
      <div
        key={key}
        style={style}
        className={`row ${rowIndex % 2 === 0 ? 'row-even' : 'row-odd'}`}
        onContextMenu={handleContextMenu}
      >
        {item ? (
          <div className="columns row-inner" title={path}>
            <MiddleEllipsis className="filename-text" text={filename} />
            <MiddleEllipsis className="path-text" text={path} />
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

  // 上下文菜单处理
  const closeContextMenu = () => {
    setContextMenu({ ...contextMenu, visible: false });
  };

  const menuItems = [
    {
      label: 'Open in Finder',
      action: () => invoke('open_in_finder', { path: contextMenu.path }),
    },
  ];

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
          ['--w-filename']: `${colWidths.filename}px`,
          ['--w-path']: `${colWidths.path}px`,
          ['--w-modified']: `${colWidths.modified}px`,
          ['--w-created']: `${colWidths.created}px`,
          ['--w-size']: `${colWidths.size}px`,
        }}
      >
        <div className="scroll-area" ref={scrollAreaRef}>
          <ColumnHeader colWidths={colWidths} onResizeStart={onResizeStart} />
          <div style={{ flex: 1, minHeight: 0 }}>
            <InfiniteLoader
              ref={infiniteLoaderRef}
              isRowLoaded={isCellLoaded}
              loadMoreRows={loadMoreRows}
              rowCount={results.length}
            >
              {({ onRowsRendered, registerChild }) => (
                <AutoSizer>
                  {({ height, width }) => {
                    const columnsTotal =
                      colWidths.filename + colWidths.path + colWidths.modified + colWidths.created + colWidths.size + (4 * COL_GAP) + COLUMNS_EXTRA;
                    return (
                      <Grid
                        ref={el => {
                          registerChild(el);
                          listRef.current = el;
                        }}
                        onSectionRendered={({ rowStartIndex, rowStopIndex }) => 
                          onRowsRendered({ startIndex: rowStartIndex, stopIndex: rowStopIndex })
                        }
                        width={Math.max(width, columnsTotal)}
                        height={height}
                        rowCount={results.length}
                        columnCount={1}
                        rowHeight={ROW_HEIGHT}
                        columnWidth={Math.max(width, columnsTotal)}
                        cellRenderer={cellRenderer}
                      />
                    );
                  }}
                </AutoSizer>
              )}
            </InfiniteLoader>
          </div>
        </div>
      </div>
      {isStatusBarVisible && (
        <div className={`status-bar ${isInitialized ? 'fade-out' : ''}`}>
          {isInitialized ? 'Initialized' : (
            <div className="initializing-container">
              <div className="spinner"></div>
              <span>{statusText}</span>
            </div>
          )}
        </div>
      )}
      {contextMenu.visible && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={menuItems}
          onClose={closeContextMenu}
        />
      )}
    </main>
  );
}

export default App;
