import { useRef, useCallback, useEffect } from 'react';
import './App.css';
import { ContextMenu } from './components/ContextMenu';
import { ColumnHeader } from './components/ColumnHeader';
import { FileRow } from './components/FileRow';
import StatusBar from './components/StatusBar';
import { useAppState, useSearch } from './hooks';
import { useColumnResize } from './hooks/useColumnResize';
import { useContextMenu } from './hooks/useContextMenu';
import { ROW_HEIGHT, OVERSCAN_ROW_COUNT } from './constants';
import { VirtualList } from './components/VirtualList';
import { StateDisplay } from './components/StateDisplay';
import { usePreventRefresh } from './hooks/usePreventRefresh';

function App() {
  usePreventRefresh();
  const { results, setResults, isInitialized, scannedFiles, processedEvents } = useAppState();
  const { colWidths, onResizeStart, autoFitColumns } = useColumnResize();
  const {
    menu, showContextMenu, showHeaderContextMenu, closeMenu, getMenuItems
  } = useContextMenu(autoFitColumns);
  const { onQueryChange, currentQuery, showLoadingUI, initialFetchCompleted, durationMs, resultCount, searchError } = useSearch(setResults);

  const headerRef = useRef(null);
  const virtualListRef = useRef(null);
  const prevQueryRef = useRef('');
  const prevResultsLenRef = useRef(0);

  // 优化的搜索结果处理逻辑（保持使用 useRef，但简化其他逻辑）
  useEffect(() => {
    if (results.length === 0) return;
    const isNewQuery = prevQueryRef.current !== currentQuery;
    const wasEmpty = prevResultsLenRef.current === 0;

    if (isNewQuery && virtualListRef.current) {
      virtualListRef.current.scrollToTop();
    }

    if ((isNewQuery || wasEmpty) && virtualListRef.current?.ensureRangeLoaded) {
      const preloadCount = Math.min(30, results.length);
      virtualListRef.current.ensureRangeLoaded(0, preloadCount - 1);
    }
    prevQueryRef.current = currentQuery;
    prevResultsLenRef.current = results.length;
  }, [results, currentQuery]);

  useEffect(() => {
    const handleKeyDown = (event) => {
      // Prevent F5 or Ctrl+R (Windows/Linux) and Command+R (Mac) from refreshing the page
      if (
        event.key === 'F5' ||
        (event.ctrlKey && event.key === 'r') ||
        (event.metaKey && event.key === 'r')
      ) {
        event.preventDefault();
      }
    };

    const handleContextMenu = (event) => {
      // Only prevent the default context menu if the click is not on an element
      // that should have a custom context menu.
      // A simple check could be to see if the target or its parents have a specific class or attribute.
      // For now, we will prevent it everywhere except on rows and headers.
      if (!event.target.closest('.virtual-list') && !event.target.closest('.column-header')) {
        event.preventDefault();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    document.addEventListener('contextmenu', handleContextMenu);

    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('contextmenu', handleContextMenu);
    };
  }, []);

  useEffect(() => {
    const disableRefresh = () => { 
      document.addEventListener('keydown', function (event) { 
        // Prevent F5 or Ctrl+R (Windows/Linux) and Command+R (Mac) from refreshing the page 
        if ( 
          event.key === 'F5' || 
          (event.ctrlKey && event.key === 'r') || 
          (event.metaKey && event.key === 'r') 
        ) { 
          event.preventDefault(); 
        } 
      }); 
    
      document.addEventListener('contextmenu', function (event) { 
        event.preventDefault(); 
      }); 
    }; 
    disableRefresh();
  }, []);

  // 滚动同步处理 - 单向同步版本（Grid -> Header）
  const handleHorizontalSync = useCallback((scrollLeft) => {
    if (headerRef.current) headerRef.current.scrollLeft = scrollLeft;
  }, []);

  // 单元格渲染
  const renderRow = (rowIndex, item, rowStyle) => (
    <FileRow
      key={rowIndex}
      item={item}
      rowIndex={rowIndex}
      style={{ ...rowStyle, width: 'var(--columns-total)' }}
      onContextMenu={showContextMenu}
      searchQuery={currentQuery}
    />
  );

  const getDisplayState = () => {
    if (showLoadingUI || !initialFetchCompleted) return 'loading';
    if (searchError) return 'error';
    if (results.length === 0) return 'empty';
    return 'results';
  };

  const displayState = getDisplayState();

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
        <div className="scroll-area">
          <ColumnHeader
            ref={headerRef}
            onResizeStart={onResizeStart}
            onContextMenu={showHeaderContextMenu}
          />
          <div className="flex-fill">
            {displayState !== 'results' ? (
              <StateDisplay state={displayState} message={searchError} query={currentQuery} />
            ) : (
              <VirtualList
                ref={virtualListRef}
                results={results}
                rowHeight={ROW_HEIGHT}
                overscan={OVERSCAN_ROW_COUNT}
                renderRow={renderRow}
                onScrollSync={handleHorizontalSync}
                className="virtual-list"
              />
            )}
          </div>
        </div>
      </div>
      {menu.visible && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={getMenuItems()}
          onClose={closeMenu}
        />
      )}
      <StatusBar
        scannedFiles={scannedFiles}
        processedEvents={processedEvents}
        isReady={isInitialized}
        searchDurationMs={durationMs}
        resultCount={resultCount}
      />
    </main>
  );
}

export default App;
