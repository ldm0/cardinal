import React, { useRef, useCallback } from "react";
import { InfiniteLoader, Grid, AutoSizer } from 'react-virtualized';
import 'react-virtualized/styles.css';
import "./App.css";
import { ContextMenu } from "./components/ContextMenu";
import { ColumnHeader } from "./components/ColumnHeader";
import { FileRow } from "./components/FileRow";
import { useAppState, useSearch, useVirtualizedList } from "./hooks";
import { useColumnResize } from "./hooks/useColumnResize";
import { useContextMenu } from "./hooks/useContextMenu";
import { ROW_HEIGHT, OVERSCAN_ROW_COUNT, calculateColumnsTotal } from "./constants";

function App() {
  const { results, setResults, isInitialized, isStatusBarVisible, statusText } = useAppState();
  const { colWidths, onResizeStart } = useColumnResize();
  const { lruCache, infiniteLoaderRef, isCellLoaded, loadMoreRows } = useVirtualizedList(results);
  const { contextMenu, showContextMenu, closeContextMenu, menuItems } = useContextMenu();
  const { onQueryChange } = useSearch(setResults, lruCache);
  
  const headerRef = useRef(null);
  const listRef = useRef(null);
  const scrollAreaRef = useRef(null);

  // 滚动同步处理 - 单向同步版本（Grid -> Header）
  const handleGridScroll = useCallback(({ scrollLeft }) => {
    if (headerRef.current) {
      headerRef.current.scrollLeft = scrollLeft;
    }
  }, []);

  // 单元格渲染
  const cellRenderer = ({ columnIndex, key, rowIndex, style }) => {
    // Grid只渲染一列，但我们把整行内容放在第一列
    if (columnIndex !== 0) return null;
    
    const item = lruCache.current.get(rowIndex);
    
    return (
      <FileRow
        key={key}
        item={item}
        rowIndex={rowIndex}
        style={style}
        onContextMenu={showContextMenu}
      />
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
          ['--w-filename']: `${colWidths.filename}px`,
          ['--w-path']: `${colWidths.path}px`,
          ['--w-size']: `${colWidths.size}px`,
          ['--w-modified']: `${colWidths.modified}px`,
          ['--w-created']: `${colWidths.created}px`,
        }}
      >
        <div className="scroll-area" ref={scrollAreaRef}>
          <ColumnHeader 
            ref={headerRef} 
            colWidths={colWidths} 
            onResizeStart={onResizeStart}
          />
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
                    const columnsTotal = calculateColumnsTotal(colWidths);
                    return (
                      <Grid
                        ref={el => {
                          registerChild(el);
                          listRef.current = el;
                        }}
                        onSectionRendered={({ rowStartIndex, rowStopIndex }) => 
                          onRowsRendered({ startIndex: rowStartIndex, stopIndex: rowStopIndex })
                        }
                        onScroll={handleGridScroll}
                        width={width}
                        height={height}
                        rowCount={results.length}
                        columnCount={1}
                        rowHeight={ROW_HEIGHT}
                        columnWidth={columnsTotal}
                        cellRenderer={cellRenderer}
                        overscanRowCount={OVERSCAN_ROW_COUNT}
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
