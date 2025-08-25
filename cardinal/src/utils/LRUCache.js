// LRU (Least Recently Used) Cache implementation for caching virtualized row data
export class LRUCache {
  constructor(capacity) {
    this.capacity = capacity;
    this.cache = new Map();
  }

  get(key) {
    if (!this.cache.has(key)) return undefined;
    
    // Move to end (most recently used)
    const value = this.cache.get(key);
    this.cache.delete(key);
    this.cache.set(key, value);
    return value;
  }

  put(key, value) {
    if (this.cache.has(key)) {
      // Update existing key
      this.cache.delete(key);
    } else if (this.cache.size >= this.capacity) {
      // Remove least recently used item
      const oldestKey = this.cache.keys().next().value;
      this.cache.delete(oldestKey);
    }
    
    this.cache.set(key, value);
  }

  has(key) {
    return this.cache.has(key);
  }

  clear() {
    this.cache.clear();
  }

  get size() {
    return this.cache.size;
  }
}
