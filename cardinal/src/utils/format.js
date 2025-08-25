// Format bytes into human-readable sizes
export function formatBytes(bytes) {
  if (bytes == null || !isFinite(bytes)) return null;
  
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = bytes;
  let unitIndex = 0;
  
  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex++;
  }
  
  const decimals = size < 10 && unitIndex > 0 ? 1 : 0;
  return `${size.toFixed(decimals)} ${units[unitIndex]}`;
}

// Format bytes into KB with one decimal place (legacy function)
export function formatKB(bytes) {
  if (bytes == null || !isFinite(bytes)) return null;
  const kb = bytes / 1024;
  return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
}
