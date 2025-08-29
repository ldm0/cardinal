// UI Constants

// 列宽比例 - 基于窗口宽度的百分比分配
export const COL_WIDTH_RATIOS = {
  filename: 0.25, // 25%
  path: 0.38, // 38%
  size: 0.08, // 8%
  modified: 0.145, // 14.5%
  created: 0.145 // 14.5%
};

// 根据窗口宽度计算初始列宽
export const calculateInitialColWidths = (windowWidth) => {
  // 使用常量滚动条宽度（需与 CSS 中的 --virtual-scrollbar-width 保持一致）
  const availableWidth = windowWidth - (Object.keys(COL_WIDTH_RATIOS).length - 1) * COL_GAP - COLUMNS_EXTRA - CONTAINER_PADDING - SCROLLBAR_WIDTH;
  
  const calculatedWidths = {};
  
  for (const [key, ratio] of Object.entries(COL_WIDTH_RATIOS)) {
    const calculatedWidth = Math.floor(availableWidth * ratio);
    calculatedWidths[key] = Math.max(calculatedWidth, MIN_COL_WIDTH);
  }
  
  return calculatedWidths;
};

export const COL_GAP = 12;
export const COLUMNS_EXTRA = 20; // 与 CSS --columns-extra 一致，避免多余宽度溢出
export const ROW_HEIGHT = 24;
export const CONTAINER_PADDING = 10;
// 与 CSS 变量 --virtual-scrollbar-width 保持同步
export const SCROLLBAR_WIDTH = 14;

// Minimum thumb height for the virtual scrollbar (in px). Keep this in sync with
// the CSS variable --virtual-scrollbar-thumb-min in src/App.css.
export const SCROLLBAR_THUMB_MIN = 24;

// Cache and Performance
export const CACHE_SIZE = 1000;
export const SEARCH_DEBOUNCE_MS = 300;
export const STATUS_FADE_DELAY_MS = 2000;
export const OVERSCAN_ROW_COUNT = 1;

export const MIN_COL_WIDTH = 30;
export const MAX_COL_WIDTH = 10000;
