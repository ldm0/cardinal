import { useRef, useCallback, useEffect, useState, useMemo } from 'react';
import type { ChangeEvent, CSSProperties, MouseEvent as ReactMouseEvent } from 'react';
import './App.css';
import { ContextMenu } from './components/ContextMenu';
import { ColumnHeader } from './components/ColumnHeader';
import { FileRow } from './components/FileRow';
import StatusBar from './components/StatusBar';
import type { StatusTabKey } from './components/StatusBar';
import type { NodeInfoResponse, SearchResultItem } from './types/search';
import type { AppLifecycleStatus, StatusBarUpdatePayload } from './types/ipc';
import { useColumnResize } from './hooks/useColumnResize';
import { useContextMenu } from './hooks/useContextMenu';
import { useFileSearch } from './hooks/useFileSearch';
import { useEventColumnWidths } from './hooks/useEventColumnWidths';
import { useRecentFSEvents } from './hooks/useRecentFSEvents';
import { ROW_HEIGHT, OVERSCAN_ROW_COUNT } from './constants';
import { VirtualList } from './components/VirtualList';
import type { VirtualListHandle } from './components/VirtualList';
import { StateDisplay } from './components/StateDisplay';
import FSEventsPanel from './components/FSEventsPanel';
import type { FSEventsPanelHandle } from './components/FSEventsPanel';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
  checkFullDiskAccessPermission,
  requestFullDiskAccessPermission,
} from 'tauri-plugin-macos-permissions-api';
import { useTranslation } from 'react-i18next';
import type { SlabIndex } from './types/slab';
import type { SortKey, SortState } from './types/sort';

const SORTABLE_RESULT_THRESHOLD = 1000;
const SORT_INDICATOR_CIRCLE_THRESHOLD = 10000;

type SortableMetadata = {
  path: string;
  size: number | null;
  mtime: number | null;
  ctime: number | null;
};

type SortEntry = {
  slabIndex: SlabIndex;
  idx: number;
};

const normalizePath = (value: string | undefined | null): string =>
  value ? value.toLocaleLowerCase() : '';

const numericValue = (metadata: SortableMetadata | undefined, key: SortKey): number => {
  switch (key) {
    case 'size':
      return typeof metadata?.size === 'number' ? metadata.size : Number.MIN_SAFE_INTEGER;
    case 'mtime':
      return typeof metadata?.mtime === 'number' ? metadata.mtime : Number.MIN_SAFE_INTEGER;
    case 'ctime':
      return typeof metadata?.ctime === 'number' ? metadata.ctime : Number.MIN_SAFE_INTEGER;
    default:
      return Number.MIN_SAFE_INTEGER;
  }
};

const compareEntries = (
  a: SortEntry,
  b: SortEntry,
  sortState: Exclude<SortState, null>,
  metadataMap: Map<number, SortableMetadata>,
): number => {
  const direction = sortState.direction === 'asc' ? 1 : -1;
  const metaA = metadataMap.get(a.slabIndex as number);
  const metaB = metadataMap.get(b.slabIndex as number);

  switch (sortState.key) {
    case 'fullPath': {
      const comparison = normalizePath(metaA?.path).localeCompare(normalizePath(metaB?.path), undefined, {
        sensitivity: 'base',
      });
      if (comparison !== 0) {
        return comparison * direction;
      }
      break;
    }
    case 'size':
    case 'mtime':
    case 'ctime': {
      const diff = numericValue(metaA, sortState.key) - numericValue(metaB, sortState.key);
      if (diff !== 0) {
        return diff > 0 ? direction : -direction;
      }
      break;
    }
    default:
      break;
  }

  return a.idx - b.idx;
};

type ActiveTab = StatusTabKey;

function App() {
  const {
    state,
    searchParams,
    updateSearchParams,
    queueSearch,
    resetSearchQuery,
    cancelPendingSearches,
    handleStatusUpdate,
    setLifecycleState,
    requestRescan,
  } = useFileSearch();
  const {
    results,
    scannedFiles,
    processedEvents,
    currentQuery,
    showLoadingUI,
    initialFetchCompleted,
    durationMs,
    resultCount,
    searchError,
    lifecycleState,
  } = state;
  const [sortState, setSortState] = useState<SortState>(null);
  const [sortedResults, setSortedResults] = useState<SlabIndex[]>(results);
  const sortMetadataRef = useRef<Map<number, SortableMetadata>>(new Map());
  const sortRequestRef = useRef(0);
  const [activeTab, setActiveTab] = useState<ActiveTab>('files');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [isWindowFocused, setIsWindowFocused] = useState<boolean>(() => {
    if (typeof document === 'undefined') {
      return true;
    }
    return document.hasFocus();
  });
  const totalResults = resultCount || results.length;
  const canSort = totalResults <= SORTABLE_RESULT_THRESHOLD;
  const shouldUseSortedResults = Boolean(sortState && canSort);
  const showCircleIndicators = totalResults > SORT_INDICATOR_CIRCLE_THRESHOLD;
  const displayedResults = shouldUseSortedResults ? sortedResults : results;
  const eventsPanelRef = useRef<FSEventsPanelHandle | null>(null);
  const headerRef = useRef<HTMLDivElement | null>(null);
  const virtualListRef = useRef<VirtualListHandle | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const isMountedRef = useRef(false);
  const { colWidths, onResizeStart, autoFitColumns } = useColumnResize();
  const { useRegex, caseSensitive } = searchParams;
  const { eventColWidths, onEventResizeStart, autoFitEventColumns } = useEventColumnWidths();
  const { filteredEvents, eventFilterQuery, setEventFilterQuery } = useRecentFSEvents({
    caseSensitive,
    useRegex,
  });
  const { t, i18n } = useTranslation();
  const sortLimitText = useMemo(
    () => new Intl.NumberFormat(i18n.language).format(SORTABLE_RESULT_THRESHOLD),
    [i18n.language],
  );
  const sortDisabledTooltip = canSort ? null : t('sorting.disabled', { limit: sortLimitText });
  const handleRowSelect = useCallback((path: string) => {
    setSelectedPath(path);
  }, []);
  const handleSortToggle = useCallback(
    (nextKey: SortKey) => {
      if (!canSort) return;
      setSortState((prev) => {
        if (!prev || prev.key !== nextKey) {
          return { key: nextKey, direction: 'asc' };
        }
        if (prev.direction === 'asc') {
          return { key: nextKey, direction: 'desc' };
        }
        return null;
      });
    },
    [canSort],
  );

  const {
    menu: filesMenu,
    showContextMenu: showFilesContextMenu,
    showHeaderContextMenu: showFilesHeaderContextMenu,
    closeMenu: closeFilesMenu,
    getMenuItems: getFilesMenuItems,
  } = useContextMenu(autoFitColumns);

  const {
    menu: eventsMenu,
    showContextMenu: showEventsContextMenu,
    showHeaderContextMenu: showEventsHeaderContextMenu,
    closeMenu: closeEventsMenu,
    getMenuItems: getEventsMenuItems,
  } = useContextMenu(autoFitEventColumns);

  const [fullDiskAccessStatus, setFullDiskAccessStatus] = useState<'granted' | 'denied'>('granted');
  const [isCheckingFullDiskAccess, setIsCheckingFullDiskAccess] = useState(true);
  const hasLoggedPermissionStatusRef = useRef(false);
  const menu = activeTab === 'events' ? eventsMenu : filesMenu;
  const showContextMenu = activeTab === 'events' ? showEventsContextMenu : showFilesContextMenu;
  const showHeaderContextMenu =
    activeTab === 'events' ? showEventsHeaderContextMenu : showFilesHeaderContextMenu;
  const closeMenu = activeTab === 'events' ? closeEventsMenu : closeFilesMenu;
  const getMenuItems = activeTab === 'events' ? getEventsMenuItems : getFilesMenuItems;

  useEffect(() => {
    const checkFullDiskAccess = async () => {
      setIsCheckingFullDiskAccess(true);
      try {
        const authorized = await checkFullDiskAccessPermission();
        if (!hasLoggedPermissionStatusRef.current) {
          console.log('Full Disk Access granted:', authorized);
          hasLoggedPermissionStatusRef.current = true;
        }
        setFullDiskAccessStatus(authorized ? 'granted' : 'denied');
      } catch (error) {
        console.error('Failed to check full disk access permission', error);
        setFullDiskAccessStatus('denied');
      } finally {
        setIsCheckingFullDiskAccess(false);
      }
    };

    void checkFullDiskAccess();
  }, []);

  const focusSearchInput = useCallback(() => {
    requestAnimationFrame(() => {
      const input = searchInputRef.current;
      if (!input) return;
      input.focus();
      input.select();
    });
  }, []);

  const ensureSortMetadata = useCallback(async (indices: SlabIndex[]) => {
    const missing: SlabIndex[] = [];
    indices.forEach((index) => {
      const key = index as number;
      if (!sortMetadataRef.current.has(key)) {
        missing.push(index);
      }
    });

    if (missing.length === 0) {
      return sortMetadataRef.current;
    }

    const fetched = await invoke<NodeInfoResponse[]>('get_nodes_info', {
      results: missing,
      include_icons: false,
    });

    const nextMap = new Map(sortMetadataRef.current);
    fetched.forEach((node, idx) => {
      const slabIndex = missing[idx];
      if (!node || slabIndex === undefined) {
        return;
      }
      nextMap.set(slabIndex as number, {
        path: node.path,
        size: typeof node.metadata?.size === 'number' ? node.metadata.size : null,
        mtime: node.metadata?.mtime ?? null,
        ctime: node.metadata?.ctime ?? null,
      });
    });
    sortMetadataRef.current = nextMap;
    return nextMap;
  }, []);

  useEffect(() => {
    isMountedRef.current = true;
    let unlistenStatus: UnlistenFn | undefined;
    let unlistenLifecycle: UnlistenFn | undefined;
    let unlistenQuickLaunch: UnlistenFn | undefined;

    const setupListeners = async (): Promise<void> => {
      unlistenStatus = await listen<StatusBarUpdatePayload>('status_bar_update', (event) => {
        if (!isMountedRef.current) return;
        const payload = event.payload;
        if (!payload) return;
        const { scannedFiles, processedEvents } = payload;
        handleStatusUpdate(scannedFiles, processedEvents);
      });

      unlistenLifecycle = await listen<AppLifecycleStatus>('app_lifecycle_state', (event) => {
        if (!isMountedRef.current) return;
        const status = event.payload;
        if (!status) return;
        setLifecycleState(status);
      });

      unlistenQuickLaunch = await listen('quick_launch', () => {
        if (!isMountedRef.current) return;
        focusSearchInput();
      });
    };

    void setupListeners();

    return () => {
      isMountedRef.current = false;
      unlistenStatus?.();
      unlistenLifecycle?.();
      unlistenQuickLaunch?.();
    };
  }, [focusSearchInput, handleStatusUpdate, setLifecycleState]);

  useEffect(() => {
    focusSearchInput();
  }, [focusSearchInput]);

  useEffect(() => {
    sortMetadataRef.current = new Map();
    setSortedResults(results);
  }, [results]);

  const runSort = useCallback(
    async (activeSort: SortState, sourceResults: SlabIndex[]) => {
      if (!activeSort || !canSort || sourceResults.length === 0) {
        setSortedResults(sourceResults);
        return;
      }

      const requestId = sortRequestRef.current + 1;
      sortRequestRef.current = requestId;

      try {
        await ensureSortMetadata(sourceResults);
        if (sortRequestRef.current !== requestId) {
          return;
        }

        const ordered = sourceResults
          .map((slabIndex, idx) => ({ slabIndex, idx }))
          .sort((a, b) => compareEntries(a, b, activeSort, sortMetadataRef.current));

        if (sortRequestRef.current === requestId) {
          setSortedResults(ordered.map((entry) => entry.slabIndex));
        }
      } catch (error) {
        console.error('Failed to sort search results', error);
        if (sortRequestRef.current === requestId) {
          setSortedResults(sourceResults);
        }
      }
    },
    [canSort, ensureSortMetadata],
  );

  useEffect(() => {
    void runSort(sortState, results);
  }, [runSort, sortState, results]);

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }
    const handleWindowFocus = () => setIsWindowFocused(true);
    const handleWindowBlur = () => setIsWindowFocused(false);
    window.addEventListener('focus', handleWindowFocus);
    window.addEventListener('blur', handleWindowBlur);
    return () => {
      window.removeEventListener('focus', handleWindowFocus);
      window.removeEventListener('blur', handleWindowBlur);
    };
  }, []);

  useEffect(() => {
    if (typeof document === 'undefined') {
      return;
    }
    document.documentElement.dataset.windowFocused = isWindowFocused ? 'true' : 'false';
  }, [isWindowFocused]);

  useEffect(() => {
    if (activeTab !== 'files') {
      setSelectedPath(null);
    }
  }, [activeTab]);

  useEffect(() => {
    if (activeTab !== 'files') {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      const isSpaceKey = event.code === 'Space' || event.key === ' ';
      if (!isSpaceKey || event.repeat) {
        return;
      }

      const target = event.target as HTMLElement | null;
      if (target) {
        const tagName = target.tagName;
        if (tagName === 'INPUT' || tagName === 'TEXTAREA' || target.isContentEditable) {
          return;
        }
      }

      if (!selectedPath) {
        return;
      }

      event.preventDefault();
      invoke('preview_with_quicklook', { path: selectedPath }).catch((error) => {
        console.error('Failed to preview file with Quick Look', error);
      });
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [activeTab, selectedPath]);

  useEffect(() => {
    const handleGlobalShortcuts = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) {
        return;
      }

      const key = event.key.toLowerCase();

      if (key === 'f') {
        event.preventDefault();
        focusSearchInput();
        return;
      }

      if (key === 'r') {
        if (activeTab !== 'files' || !selectedPath) {
          return;
        }
        event.preventDefault();
        invoke('open_in_finder', { path: selectedPath }).catch((error) => {
          console.error('Failed to reveal file in Finder', error);
        });
        return;
      }

      if (key === 'c') {
        if (activeTab !== 'files' || !selectedPath) {
          return;
        }
        event.preventDefault();
        if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
          navigator.clipboard.writeText(selectedPath).catch((error) => {
            console.error('Failed to copy file path', error);
          });
        }
      }
    };

    window.addEventListener('keydown', handleGlobalShortcuts);
    return () => window.removeEventListener('keydown', handleGlobalShortcuts);
  }, [focusSearchInput, activeTab, selectedPath]);

  const onQueryChange = useCallback(
    (e: ChangeEvent<HTMLInputElement>) => {
      const inputValue = e.target.value;

      if (activeTab === 'events') {
        setEventFilterQuery(inputValue);
      } else {
        queueSearch(inputValue);
      }
    },
    [activeTab, queueSearch, setEventFilterQuery],
  );

  const onToggleRegex = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const nextValue = event.target.checked;
      updateSearchParams({ useRegex: nextValue });
    },
    [updateSearchParams],
  );

  const onToggleCaseSensitive = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const nextValue = event.target.checked;
      updateSearchParams({ caseSensitive: nextValue });
    },
    [updateSearchParams],
  );

  useEffect(() => {
    // Reset vertical scroll and prefetch initial rows to keep first render responsive
    const list = virtualListRef.current;
    if (!list) return;

    list.scrollToTop?.();

    if (!displayedResults.length || !list.ensureRangeLoaded) {
      return;
    }

    const preloadCount = Math.min(30, displayedResults.length);
    list.ensureRangeLoaded(0, preloadCount - 1);
  }, [displayedResults]);

  const handleHorizontalSync = useCallback((scrollLeft: number) => {
    // VirtualList drives the scroll position; mirror it onto the sticky header for alignment
    if (headerRef.current) {
      headerRef.current.scrollLeft = scrollLeft;
    }
  }, []);

  const handleRowContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>, path: string) => {
      handleRowSelect(path);
      showContextMenu(event, path);
    },
    [handleRowSelect, showContextMenu],
  );

  const renderRow = useCallback(
    (rowIndex: number, item: SearchResultItem | undefined, rowStyle: CSSProperties) => (
      <FileRow
        key={rowIndex}
        item={item}
        rowIndex={rowIndex}
        style={{ ...rowStyle, width: 'var(--columns-total)' }} // Enforce column width CSS vars for virtualization rows
        onContextMenu={handleRowContextMenu}
        onSelect={handleRowSelect}
        isSelected={item ? selectedPath === item.path : false}
        searchQuery={currentQuery}
        caseInsensitive={!caseSensitive}
      />
    ),
    [handleRowContextMenu, handleRowSelect, selectedPath, currentQuery, caseSensitive],
  );

  const getDisplayState = (): 'loading' | 'error' | 'empty' | 'results' => {
    // Derive the UI state from search lifecycle, preserving existing semantics
    if (!initialFetchCompleted) return 'loading';
    if (showLoadingUI) return 'loading';
    if (searchError) return 'error';
    if (results.length === 0) return 'empty';
    return 'results';
  };

  const displayState = getDisplayState();
  const searchErrorMessage =
    typeof searchError === 'string' ? searchError : (searchError?.message ?? null);

  useEffect(() => {
    if (activeTab === 'events') {
      // Defer to next microtask so AutoSizer/Virtualized list have measured before scrolling
      queueMicrotask(() => {
        eventsPanelRef.current?.scrollToBottom?.();
      });
    }
  }, [activeTab]);

  const handleTabChange = useCallback(
    (newTab: ActiveTab) => {
      setActiveTab(newTab);
      if (newTab === 'events') {
        // Switch to events: always show newest items and clear transient filters
        setEventFilterQuery('');
      } else {
        // Switch to files: sync with reducer-managed search state and cancel pending timers
        resetSearchQuery();
        cancelPendingSearches();
      }
    },
    [cancelPendingSearches, resetSearchQuery, setEventFilterQuery],
  );

  const searchInputValue = activeTab === 'events' ? eventFilterQuery : searchParams.query;

  const containerStyle = {
    '--w-filename': `${colWidths.filename}px`,
    '--w-path': `${colWidths.path}px`,
    '--w-size': `${colWidths.size}px`,
    '--w-modified': `${colWidths.modified}px`,
    '--w-created': `${colWidths.created}px`,
    '--w-event-name': `${eventColWidths.name}px`,
    '--w-event-path': `${eventColWidths.path}px`,
    '--w-event-time': `${eventColWidths.time}px`,
    '--columns-events-total': `${
      eventColWidths.name + eventColWidths.path + eventColWidths.time
    }px`,
  } as CSSProperties;

  const showFullDiskAccessOverlay = fullDiskAccessStatus === 'denied';
  const overlayStatusMessage = isCheckingFullDiskAccess
    ? t('app.fullDiskAccess.status.checking')
    : t('app.fullDiskAccess.status.disabled');
  const caseSensitiveLabel = t('search.options.caseSensitive');
  const regexLabel = t('search.options.regex');

  return (
    <>
      <main className="container" aria-hidden={showFullDiskAccessOverlay}>
        <div className="search-container">
          <div className="search-bar">
            <input
              id="search-input"
              ref={searchInputRef}
              value={searchInputValue}
              onChange={onQueryChange}
              placeholder={
                activeTab === 'files'
                  ? t('search.placeholder.files')
                  : t('search.placeholder.events')
              }
              spellCheck={false}
              autoCorrect="off"
              autoComplete="off"
              autoCapitalize="off"
            />
            <div className="search-options">
              <label className="search-option" title={caseSensitiveLabel}>
                <input
                  type="checkbox"
                  checked={caseSensitive}
                  onChange={onToggleCaseSensitive}
                  aria-label={caseSensitiveLabel}
                />
                <span className="search-option__display" aria-hidden="true">
                  Aa
                </span>
                <span className="sr-only">{caseSensitiveLabel}</span>
              </label>
              <label className="search-option" title={regexLabel}>
                <input
                  type="checkbox"
                  checked={useRegex}
                  onChange={onToggleRegex}
                  aria-label={regexLabel}
                />
                <span className="search-option__display" aria-hidden="true">
                  .*
                </span>
                <span className="sr-only">{regexLabel}</span>
              </label>
            </div>
          </div>
        </div>
        <div className="results-container" style={containerStyle}>
          {activeTab === 'events' ? (
            <FSEventsPanel
              ref={eventsPanelRef}
              events={filteredEvents}
              onResizeStart={onEventResizeStart}
              onContextMenu={showContextMenu}
              onHeaderContextMenu={showHeaderContextMenu}
              searchQuery={eventFilterQuery}
              caseInsensitive={!caseSensitive}
            />
          ) : (
            <div className="scroll-area">
              <ColumnHeader
                ref={headerRef}
                onResizeStart={onResizeStart}
                onContextMenu={showHeaderContextMenu}
                sortState={sortState}
                onSortToggle={handleSortToggle}
                sortDisabled={!canSort}
                sortIndicatorMode={showCircleIndicators ? 'circle' : 'triangle'}
                sortDisabledTooltip={sortDisabledTooltip}
              />
              <div className="flex-fill">
                {displayState !== 'results' ? (
                  <StateDisplay
                    state={displayState}
                    message={searchErrorMessage}
                    query={currentQuery}
                  />
                ) : (
                  <VirtualList
                    ref={virtualListRef}
                    results={displayedResults}
                    rowHeight={ROW_HEIGHT}
                    overscan={OVERSCAN_ROW_COUNT}
                    renderRow={renderRow}
                    onScrollSync={handleHorizontalSync}
                    className="virtual-list"
                  />
                )}
              </div>
            </div>
          )}
        </div>
        {menu.visible && (
          <ContextMenu x={menu.x} y={menu.y} items={getMenuItems()} onClose={closeMenu} />
        )}
        <StatusBar
          scannedFiles={scannedFiles}
          processedEvents={processedEvents}
          lifecycleState={lifecycleState}
          searchDurationMs={durationMs}
          resultCount={resultCount}
          activeTab={activeTab}
          onTabChange={handleTabChange}
          onRequestRescan={requestRescan}
        />
      </main>
      {showFullDiskAccessOverlay && (
        <div className="permission-overlay">
          <div className="permission-card" role="dialog" aria-modal="true">
            <h1>{t('app.fullDiskAccess.title')}</h1>
            <p>{t('app.fullDiskAccess.description')}</p>
            <ol>
              <li>{t('app.fullDiskAccess.steps.one')}</li>
              <li>{t('app.fullDiskAccess.steps.two')}</li>
              <li>{t('app.fullDiskAccess.steps.three')}</li>
            </ol>
            <p className="permission-status" role="status" aria-live="polite">
              {overlayStatusMessage}
            </p>
            <div className="permission-actions">
              <button
                type="button"
                onClick={requestFullDiskAccessPermission}
                disabled={isCheckingFullDiskAccess}
              >
                {t('app.fullDiskAccess.openSettings')}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export default App;
