import { actionErrorCode } from './page-actions.ts';

export type CursorPage<T> = { items: T[]; next_cursor: string | null };
export type QuerySnapshot<T> = {
  page: CursorPage<T> | null;
  loading: boolean;
  error: string | null;
};
/** Page results belong to one query. Only the last request may publish. */
export class ScopedQuery<T> {
  private sequence = 0;
  private state: QuerySnapshot<T> = { page: null, loading: false, error: null };
  private listeners = new Set<() => void>();
  private read: (cursor: string | null) => Promise<CursorPage<T>>;
  constructor(read: (cursor: string | null) => Promise<CursorPage<T>>) {
    this.read = read;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private emit(patch: Partial<QuerySnapshot<T>>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  invalidate() {
    ++this.sequence;
  }
  async load(more = false) {
    if (more && (!this.state.page?.next_cursor || this.state.loading)) return;
    const sequence = ++this.sequence;
    let cursor = more ? (this.state.page?.next_cursor ?? null) : null;
    this.emit({ loading: true, error: null });
    try {
      let page;
      try {
        page = await this.read(cursor);
      } catch (error) {
        if (sequence !== this.sequence) return;
        if (!cursor || actionErrorCode(error) !== 'query_expired') throw error;
        cursor = null;
        page = await this.read(null);
      }
      if (sequence !== this.sequence) return;
      this.emit({
        page:
          cursor && this.state.page
            ? { ...page, items: [...this.state.page.items, ...page.items] }
            : page,
      });
    } catch (error) {
      if (sequence === this.sequence)
        this.emit({ error: error instanceof Error ? error.message : String(error) });
    } finally {
      if (sequence === this.sequence) this.emit({ loading: false });
    }
  }
}
