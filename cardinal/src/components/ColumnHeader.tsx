import React, { forwardRef } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import type { ColumnKey } from '../constants';
import type { SortKey, SortState } from '../types/sort';
import { useTranslation } from 'react-i18next';

const columns: Array<{ key: ColumnKey; labelKey: string; className: string }> = [
  { key: 'filename', labelKey: 'columns.filename', className: 'filename-text' },
  { key: 'path', labelKey: 'columns.path', className: 'path-text' },
  { key: 'size', labelKey: 'columns.size', className: 'size-text' },
  { key: 'modified', labelKey: 'columns.modified', className: 'mtime-text' },
  { key: 'created', labelKey: 'columns.created', className: 'ctime-text' },
];

const sortableColumns: Partial<Record<ColumnKey, SortKey>> = {
  path: 'fullPath',
  size: 'size',
  modified: 'mtime',
  created: 'ctime',
};

const sortingLabelKeys: Record<SortKey, string> = {
  fullPath: 'sorting.labels.fullPath',
  size: 'sorting.labels.size',
  mtime: 'sorting.labels.mtime',
  ctime: 'sorting.labels.ctime',
};

type ColumnHeaderProps = {
  onResizeStart: (columnKey: ColumnKey) => (event: ReactMouseEvent<HTMLSpanElement>) => void;
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void;
  sortState?: SortState;
  onSortToggle?: (sortKey: SortKey) => void;
  sortDisabled?: boolean;
  sortIndicatorMode?: 'triangle' | 'circle';
  sortDisabledTooltip?: string | null;
};

// Column widths are applied via CSS vars on container; no need to pass colWidths prop.
export const ColumnHeader = forwardRef<HTMLDivElement, ColumnHeaderProps>(
  (
    {
      onResizeStart,
      onContextMenu,
      sortState = null,
      onSortToggle,
      sortDisabled = false,
      sortIndicatorMode = 'triangle',
      sortDisabledTooltip,
    },
    ref,
  ) => {
    const { t } = useTranslation();
    return (
      <div ref={ref} className="header-row-container">
        <div className="header-row columns" onContextMenu={onContextMenu}>
          {columns.map(({ key, labelKey, className }) => {
            const sortKey = sortableColumns[key];
            const isSortable = Boolean(sortKey && onSortToggle);
            const isActive = Boolean(sortKey && sortState?.key === sortKey);
            const label = t(labelKey);
            const indicatorMode = sortIndicatorMode;
            const indicatorClasses = ['sort-indicator'];
            if (sortDisabled) {
              indicatorClasses.push('sort-indicator--disabled');
            }
            if (indicatorMode === 'circle') {
              indicatorClasses.push('sort-indicator--circle');
            } else if (isActive && sortState) {
              indicatorClasses.push(
                sortState.direction === 'asc' ? 'sort-indicator--asc' : 'sort-indicator--desc',
              );
            } else {
              indicatorClasses.push('sort-indicator--neutral');
            }

            if (isActive && !sortDisabled) {
              indicatorClasses.push('sort-indicator--active');
            }

            const sortLabel = sortKey ? t(sortingLabelKeys[sortKey]) : label;
            const title =
              sortDisabled && sortDisabledTooltip ? sortDisabledTooltip : sortLabel ?? label;
            const buttonClasses = ['sort-button'];
            if (isActive && !sortDisabled && indicatorMode !== 'circle') {
              buttonClasses.push('is-active');
            }

            return (
              <span key={key} className={`${className} header header-cell`}>
                {isSortable ? (
                  <button
                    type="button"
                    className={buttonClasses.join(' ')}
                    onClick={() => (sortKey ? onSortToggle?.(sortKey) : undefined)}
                    disabled={sortDisabled}
                    title={title}
                    aria-pressed={isActive && !sortDisabled}
                  >
                    <span className="sort-button__label">{label}</span>
                    <span
                      className={indicatorClasses.join(' ')}
                      aria-hidden="true"
                      data-mode={indicatorMode}
                    />
                  </button>
                ) : (
                  label
                )}
                <span
                  className="col-resizer"
                  onMouseDown={onResizeStart(key)} // consume column-specific resize closures from the parent hook
                />
              </span>
            );
          })}
          {/* Spacer for scrollbar width alignment */}
          <span className="header-scrollbar-spacer" />
        </div>
      </div>
    );
  },
);

ColumnHeader.displayName = 'ColumnHeader';
