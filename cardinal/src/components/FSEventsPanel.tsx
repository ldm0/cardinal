import React, {
  useCallback,
  useRef,
  memo,
  useEffect,
  useImperativeHandle,
  forwardRef,
  useMemo,
  useLayoutEffect,
  useState,
} from 'react';
import { List } from 'react-window';
import type { ListImperativeAPI, RowComponentProps } from 'react-window';
import { ROW_HEIGHT } from '../constants';
import { MiddleEllipsisHighlight } from './MiddleEllipsisHighlight';
import { formatTimestamp } from '../utils/format';
import type { RecentEventPayload } from '../types/ipc';
import { useTranslation } from 'react-i18next';

const COLUMNS = [
  { key: 'time', labelKey: 'events.columns.time' },
  { key: 'name', labelKey: 'events.columns.name' },
  { key: 'path', labelKey: 'events.columns.path' },
] as const;

type EventColumnKey = (typeof COLUMNS)[number]['key'];

// Distance (px) from the bottom that still counts as "user is at the end".
const BOTTOM_THRESHOLD = 50;

export type FileSystemEvent = RecentEventPayload;

type EventRowContext = {
  events: FileSystemEvent[];
  onContextMenu?: (event: React.MouseEvent<HTMLDivElement>, path: string) => void;
  searchQuery: string;
  caseInsensitive: boolean;
};

type EventRowProps = RowComponentProps<EventRowContext>;

const splitPath = (path: string | undefined): { name: string; directory: string } => {
  if (!path) {
    return { name: '—', directory: '' };
  }
  const normalized = path.replace(/\\/g, '/');
  if (normalized === '/') {
    return { name: '/', directory: '/' };
  }
  const slashIndex = normalized.lastIndexOf('/');
  if (slashIndex === -1) {
    return { name: normalized, directory: '' };
  }
  const directory = normalized.slice(0, slashIndex) || '/';
  const name = normalized.slice(slashIndex + 1) || normalized;
  return { name, directory };
};

const EventRow = memo(function EventRow({
  index,
  style,
  events,
  onContextMenu,
  searchQuery,
  caseInsensitive,
  ariaAttributes,
}: EventRowProps): React.JSX.Element {
  const event = events[index];
  const pathSource = event?.path ?? '';
  const { name, directory } = splitPath(pathSource);
  const timestamp = typeof event?.timestamp === 'number' ? event.timestamp : undefined;
  const formattedDate = formatTimestamp(timestamp) || '—';

  const handleContextMenu = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      if (pathSource && onContextMenu) {
        onContextMenu(e, pathSource);
      }
    },
    [pathSource, onContextMenu],
  );

  return (
    <div
      {...ariaAttributes}
      style={{ ...style, width: 'var(--columns-events-total)' }}
      className={`row columns-events ${index % 2 === 0 ? 'row-even' : 'row-odd'}`}
      title={pathSource}
      onContextMenu={handleContextMenu}
    >
      <div className="event-time-column">
        <span className="event-time-primary">{formattedDate}</span>
      </div>
      <div className="event-name-column">
        <MiddleEllipsisHighlight
          text={name || '—'}
          className="event-name-text"
          highlightTerm={searchQuery}
          caseInsensitive={caseInsensitive}
        />
      </div>
      <span className="event-path-text" title={directory}>
        {directory || (pathSource ? '/' : '—')}
      </span>
    </div>
  );
});

EventRow.displayName = 'EventRow';

type FSEventsPanelProps = {
  events: FileSystemEvent[];
  onResizeStart: (event: React.MouseEvent<HTMLSpanElement>, columnKey: EventColumnKey) => void;
  onContextMenu?: (event: React.MouseEvent<HTMLDivElement>, path: string) => void;
  onHeaderContextMenu?: (event: React.MouseEvent<HTMLDivElement>) => void;
  searchQuery: string;
  caseInsensitive: boolean;
};

export type FSEventsPanelHandle = {
  scrollToBottom: () => void;
};

const FSEventsPanel = forwardRef<FSEventsPanelHandle, FSEventsPanelProps>(
  (
    { events, onResizeStart, onContextMenu, onHeaderContextMenu, searchQuery, caseInsensitive },
    ref,
  ) => {
    const { t } = useTranslation();
    const headerRef = useRef<HTMLDivElement | null>(null);
    const listRef = useRef<ListImperativeAPI | null>(null);
    const scrollContainerRef = useRef<HTMLDivElement | null>(null);
    const listContainerRef = useRef<HTMLDivElement | null>(null);
    const [listSize, setListSize] = useState({ width: 0, height: 0 });
    const isAtBottomRef = useRef(true); // Track whether the viewport is watching the newest events.
    const prevEventsLengthRef = useRef(events.length);

    const restoreHorizontalScroll = useCallback((scrollLeft: number) => {
      if (scrollContainerRef.current) {
        scrollContainerRef.current.scrollLeft = scrollLeft;
      }
      if (headerRef.current) {
        headerRef.current.scrollLeft = scrollLeft;
      }
    }, []);

    const syncScrollState = useCallback(() => {
      const container = scrollContainerRef.current;
      if (!container) return;

      if (headerRef.current) {
        headerRef.current.scrollLeft = container.scrollLeft;
      }

      const distanceFromBottom =
        container.scrollHeight - (container.scrollTop + container.clientHeight);
      isAtBottomRef.current = distanceFromBottom <= BOTTOM_THRESHOLD;
    }, []);

    // Allow the parent (App) to imperatively jump to the latest event after tab switches.
    useImperativeHandle(
      ref,
      () => ({
        scrollToBottom: () => {
          const list = listRef.current;
          if (!list || events.length === 0) return;

          const previousScrollLeft = scrollContainerRef.current?.scrollLeft ?? 0;
          list.scrollToRow({ index: events.length - 1, align: 'end' });
          requestAnimationFrame(() => {
            restoreHorizontalScroll(previousScrollLeft);
          });
          isAtBottomRef.current = true; // Mark as at bottom.
        },
      }),
      [events.length, restoreHorizontalScroll],
    );

    const rowProps = useMemo<EventRowContext>(
      () => ({
        events,
        onContextMenu,
        searchQuery,
        caseInsensitive,
      }),
      [events, onContextMenu, searchQuery, caseInsensitive],
    );

    // Keep appending events visible when the user is already watching the feed tail.
    useEffect(() => {
      const prevLength = prevEventsLengthRef.current;
      const currentLength = events.length;
      prevEventsLengthRef.current = currentLength;

      if (currentLength > prevLength && isAtBottomRef.current) {
        const list = listRef.current;
        if (list && currentLength > 0) {
          const previousScrollLeft = scrollContainerRef.current?.scrollLeft ?? 0;
          list.scrollToRow({ index: currentLength - 1, align: 'end' });
          requestAnimationFrame(() => {
            restoreHorizontalScroll(previousScrollLeft);
          });
        }
      }
    }, [events.length, restoreHorizontalScroll]);

    const attachScrollContainer = useCallback(
      (instance: ListImperativeAPI | null) => {
        const previous = scrollContainerRef.current;
        if (previous) {
          previous.removeEventListener('scroll', syncScrollState);
          scrollContainerRef.current = null;
        }

        listRef.current = instance;

        const node = instance?.element ?? null;
        if (node) {
          scrollContainerRef.current = node;
          node.addEventListener('scroll', syncScrollState);
          syncScrollState();
        }
      },
      [syncScrollState],
    );

    useLayoutEffect(() => {
      const node = listContainerRef.current;
      if (!node) return;

      const observer = new ResizeObserver(([entry]) => {
        const { width, height } = entry.contentRect;
        setListSize((prev) =>
          prev.width === width && prev.height === height ? prev : { width, height },
        );
      });

      observer.observe(node);

      // Populate initial size synchronously so the list renders on first mount.
      const rect = node.getBoundingClientRect();
      setListSize({ width: rect.width, height: rect.height });

      return () => {
        observer.disconnect();
      };
    }, []);

    useEffect(() => {
      return () => {
        const container = scrollContainerRef.current;
        if (container) {
          container.removeEventListener('scroll', syncScrollState);
        }
      };
    }, [syncScrollState]);

    return (
      <div className="events-panel-wrapper">
        <div ref={headerRef} className="header-row-container">
          <div className="header-row columns-events" onContextMenu={onHeaderContextMenu}>
            {COLUMNS.map(({ key, labelKey }) => (
              <span key={key} className={`event-${key}-header header header-cell`}>
                {t(labelKey)}
                <span
                  className="col-resizer"
                  onMouseDown={(e) => onResizeStart(e, key)}
                  role="separator"
                  aria-orientation="vertical"
                />
              </span>
            ))}
          </div>
        </div>
        <div className="flex-fill">
          {events.length === 0 ? (
            <div className="events-empty" role="status">
              <p>{t('events.empty.title')}</p>
              <p className="events-empty__hint">{t('events.empty.hint')}</p>
            </div>
          ) : (
            <div className="events-list-container" ref={listContainerRef}>
              {listSize.width > 0 && listSize.height > 0 && (
                <List
                  listRef={attachScrollContainer}
                  rowComponent={EventRow}
                  rowCount={events.length}
                  rowHeight={ROW_HEIGHT}
                  rowProps={rowProps}
                  className="events-list"
                  style={{ width: listSize.width, height: listSize.height }}
                  overscanCount={10}
                />
              )}
            </div>
          )}
        </div>
      </div>
    );
  },
);

FSEventsPanel.displayName = 'FSEventsPanel';

export default FSEventsPanel;
