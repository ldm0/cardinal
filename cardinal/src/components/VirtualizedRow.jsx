import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatKB } from '../utils/format';
import { MiddleEllipsis } from './MiddleEllipsis';
import { ContextMenu } from './ContextMenu';

export function VirtualizedRow({ item, index, style }) {
  const [contextMenu, setContextMenu] = useState({ visible: false, x: 0, y: 0 });
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
      setContextMenu({ visible: true, x: e.clientX, y: e.clientY });
    }
  };

  const closeContextMenu = () => {
    setContextMenu({ ...contextMenu, visible: false });
  };

  const menuItems = [
    {
      label: 'Open in Finder',
      action: () => invoke('open_in_finder', { path }),
    },
  ];

  return (
    <>
      <div
        style={style}
        className={`row ${index % 2 === 0 ? 'row-even' : 'row-odd'}`}
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
      {contextMenu.visible && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={menuItems}
          onClose={closeContextMenu}
        />
      )}
    </>
  );
}
