// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A tiny LRU cache. Distinct "eviction"/"capacity" vocabulary so a query about
// least-recently-used eviction lands on this class, not the validation file.

/// A fixed-capacity least-recently-used cache. When the cache is full, inserting
/// a new key evicts the least recently accessed entry.
export class LruCache<K, V> {
  private readonly capacity: number;
  private store: Map<K, V>;

  constructor(capacity: number) {
    this.capacity = Math.max(1, capacity);
    this.store = new Map();
  }

  /// Look up a key, marking it most-recently-used on a hit. Returns undefined on
  /// a miss.
  get(key: K): V | undefined {
    if (!this.store.has(key)) {
      return undefined;
    }
    const value = this.store.get(key) as V;
    this.store.delete(key);
    this.store.set(key, value);
    return value;
  }

  /// Insert or update a key, evicting the least-recently-used entry when the
  /// cache would exceed its capacity.
  set(key: K, value: V): void {
    if (this.store.has(key)) {
      this.store.delete(key);
    } else if (this.store.size >= this.capacity) {
      const oldest = this.store.keys().next().value as K;
      this.store.delete(oldest);
    }
    this.store.set(key, value);
  }

  /// Current number of entries held.
  get size(): number {
    return this.store.size;
  }
}

/// Build an LRU cache sized for a working set, never smaller than one slot.
export function makeBoundedCache<K, V>(workingSet: number): LruCache<K, V> {
  return new LruCache<K, V>(Math.max(1, workingSet));
}
