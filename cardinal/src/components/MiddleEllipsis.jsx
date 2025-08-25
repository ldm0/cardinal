import React, { useEffect, useRef, useState } from 'react';

const CHAR_WIDTH = 8; // approximate character width in pixels

export function MiddleEllipsis({ text, className }) {
  const containerRef = useRef(null);
  const [display, setDisplay] = useState(text || '');

  useEffect(() => {
    const computeDisplay = () => {
      const el = containerRef.current;
      if (!el) return;
      
      const str = text || '';
      if (!str) {
        setDisplay('');
        return;
      }

      const containerWidth = el.getBoundingClientRect().width;
      const maxChars = Math.floor(containerWidth / CHAR_WIDTH) - 1; // -1 for ellipsis

      if (str.length <= maxChars) {
        setDisplay(str);
        return;
      }

      if (maxChars <= 2) {
        setDisplay('…');
        return;
      }

      // Keep equal parts from start and end
      const leftChars = Math.floor(maxChars / 2);
      const rightChars = maxChars - leftChars;
      
      const leftStr = str.slice(0, leftChars);
      const rightStr = str.slice(str.length - rightChars);
      setDisplay(`${leftStr}…${rightStr}`);
    };

    computeDisplay();
    
    const resizeObserver = new ResizeObserver(computeDisplay);
    const el = containerRef.current;
    if (el) resizeObserver.observe(el);
    
    return () => resizeObserver.disconnect();
  }, [text]);

  return <span ref={containerRef} className={className} title={text}>{display}</span>;
}
