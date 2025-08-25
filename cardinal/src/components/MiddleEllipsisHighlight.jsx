import React, { useEffect, useRef, useState, useCallback, useMemo } from 'react';

const CHAR_WIDTH = 8; // approximate character width in pixels

function splitTextWithHighlight(text, searchQuery) {
  if (!searchQuery) return [{ text, isHighlight: false }];
  
  const parts = [];
  let start = 0;
  let index;
  
  while ((index = text.indexOf(searchQuery, start)) !== -1) {
    // 添加匹配前的文本
    if (index > start) {
      parts.push({ text: text.slice(start, index), isHighlight: false });
    }
    
    // 添加匹配的文本
    parts.push({ text: searchQuery, isHighlight: true });
    start = index + searchQuery.length;
  }
  
  // 添加剩余文本
  if (start < text.length) {
    parts.push({ text: text.slice(start), isHighlight: false });
  }
  
  return parts;
}

function applyMiddleEllipsis(parts, maxChars) {
  if (maxChars <= 2) {
    return [{ text: '…', isHighlight: false }];
  }

  const totalLength = parts.reduce((sum, part) => sum + part.text.length, 0);
  if (totalLength <= maxChars) {
    return parts;
  }

  const leftChars = Math.floor((maxChars - 1) / 2); // -1 for ellipsis
  const rightChars = maxChars - leftChars - 1;

  // 收集左侧部分
  const leftParts = [];
  let leftCount = 0;
  for (const part of parts) {
    const remainingSpace = leftChars - leftCount;
    if (remainingSpace <= 0) break;
    
    if (part.text.length <= remainingSpace) {
      leftParts.push(part);
      leftCount += part.text.length;
    } else {
      leftParts.push({
        text: part.text.slice(0, remainingSpace),
        isHighlight: part.isHighlight
      });
      break;
    }
  }

  // 收集右侧部分
  const rightParts = [];
  let rightCount = 0;
  for (let i = parts.length - 1; i >= 0; i--) {
    const part = parts[i];
    const remainingSpace = rightChars - rightCount;
    if (remainingSpace <= 0) break;
    
    if (part.text.length <= remainingSpace) {
      rightParts.unshift(part);
      rightCount += part.text.length;
    } else {
      rightParts.unshift({
        text: part.text.slice(-remainingSpace),
        isHighlight: part.isHighlight
      });
      break;
    }
  }

  return [...leftParts, { text: '…', isHighlight: false }, ...rightParts];
}

export function MiddleEllipsisHighlight({ text, className, searchQuery }) {
  const containerRef = useRef(null);
  const [containerWidth, setContainerWidth] = useState(0);

  // 计算高亮部分（只在 text 或 searchQuery 变化时重新计算）
  const highlightedParts = useMemo(() => {
    return text ? splitTextWithHighlight(text, searchQuery) : [];
  }, [text, searchQuery]);

  // 计算显示部分（只在 highlightedParts 或 containerWidth 变化时重新计算）
  const displayParts = useMemo(() => {
    if (!containerWidth || !highlightedParts.length) return highlightedParts;
    
    const maxChars = Math.floor(containerWidth / CHAR_WIDTH) - 1;
    return applyMiddleEllipsis(highlightedParts, maxChars);
  }, [highlightedParts, containerWidth]);

  // 优化的 resize 处理
  const updateWidth = useCallback(() => {
    const el = containerRef.current;
    if (el) {
      const newWidth = el.getBoundingClientRect().width;
      setContainerWidth(newWidth);
    }
  }, []);

  useEffect(() => {
    updateWidth();
    
    const resizeObserver = new ResizeObserver(updateWidth);
    const el = containerRef.current;
    if (el) resizeObserver.observe(el);
    
    return () => resizeObserver.disconnect();
  }, [updateWidth]);

  return (
    <span ref={containerRef} className={className} title={text}>
      {displayParts.map((part, index) => (part.isHighlight ? <strong key={index}>{part.text}</strong> : <span key={index}>{part.text}</span>))}
    </span>
  );
}
