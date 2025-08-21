import React from 'react';
import { MiddleEllipsis } from './MiddleEllipsis';
import { formatKB } from '../utils/format';

export function FileRow({ item, rowIndex, style, onContextMenu }) {
  if (!item) {
    return <div key={`empty-${rowIndex}`} style={style} />;
  }

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
    if (path && onContextMenu) {
      onContextMenu(e, path);
    }
  };

  return (
    <div
      style={style}
      className={`row ${rowIndex % 2 === 0 ? 'row-even' : 'row-odd'}`}
      onContextMenu={handleContextMenu}
    >
      <div className="columns row-inner" title={path}>
        <MiddleEllipsis className="filename-text" text={filename} />
        <MiddleEllipsis className="path-text" text={path} />
        <span className={`size-text ${!sizeText ? 'muted' : ''}`}>
          {sizeText || '—'}
        </span>
        <span className={`mtime-text ${!mtimeText ? 'muted' : ''}`}>
          {mtimeText || '—'}
        </span>
        <span className={`ctime-text ${!ctimeText ? 'muted' : ''}`}>
          {ctimeText || '—'}
        </span>
      </div>
    </div>
  );
}
