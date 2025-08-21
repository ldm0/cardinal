import React, { forwardRef } from 'react';

export const ColumnHeader = forwardRef(({ colWidths, onResizeStart }, ref) => {
  return (
    <div ref={ref} className="header-row-container">
      <div className="header-row columns">
        <span className="filename-text header header-cell">
          Filename
          <span className="col-resizer" onMouseDown={onResizeStart('filename')} />
        </span>
        <span className="path-text header header-cell">
          Path
          <span className="col-resizer" onMouseDown={onResizeStart('path')} />
        </span>
        <span className="size-text header header-cell">
          Size
          <span className="col-resizer" onMouseDown={onResizeStart('size')} />
        </span>
        <span className="mtime-text header header-cell">
          Modified
          <span className="col-resizer" onMouseDown={onResizeStart('modified')} />
        </span>
        <span className="ctime-text header header-cell">
          Created
          <span className="col-resizer" onMouseDown={onResizeStart('created')} />
        </span>
      </div>
    </div>
  );
});
