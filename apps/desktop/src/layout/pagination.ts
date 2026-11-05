import { actionErrorCode } from '../organization/page-actions.ts';
import { activeLanguage, type Language } from '../language.ts';

/**
 * Real page-number pagination for the cursor-based lists.
 *
 * A list command only answers "the page that follows this cursor", so a page
 * number is rebuilt from a stack of cursors: page 1 reads with `null`, and
 * entering page N+1 remembers the `next_cursor` of page N. Anything that
 * changes what page 1 means (workspace identity, filter/sort scope, page size,
 * an expired cursor) drops the whole stack, so a cursor recorded for one query
 * can never be replayed against another query or another workspace.
 */

export const PAGE_SIZE_OPTIONS = [10, 20, 30, 50, 100] as const;
export const DEFAULT_PAGE_SIZE = 10;
export type PageSize = (typeof PAGE_SIZE_OPTIONS)[number];

/** The two ways a pager can sit in a page: its own last row, or inline. */
export const PAGINATION_VARIANTS = ['footer', 'inline'] as const;
export type PaginationVariant = (typeof PAGINATION_VARIANTS)[number];

export function isPageSize(value: unknown): value is PageSize {
  return PAGE_SIZE_OPTIONS.includes(value as PageSize);
}

/** Sizes come from the picker, so an unknown value falls back to the default. */
export function clampPageSize(value: number): PageSize {
  return isPageSize(value) ? value : DEFAULT_PAGE_SIZE;
}

export type CursorPage<T> = {
  items: T[];
  next_cursor: string | null;
  total?: number;
  page_index?: number;
};

/** A cursor stack entry is the cursor that reads page `index + 1`. */
export type CursorStack = readonly (string | null)[];

/**
 * Every reason here makes page 1 mean something new. `scope` covers the
 * workspace identity plus whatever filters and sort the query carries.
 */
export const PAGE_RESET_REASONS = [
  'scope',
  'identity',
  'sort',
  'page_size',
  'query_expired',
] as const;
export type PageResetReason = (typeof PAGE_RESET_REASONS)[number];

export type PageWindow = {
  index: number;
  size: number;
  /** First row of the window in the whole result set, 1-based. */
  start: number;
  /** Last row of the window; below `start` when the page is empty. */
  end: number;
  count: number;
};

export function pageWindow(index: number, size: number, count: number): PageWindow {
  const page = Math.max(1, Math.floor(index) || 1);
  const width = Math.max(1, Math.floor(size) || DEFAULT_PAGE_SIZE);
  const rows = Math.max(0, Math.floor(count) || 0);
  const start = (page - 1) * width + 1;
  return { index: page, size: width, start, end: start + rows - 1, count: rows };
}

export function pageRangeText(window: PageWindow, language: Language = activeLanguage()): string {
  if (!window.count) return '';
  return language === 'en'
    ? `Items ${window.start}-${window.end}`
    : `第 ${window.start}–${window.end} 条`;
}

export function pageCountText(window: PageWindow, language: Language = activeLanguage()): string {
  return language === 'en' ? `${window.count} on this page` : `本页 ${window.count} 项`;
}

/**
 * The footer's status line: the row range on this page plus the rows it holds,
 * or the empty-list total so a footer never renders blank ("共 0 项").
 */
export function pageStatusText(window: PageWindow, language: Language = activeLanguage()): string {
  if (!window.count) return language === 'en' ? '0 items' : '共 0 项';
  const range = pageRangeText(window, language);
  const rows = pageCountText(window, language);
  return range ? `${range} · ${rows}` : rows;
}

/**
 * The pager is the pane's last row, not a reward for having rows. An empty list
 * with no previous or next page still renders it, showing "共 0 项" with both
 * step buttons disabled, so a short or empty list keeps one stable footer
 * instead of the pager vanishing. The predicate stays the single place that
 * decides this and therefore always renders.
 */
export function shouldRenderPagination(state: {
  count: number;
  canPrevious: boolean;
  canNext: boolean;
  variant?: PaginationVariant;
}): boolean {
  // A page footer is a stable part of the page even when its list is empty.
  // Inline pagers live inside pickers and panels instead, where an empty,
  // non-navigable control would just add furniture without an action.
  return state.variant !== 'inline' || state.count > 0 || state.canPrevious || state.canNext;
}
export function pageIndexText(index: number, language: Language = activeLanguage()): string {
  return language === 'en' ? `Page ${Math.max(1, index)}` : `第 ${Math.max(1, index)} 页`;
}

export function totalPageCount(total: number, size: number): number {
  return Math.max(1, Math.ceil(Math.max(0, total) / Math.max(1, size)));
}

/** Only lists that report a real total may show this; other lists have no N. */
export function totalPageText(total: number, size: number, language: Language = activeLanguage()) {
  const pages = totalPageCount(total, size);
  return language === 'en' ? `${pages} pages` : `共 ${pages} 页`;
}

export function pushCursor(cursors: CursorStack, next_cursor: string | null): CursorStack {
  return [...cursors, next_cursor];
}

export function popCursor(cursors: CursorStack): CursorStack {
  return cursors.length > 1 ? cursors.slice(0, -1) : [null];
}

/** The cursor that reads `index`; the first page always reads with `null`. */
export function pageCursor(cursors: CursorStack, index: number): string | null {
  if (index <= 1) return null;
  return cursors[index - 1] ?? null;
}

export type PagedSnapshot<T> = {
  page: CursorPage<T> | null;
  index: number;
  size: PageSize;
  window: PageWindow;
  loading: boolean;
  error: string | null;
  canPrevious: boolean;
  canNext: boolean;
  scope: string;
};

const errorText = (error: unknown) => {
  if (error instanceof Error) return error.message;
  const code = actionErrorCode(error);
  return code ?? String(error);
};

/**
 * One paged list. Results belong to one query: only the newest request may
 * publish, and a response that lands after a reset is discarded instead of
 * mixing another scope's rows into the current page.
 */
export class PagedList<T> {
  private read: (cursor: string | null, size: number) => Promise<CursorPage<T>>;
  private scopeValue: string;
  private numbered: boolean;
  private sizeValue: PageSize;
  private cursors: CursorStack = [null];
  private indexValue = 1;
  private pageValue: CursorPage<T> | null = null;
  private loadingValue = false;
  private errorValue: string | null = null;
  private sequence = 0;
  private listeners = new Set<() => void>();
  private state: PagedSnapshot<T>;

  constructor(
    read: (cursor: string | null, size: number) => Promise<CursorPage<T>>,
    scope = '',
    size: number = DEFAULT_PAGE_SIZE,
    numbered = false,
  ) {
    this.read = read;
    this.numbered = numbered;
    this.scopeValue = scope;
    this.sizeValue = clampPageSize(size);
    this.state = this.snapshot();
  }

  getSnapshot = (): PagedSnapshot<T> => this.state;

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /** The scope string a list was opened for; used to detect a wrong reuse. */
  get scope(): string {
    return this.scopeValue;
  }

  private snapshot(): PagedSnapshot<T> {
    return {
      page: this.pageValue,
      index: this.indexValue,
      size: this.sizeValue,
      window: pageWindow(this.indexValue, this.sizeValue, this.pageValue?.items.length ?? 0),
      loading: this.loadingValue,
      error: this.errorValue,
      canPrevious: this.indexValue > 1,
      canNext: Boolean(this.pageValue?.next_cursor),
      scope: this.scopeValue,
    };
  }

  /** Listeners hear about real changes only, so a snapshot stays stable. */
  private publish() {
    const next = this.snapshot();
    const previous = this.state;
    if (
      previous.page === next.page &&
      previous.index === next.index &&
      previous.size === next.size &&
      previous.loading === next.loading &&
      previous.error === next.error &&
      previous.scope === next.scope
    )
      return;
    this.state = next;
    for (const listener of this.listeners) listener();
  }

  /**
   * Return to page 1 and forget every cursor. Any response already in flight is
   * discarded, so an unknown or foreign page can never be published after this.
   */
  invalidate(reason: PageResetReason, options: { size?: number; scope?: string } = {}) {
    if (!PAGE_RESET_REASONS.includes(reason))
      throw new Error(`unknown page reset reason: ${String(reason)}`);
    ++this.sequence;
    if (options.size !== undefined) this.sizeValue = clampPageSize(options.size);
    if (options.scope !== undefined) this.scopeValue = options.scope;
    this.cursors = [null];
    this.indexValue = 1;
    this.pageValue = null;
    this.loadingValue = false;
    this.errorValue = null;
    this.publish();
  }

  /** A different page width changes every page boundary, so page 1 restarts. */
  setSize(size: number): Promise<boolean> {
    this.invalidate('page_size', { size });
    return this.loadFirst();
  }

  /** Page 1 of the current scope, with a cleared cursor stack. */
  loadFirst(): Promise<boolean> {
    this.cursors = [null];
    this.indexValue = 1;
    return this.run(1, null);
  }

  /**
   * Replace rows already on screen after a live poll. The cursor, page number
   * and page size stay unchanged; the next read still comes from the server.
   */
  updateItems(mutator: (items: readonly T[]) => T[]): void {
    if (!this.pageValue) return;
    const previous = this.pageValue.items;
    const items = mutator(previous);
    if (items.length === previous.length && items.every((item, index) => item === previous[index]))
      return;
    this.pageValue = { ...this.pageValue, items };
    this.publish();
  }
  /** Re-read the page on screen without changing the stack. */
  refresh(): Promise<boolean> {
    if (this.loadingValue) return Promise.resolve(false);
    return this.run(this.indexValue, pageCursor(this.cursors, this.indexValue));
  }

  async next(): Promise<boolean> {
    if (this.loadingValue) return false;
    const cursor = this.pageValue?.next_cursor;
    if (!cursor) return false;
    return this.run(this.indexValue + 1, cursor);
  }

  async previous(): Promise<boolean> {
    if (this.loadingValue || this.indexValue <= 1) return false;
    const target = this.indexValue - 1;
    // Going back drops the cursor recorded for the page being left behind.
    const cursor = pageCursor(popCursor(this.cursors), target);
    const reached = await this.run(target, cursor);
    return reached && this.indexValue === target;
  }

  /**
   * Walk to a page number, one cursor step at a time. Returns false when the
   * list ran out of pages first, which keeps the caller on the last real page.
   */
  async goTo(target: number): Promise<boolean> {
    const desired = Math.max(
      1,
      Math.min(0xffffffff, Number.isFinite(target) ? Math.floor(target) : 1),
    );
    if (this.numbered) return this.run(desired, String(desired));
    if (desired === this.indexValue && this.pageValue) return this.refresh();
    while (this.indexValue !== desired) {
      const moved = this.indexValue < desired ? await this.next() : await this.previous();
      if (!moved) return false;
    }
    return true;
  }

  private async run(index: number, cursor: string | null): Promise<boolean> {
    const sequence = ++this.sequence;
    this.loadingValue = true;
    this.errorValue = null;
    this.publish();
    try {
      let result: CursorPage<T>;
      let page = index;
      let effective = cursor;
      try {
        result = await this.read(this.numbered ? String(index) : cursor, this.sizeValue);
      } catch (error) {
        if (sequence !== this.sequence) return false;
        if (cursor === null || actionErrorCode(error) !== 'query_expired') throw error;
        // The cursor is gone (a rebuilt index, for example). Only page 1 can be
        // derived again, so the stack restarts instead of asking again with it.
        this.cursors = [null];
        this.indexValue = 1;
        page = 1;
        effective = null;
        result = await this.read(null, this.sizeValue);
      }
      if (sequence !== this.sequence) return false;
      if (this.numbered && result.page_index !== undefined) {
        if (!Number.isSafeInteger(result.page_index) || result.page_index < 1)
          throw new Error('Invalid page returned by the library');
        page = result.page_index;
      }
      // Keep the cursors of the pages still reachable, and the one that reads
      // the page after the fresh result. Anything deeper is stale now.
      const behind = pushCursor(this.cursors.slice(0, page - 1), effective);
      this.cursors = result.next_cursor ? pushCursor(behind, result.next_cursor) : behind;
      this.indexValue = page;
      this.pageValue = result;
      this.errorValue = null;
      this.publish();
      return true;
    } catch (error) {
      if (sequence === this.sequence) this.errorValue = errorText(error);
      return false;
    } finally {
      if (sequence === this.sequence) {
        this.loadingValue = false;
        this.publish();
      }
    }
  }
}
