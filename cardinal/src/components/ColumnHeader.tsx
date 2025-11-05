import React, { forwardRef } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import type { ColumnKey } from '../constants';
import { useTranslation } from 'react-i18next';

const columns: Array<{ key: ColumnKey; labelKey: string; className: string }> = [
  { key: 'filename', labelKey: 'columns.filename', className: 'filename-text' },
  { key: 'path', labelKey: 'columns.path', className: 'path-text' },
  { key: 'size', labelKey: 'columns.size', className: 'size-text' },
  { key: 'modified', labelKey: 'columns.modified', className: 'mtime-text' },
  { key: 'created', labelKey: 'columns.created', className: 'ctime-text' },
];

type ColumnHeaderProps = {
  onResizeStart: (columnKey: ColumnKey) => (event: ReactMouseEvent<HTMLSpanElement>) => void;
  onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void;
};

// Column widths are applied via CSS vars on container; no need to pass colWidths prop.
export const ColumnHeader = forwardRef<HTMLDivElement, ColumnHeaderProps>(
  ({ onResizeStart, onContextMenu }, ref) => {
    const { t } = useTranslation();
    return (
      <div ref={ref} className="header-row-container">
        <div className="header-row columns" onContextMenu={onContextMenu}>
          {columns.map(({ key, labelKey, className }) => (
            <span key={key} className={`${className} header header-cell`}>
              {t(labelKey)}
              <span
                className="col-resizer"
                onMouseDown={onResizeStart(key)} // consume column-specific resize closures from the parent hook
              />
            </span>
          ))}
          {/* Spacer for scrollbar width alignment */}
          <span className="header-scrollbar-spacer" />
        </div>
      </div>
    );
  },
);

ColumnHeader.displayName = 'ColumnHeader';
