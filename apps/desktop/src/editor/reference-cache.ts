/**
 * One shared memory for resolved references.
 *
 * Every chip, embed and picker asks the same question - "what is this target
 * right now?" - and the backend answers it identically for all of them. The
 * older in-flight map dropped its entry as soon as the request settled, so the
 * same target was asked again on every re-resolution trigger: committing the
 * note bumps the pane generation and every reference in the document starts
 * over, even though resolving a target never depends on the note that links it.
 *
 * A settled answer is now reused inside a short TTL. Callers that must not act
 * on a remembered answer - opening a target, validating an insertion - ask with
 * `fresh: true`, and a manual re-resolve drops the entry first.
 */

export const REFERENCE_CACHE_TTL_MS = 10_000;
/** Bounded so a long session across many notes cannot grow without limit. */
export const REFERENCE_CACHE_LIMIT = 256;

export type ReferenceCacheOptions = {
  ttlMs?: number;
  limit?: number;
  now?: () => number;
};

type ReferenceCacheEntry<T> = { value: T; expiresAt: number };
type ReferenceCacheRequest<T> = { serial: number; request: Promise<T> };

export class ReferenceCache<T> {
  private entries = new Map<string, ReferenceCacheEntry<T>>();
  private inFlight = new Map<string, ReferenceCacheRequest<T>>();
  private latest = new Map<string, number>();
  private serial = 0;
  private readonly ttlMs: number;
  private readonly limit: number;
  private readonly now: () => number;

  constructor(options: ReferenceCacheOptions = {}) {
    this.ttlMs = Math.max(0, options.ttlMs ?? REFERENCE_CACHE_TTL_MS);
    this.limit = Math.max(1, options.limit ?? REFERENCE_CACHE_LIMIT);
    this.now = options.now ?? (() => Date.now());
  }

  /** Remembered values; diagnostics and tests read it, callers use `peek`. */
  get size(): number {
    return this.entries.size;
  }

  /**
   * The remembered value of `key` while it is inside its TTL, so a caller can
   * render a known answer without waiting for a request. `T` must not be null.
   */
  peek(key: string): T | null {
    const entry = this.entries.get(key);
    if (!entry) return null;
    if (entry.expiresAt <= this.now()) {
      this.entries.delete(key);
      return null;
    }
    return entry.value;
  }

  /**
   * One request per key: callers that ask at the same time share the pending
   * promise, a settled value is reused inside its TTL, and a failure is never
   * remembered. `fresh` always asks again and replaces what is remembered.
   */
  resolve(key: string, load: () => Promise<T>, options: { fresh?: boolean } = {}): Promise<T> {
    if (!options.fresh) {
      const pending = this.inFlight.get(key);
      if (pending) return pending.request;
      const remembered = this.peek(key);
      if (remembered !== null) return Promise.resolve(remembered);
    }
    const serial = ++this.serial;
    this.latest.set(key, serial);
    const request = load().then(
      (value) => {
        this.release(key, serial);
        // A slower answer must never replace a newer one for the same key.
        if (this.latest.get(key) === serial) this.remember(key, value);
        return value;
      },
      (error) => {
        this.release(key, serial);
        throw error;
      },
    );
    this.inFlight.set(key, { serial, request });
    return request;
  }

  /** Drop one remembered value, or every one; pending answers stop being kept. */
  invalidate(key?: string): void {
    if (key === undefined) {
      this.entries.clear();
      this.latest.clear();
      return;
    }
    this.entries.delete(key);
    this.latest.set(key, ++this.serial);
  }

  private release(key: string, serial: number): void {
    if (this.inFlight.get(key)?.serial === serial) this.inFlight.delete(key);
  }

  private remember(key: string, value: T): void {
    // Insertion order is the eviction order, so re-entering a key refreshes it.
    this.entries.delete(key);
    this.entries.set(key, { value, expiresAt: this.now() + this.ttlMs });
    while (this.entries.size > this.limit) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      this.entries.delete(oldest.value);
    }
  }
}
