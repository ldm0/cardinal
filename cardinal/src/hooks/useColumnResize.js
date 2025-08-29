import { useState, useCallback } from 'react';
import { calculateInitialColWidths, MAX_COL_WIDTH, MIN_COL_WIDTH } from '../constants';

export function useColumnResize() {
  const [colWidths, setColWidths] = useState(() => {
    const windowWidth = window.innerWidth;
    return calculateInitialColWidths(windowWidth);
  });

  const onResizeStart = useCallback((key) => (e) => {
    e.preventDefault();
    e.stopPropagation();

    const startX = e.clientX;
    const startWidth = colWidths[key];

    const handleMouseMove = (e) => {
      const delta = e.clientX - startX;
      const newWidth = Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, startWidth + delta));
      setColWidths(prev => ({ ...prev, [key]: newWidth }));
    };

    const handleMouseUp = () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'col-resize';
  }, [colWidths]);

  const autoFitColumns = useCallback(() => {
    const windowWidth = window.innerWidth;
    const newColWidths = calculateInitialColWidths(windowWidth);
    setColWidths(newColWidths);
  }, []);

  return {
    colWidths,
    onResizeStart,
    autoFitColumns
  };
}
