import { useRef, useCallback, useEffect, useState } from 'react';
import type { ChangeEvent, CSSProperties } from 'react';
import './App.css';
import { ContextMenu } from './components/ContextMenu';
import { ColumnHeader } from './components/ColumnHeader';
import { FileRow } from './components/FileRow';
import StatusBar from './components/StatusBar';
import type { StatusTabKey } from './components/StatusBar';
import type { SearchResultItem } from './types/search';
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
import { listen } from '@tauri-apps/api/event';
import type { UnlistenFn } from '@tauri-apps/api/event';
import {
  checkFullDiskAccessPermission,
  requestFullDiskAccessPermission,
} from 'tauri-plugin-macos-permissions-api';
import { useTranslation } from 'react-i18next';

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
  const [activeTab, setActiveTab] = useState<ActiveTab>('files');
  const eventsPanelRef = useRef<FSEventsPanelHandle | null>(null);
  const headerRef = useRef<HTMLDivElement | null>(null);
  const virtualListRef = useRef<VirtualListHandle | null>(null);
  const isMountedRef = useRef(false);
  const { colWidths, onResizeStart, autoFitColumns } = useColumnResize();
  const { useRegex, caseSensitive } = searchParams;
  const { eventColWidths, onEventResizeStart, autoFitEventColumns } = useEventColumnWidths();
  const { filteredEvents, eventFilterQuery, setEventFilterQuery } = useRecentFSEvents({
    caseSensitive,
    useRegex,
  });
  const { t } = useTranslation();

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

  useEffect(() => {
    isMountedRef.current = true;
    let unlistenStatus: UnlistenFn | undefined;
    let unlistenLifecycle: UnlistenFn | undefined;

    const setupListeners = async (): Promise<void> => {
      unlistenStatus = await listen<StatusBarUpdatePayload>('status_bar_update', (event) => {
        if (!isMountedRef.current) return;
        const payload = event.payload;
        if (!payload) return;
        const { scanned_files, processed_events } = payload;
        handleStatusUpdate(scanned_files, processed_events);
      });

      unlistenLifecycle = await listen<AppLifecycleStatus>('app_lifecycle_state', (event) => {
        if (!isMountedRef.current) return;
        const status = event.payload;
        if (!status) return;
        setLifecycleState(status);
      });
    };

    void setupListeners();

    return () => {
      isMountedRef.current = false;
      unlistenStatus?.();
      unlistenLifecycle?.();
    };
  }, [handleStatusUpdate, setLifecycleState]);

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

    if (!results.length || !list.ensureRangeLoaded) {
      return;
    }

    const preloadCount = Math.min(30, results.length);
    list.ensureRangeLoaded(0, preloadCount - 1);
  }, [results]);

  const handleHorizontalSync = useCallback((scrollLeft: number) => {
    // VirtualList drives the scroll position; mirror it onto the sticky header for alignment
    if (headerRef.current) {
      headerRef.current.scrollLeft = scrollLeft;
    }
  }, []);

  const renderRow = useCallback(
    (rowIndex: number, item: SearchResultItem | undefined, rowStyle: CSSProperties) => (
      <FileRow
        key={rowIndex}
        item={item}
        rowIndex={rowIndex}
        style={{ ...rowStyle, width: 'var(--columns-total)' }} // Enforce column width CSS vars for virtualization rows
        onContextMenu={showContextMenu}
        searchQuery={currentQuery}
        caseInsensitive={!caseSensitive}
      />
    ),
    [showContextMenu, currentQuery, caseSensitive],
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
