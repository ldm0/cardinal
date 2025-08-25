import React, { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import './ContextMenu.css';

export function ContextMenu({ x, y, items, onClose }) {
  const menuRef = useRef(null);

  useEffect(() => {
    const handleClickOutside = (event) => {
      if (menuRef.current && !menuRef.current.contains(event.target)) {
        onClose();
      }
    };

    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [onClose]);

  const handleItemClick = (action) => {
    action();
    onClose();
  };

  return createPortal(
    <div ref={menuRef} className='context-menu' style={{ top: y, left: x }}>
      <ul>
        {items.map((item, index) => (
          <li key={index} onClick={() => handleItemClick(item.action)}>
            {item.label}
          </li>
        ))}
      </ul>
    </div>,
    document.body
  );
}