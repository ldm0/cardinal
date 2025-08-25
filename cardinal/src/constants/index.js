// UI Constants

// 列宽比例 - 基于窗口宽度的百分比分配
export const COL_WIDTH_RATIOS = {
  filename: 0.18, // 18%
  path: 0.45, // 45% - 最重要的列
  size: 0.08, // 8%
  modified: 0.145, // 14.5%
  created: 0.145 // 14.5%
};

// 根据窗口宽度计算初始列宽
export const calculateInitialColWidths = (windowWidth) => {
  // 减去间隙和额外空间
  const availableWidth = windowWidth - (Object.keys(COL_WIDTH_RATIOS).length - 1) * COL_GAP - COLUMNS_EXTRA - CONTAINER_PADDING * 2;
  
  const calculatedWidths = {};
  
  for (const [key, ratio] of Object.entries(COL_WIDTH_RATIOS)) {
    const calculatedWidth = Math.floor(availableWidth * ratio);
    calculatedWidths[key] = Math.max(calculatedWidth, MIN_COL_WIDTH);
  }
  
  return calculatedWidths;
};

export const COL_GAP = 12;
export const COLUMNS_EXTRA = 20;
export const ROW_HEIGHT = 24;
export const CONTAINER_PADDING = 10;

// Cache and Performance
export const CACHE_SIZE = 1000;
export const SEARCH_DEBOUNCE_MS = 300;
export const STATUS_FADE_DELAY_MS = 2000;
export const OVERSCAN_ROW_COUNT = 5;

export const MIN_COL_WIDTH = 30;
export const MAX_COL_WIDTH = 10000;

// Grid calculations
export const calculateColumnsTotal = (colWidths) =>
  Object.values(colWidths).reduce((sum, width) => sum + width, 0) + (Object.keys(colWidths).length - 1) * COL_GAP + COLUMNS_EXTRA;
