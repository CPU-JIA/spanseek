import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  DEFAULT_PAGE_SIZE,
  PAGE_SIZE_OPTIONS,
  PagedList,
  clampPageSize,
  PAGINATION_VARIANTS,
  pageCountText,
  pageCursor,
  pageIndexText,
  pageRangeText,
  pageStatusText,
  pageWindow,
  popCursor,
  pushCursor,
  shouldRenderPagination,
  totalPageCount,
  totalPageText,
} from '../src/layout/pagination.ts';

const ROWS = Array.from({ length: 75 }, (_, index) => `row-${index + 1}`);
const ids = (page) => (page?.items ?? []).map((item) => item.id);

/** Notes/sources style cursor: the cursor names the row the next page starts after. */
function windowFetcher(rows = ROWS) {
  const reads = [];
  const read = async (cursor, size) => {
    reads.push({ cursor, size });
    const start = cursor === null ? 0 : rows.indexOf(cursor) + 1;
    const items = rows.slice(start, start + size).map((id) => ({ id }));
    const last = rows[start + items.length - 1] ?? null;
    return { items, next_cursor: start + items.length < rows.length ? last : null };
  };
  return { read, reads };
}

const expired = () =>
  Object.assign(new Error('cursor expired'), { details: { code: 'query_expired' } });
const deferred = () => {
  let resolve;
  const promise = new Promise((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
};

test('page sizes are the offered options with a 10 default', () => {
  assert.deepEqual([...PAGE_SIZE_OPTIONS], [10, 20, 30, 50, 100]);
  assert.equal(DEFAULT_PAGE_SIZE, 10);
  assert.equal(clampPageSize(50), 50);
  assert.equal(clampPageSize(7), 10);
});

test('pageWindow reports the row range and the rows on the page', () => {
  assert.deepEqual(pageWindow(1, 30, 30), {
    index: 1,
    size: 30,
    start: 1,
    end: 30,
    count: 30,
  });
  assert.deepEqual(pageWindow(2, 50, 50), {
    index: 2,
    size: 50,
    start: 51,
    end: 100,
    count: 50,
  });
  assert.deepEqual(pageWindow(3, 30, 15), {
    index: 3,
    size: 30,
    start: 61,
    end: 75,
    count: 15,
  });
  assert.equal(pageRangeText(pageWindow(2, 50, 50), 'zh-CN'), '第 51–100 条');
  assert.equal(pageRangeText(pageWindow(2, 50, 50), 'en'), 'Items 51-100');
  assert.equal(pageCountText(pageWindow(3, 30, 15), 'zh-CN'), '本页 15 项');
  assert.equal(pageIndexText(4, 'zh-CN'), '第 4 页');
  assert.equal(pageIndexText(4, 'en'), 'Page 4');
  // An empty page has no range text, and no total means no page count.
  assert.equal(pageRangeText(pageWindow(2, 30, 0), 'zh-CN'), '');
  assert.equal(pageCountText(pageWindow(2, 30, 0), 'zh-CN'), '本页 0 项');
  // The footer is part of the page, so even an empty, non-navigable list renders it.
  assert.equal(shouldRenderPagination({ count: 0, canPrevious: false, canNext: false }), true);
  assert.equal(shouldRenderPagination({ count: 1, canPrevious: false, canNext: false }), true);
  assert.equal(shouldRenderPagination({ count: 0, canPrevious: true, canNext: false }), true);
  assert.equal(shouldRenderPagination({ count: 0, canPrevious: false, canNext: true }), true);
  assert.equal(totalPageCount(75, 30), 3);
  assert.equal(totalPageText(75, 30, 'zh-CN'), '共 3 页');
});

test('cursor stack pushes forward cursors and pops them on the way back', () => {
  assert.deepEqual(pushCursor([null], 'c1'), [null, 'c1']);
  assert.deepEqual(pushCursor([null, 'c1'], 'c2'), [null, 'c1', 'c2']);
  assert.deepEqual(popCursor([null, 'c1', 'c2']), [null, 'c1']);
  assert.deepEqual(popCursor([null]), [null]);
  assert.equal(pageCursor([null, 'c1', 'c2'], 1), null);
  assert.equal(pageCursor([null, 'c1', 'c2'], 2), 'c1');
  assert.equal(pageCursor([null, 'c1', 'c2'], 3), 'c2');
});

test('the first page has no previous, and the last page has no next', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  await list.loadFirst();
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().canPrevious, false);
  assert.equal(list.getSnapshot().canNext, true);
  assert.equal(await list.previous(), false);
  assert.equal(list.getSnapshot().index, 1);

  assert.equal(await list.next(), true);
  assert.equal(list.getSnapshot().index, 2);
  assert.equal(list.getSnapshot().canPrevious, true);
  assert.equal(await list.next(), true);
  assert.equal(list.getSnapshot().index, 3);
  assert.equal(list.getSnapshot().canNext, false);
  // No next_cursor: the button stays disabled and no further read happens.
  assert.equal(await list.next(), false);
  assert.equal(list.getSnapshot().index, 3);
  assert.equal(reads.length, 3);
  assert.deepEqual(
    reads.map((entry) => entry.cursor),
    [null, 'row-30', 'row-60'],
  );

  // One page at a time: going back re-reads that page and drops the rest.
  assert.equal(await list.previous(), true);
  assert.equal(list.getSnapshot().index, 2);
  assert.deepEqual(ids(list.getSnapshot().page), ROWS.slice(30, 60));
  assert.deepEqual(reads.at(-1), { cursor: 'row-30', size: 30 });
});

test('walking three pages covers every row once', async () => {
  const { read } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  await list.loadFirst();
  const seen = [...ids(list.getSnapshot().page)];
  while (list.getSnapshot().canNext) {
    await list.next();
    seen.push(...ids(list.getSnapshot().page));
  }
  assert.equal(seen.length, ROWS.length);
  assert.equal(new Set(seen).size, ROWS.length);
  assert.deepEqual(seen, ROWS);
  const snapshot = list.getSnapshot();
  assert.deepEqual(
    [pageRangeText(snapshot.window, 'zh-CN'), pageCountText(snapshot.window, 'zh-CN')],
    ['第 61–75 条', '本页 15 项'],
  );
});

test('changing the page size restarts at page 1 and never reuses the old cursor', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  await list.loadFirst();
  await list.next();
  assert.equal(list.getSnapshot().index, 2);
  assert.equal(list.getSnapshot().canPrevious, true);

  await list.setSize(100);
  const snapshot = list.getSnapshot();
  assert.equal(snapshot.index, 1);
  assert.equal(snapshot.size, 100);
  assert.equal(snapshot.canPrevious, false);
  assert.deepEqual(ids(snapshot.page), ROWS);
  assert.deepEqual(reads.at(-1), { cursor: null, size: 100 });
  assert.equal(snapshot.page.next_cursor, null);
});

test('a different scope drops the page in flight instead of mixing workspaces', async () => {
  const gate = deferred();
  const reads = [];
  const read = async (cursor, size) => {
    reads.push({ cursor, size });
    await gate.promise;
    return { items: [{ id: 'ws-a-note' }], next_cursor: 'ws-a-cursor' };
  };
  const list = new PagedList(read, 'notes/ws-a', 30);
  const pending = list.loadFirst();
  assert.equal(list.getSnapshot().loading, true);

  list.invalidate('scope', { scope: 'notes/ws-b' });
  assert.equal(list.scope, 'notes/ws-b');
  assert.equal(list.getSnapshot().page, null);
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().canPrevious, false);
  assert.equal(list.getSnapshot().loading, false);

  gate.resolve();
  assert.equal(await pending, false);
  // The late answer from the old workspace must not appear anywhere.
  assert.equal(list.getSnapshot().page, null);
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().error, null);
  assert.equal(reads.length, 1);
});

test('an identity change resets to page 1 with an empty cursor stack', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'identity/one', 30);
  await list.loadFirst();
  await list.next();
  await list.next();
  assert.equal(list.getSnapshot().index, 3);

  list.invalidate('identity', { scope: 'identity/two' });
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().page, null);
  assert.equal(list.getSnapshot().canPrevious, false);
  assert.equal(list.getSnapshot().canNext, false);

  await list.loadFirst();
  assert.deepEqual(reads.at(-1), { cursor: null, size: 30 });
  assert.deepEqual(ids(list.getSnapshot().page), ROWS.slice(0, 30));
});

test('a sort change restarts at the first page of the new order', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a/updated_desc', 30);
  await list.loadFirst();
  await list.next();
  list.invalidate('sort', { scope: 'notes/ws-a/title_asc' });
  assert.equal(list.getSnapshot().index, 1);
  await list.loadFirst();
  assert.deepEqual(reads.at(-1), { cursor: null, size: 30 });
});

test('an expired cursor falls back to page 1 with the stack cleared', async () => {
  const { read, reads } = windowFetcher();
  let expire = true;
  const list = new PagedList(
    async (cursor, size) => {
      if (expire && cursor !== null) throw expired();
      return read(cursor, size);
    },
    'notes/ws-a',
    30,
  );
  await list.loadFirst();
  assert.equal(list.getSnapshot().page.next_cursor, 'row-30');

  assert.equal(await list.next(), true);
  const snapshot = list.getSnapshot();
  assert.equal(snapshot.index, 1);
  assert.equal(snapshot.canPrevious, false);
  assert.equal(snapshot.error, null);
  assert.deepEqual(ids(snapshot.page), ROWS.slice(0, 30));
  assert.deepEqual(reads.at(-1), { cursor: null, size: 30 });
  // The failed cursor is gone: page 2 is reached again by walking forward.
  assert.equal(snapshot.canNext, true);
  expire = false;
  assert.equal(await list.next(), true);
  const second = list.getSnapshot();
  assert.equal(second.index, 2);
  assert.equal(second.canPrevious, true);
  assert.deepEqual(ids(second.page), ROWS.slice(30, 60));
});

test('query_expired resets an already loaded list to page 1', async () => {
  const { read } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 50);
  await list.loadFirst();
  await list.next();
  assert.equal(list.getSnapshot().index, 2);

  list.invalidate('query_expired');
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().page, null);
  assert.equal(list.getSnapshot().canPrevious, false);
  assert.throws(() => list.invalidate('nonsense'));
});

test('a failing first page reports the error instead of retrying forever', async () => {
  let calls = 0;
  const list = new PagedList(async () => {
    calls++;
    throw expired();
  }, 'notes/ws-a');
  assert.equal(await list.loadFirst(), false);
  assert.equal(calls, 1);
  assert.equal(list.getSnapshot().error, 'cursor expired');
  assert.equal(list.getSnapshot().loading, false);
  assert.equal(list.getSnapshot().index, 1);
});

test('a failed next page keeps the rows on screen and the retry cursor', async () => {
  let failed = false;
  const list = new PagedList(
    async (cursor) => {
      if (!cursor) return { items: [{ id: 'first' }], next_cursor: 'retry' };
      if (!failed) {
        failed = true;
        throw new Error('offline');
      }
      return { items: [{ id: 'second' }], next_cursor: null };
    },
    'scope',
    30,
  );
  assert.equal(await list.loadFirst(), true);
  assert.equal(await list.next(), false);
  assert.deepEqual(list.getSnapshot().page.items, [{ id: 'first' }]);
  assert.equal(list.getSnapshot().error, 'offline');
  assert.equal(list.getSnapshot().index, 1);
  assert.equal(list.getSnapshot().canNext, true);
  assert.equal(await list.next(), true);
  assert.deepEqual(list.getSnapshot().page.items, [{ id: 'second' }]);
  assert.equal(list.getSnapshot().error, null);
});

test('an empty page still reports page 1 with no next page', async () => {
  const list = new PagedList(async () => ({ items: [], next_cursor: null }), 'notes/ws-a', 30);
  await list.loadFirst();
  const snapshot = list.getSnapshot();
  assert.equal(snapshot.index, 1);
  assert.equal(snapshot.window.count, 0);
  assert.equal(snapshot.canNext, false);
  assert.equal(snapshot.canPrevious, false);
  assert.equal(pageRangeText(snapshot.window, 'zh-CN'), '');
  assert.equal(pageCountText(snapshot.window, 'zh-CN'), '本页 0 项');
});

test('goTo walks forward and back, and stops at the last real page', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  await list.loadFirst();
  assert.equal(await list.goTo(3), true);
  assert.equal(list.getSnapshot().index, 3);
  assert.deepEqual(ids(list.getSnapshot().page), ROWS.slice(60));
  assert.equal(await list.goTo(1), true);
  assert.equal(list.getSnapshot().index, 1);
  assert.deepEqual(ids(list.getSnapshot().page), ROWS.slice(0, 30));
  // Past the end: stay on the last page that exists.
  assert.equal(await list.goTo(9), false);
  assert.equal(list.getSnapshot().index, 3);
  assert.deepEqual(
    reads.map((entry) => entry.cursor),
    [null, 'row-30', 'row-60', 'row-30', null, 'row-30', 'row-60'],
  );
});

test('the snapshot object is stable until the page really changes', async () => {
  const { read } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  const first = list.getSnapshot();
  assert.equal(list.getSnapshot(), first);
  const seen = [];
  const unsubscribe = list.subscribe(() => seen.push(list.getSnapshot()));
  await list.loadFirst();
  const loaded = list.getSnapshot();
  assert.equal(loaded.index, 1);
  assert.equal(list.getSnapshot(), loaded);
  await list.next();
  const second = list.getSnapshot();
  assert.equal(second.index, 2);
  // Every notification carried a different snapshot, and the last one is current.
  assert.equal(seen.at(-1), second);
  assert.equal(new Set(seen).size, seen.length);
  unsubscribe();
  await list.next();
  assert.equal(seen.at(-1), second, 'no notification reaches an unsubscribed listener');
});

test('live updates replace visible rows without changing the page or cursor', async () => {
  const { read, reads } = windowFetcher();
  const list = new PagedList(read, 'notes/ws-a', 30);
  await list.loadFirst();
  const before = list.getSnapshot();
  list.updateItems((items) =>
    items.map((item) => (item.id === 'row-1' ? { id: item.id, live: 'updated' } : item)),
  );
  const after = list.getSnapshot();
  assert.equal(after.index, before.index);
  assert.equal(after.page.next_cursor, before.page.next_cursor);
  assert.deepEqual(after.page.items[0], { id: 'row-1', live: 'updated' });
  assert.equal(reads.length, 1);
});

const paginationCss = await readFile(
  new URL('../src/layout/pagination.css', import.meta.url),
  'utf8',
);

test('the pager offers exactly the footer and inline variants', () => {
  assert.deepEqual([...PAGINATION_VARIANTS], ['footer', 'inline']);
});

test('the footer status line never goes blank: an empty list reads 共 0 项', () => {
  assert.equal(pageStatusText(pageWindow(1, 30, 0), 'zh-CN'), '共 0 项');
  assert.equal(pageStatusText(pageWindow(1, 30, 0), 'en'), '0 items');
  assert.equal(pageStatusText(pageWindow(3, 30, 15), 'zh-CN'), '第 61–75 条 · 本页 15 项');
  assert.equal(pageStatusText(pageWindow(2, 50, 50), 'en'), 'Items 51-100 · 50 on this page');
});

test('an empty, non-navigable list still renders the footer', () => {
  const empty = { count: 0, canPrevious: false, canNext: false };
  assert.equal(shouldRenderPagination(empty), true);
  assert.equal(shouldRenderPagination({ count: 0, canPrevious: true, canNext: false }), true);
  assert.equal(shouldRenderPagination({ count: 0, canPrevious: false, canNext: true }), true);
  assert.equal(
    shouldRenderPagination({ count: 0, canPrevious: false, canNext: false, variant: 'inline' }),
    false,
  );
  assert.equal(
    shouldRenderPagination({ count: 1, canPrevious: false, canNext: false, variant: 'inline' }),
    true,
  );
});

test('the footer variant sticks to the bottom and keeps a single top rule', () => {
  assert.match(paginationCss, /\.pagination--footer\s*\{[^}]*margin-top:\s*auto/);
  assert.match(paginationCss, /\.pagination--inline\s*\{\s*border-top:\s*0/);
  // The page-shell footer already draws the row rule, so a nested pager must not add one.
  assert.match(paginationCss, /\.page-frame-footer:not\(\.is-inline\)\s*\.pagination--footer/);
  // The nested pager fills the footer's content box instead of adding a
  // second 48px minimum below the shared footer baseline.
  assert.match(
    paginationCss,
    /\.page-frame-footer:not\(\.is-inline\)\s*\.pagination--footer\s*\{[^}]*height:\s*100%[^}]*min-height:\s*0/,
  );
});

test('a narrow pager hides the label with display:none and keeps a 28px grid target', () => {
  assert.match(paginationCss, /container:\s*pagination\s*\/\s*inline-size/);
  assert.match(paginationCss, /@container\s+pagination\s*\(max-width:\s*460px\)/);
  assert.match(paginationCss, /\.pagination-step-label\s*\{\s*display:\s*none/);
  assert.match(paginationCss, /\.pagination\s+\.secondary-button\s*\{[^}]*display:\s*grid/);
  assert.match(paginationCss, /min-width:\s*28px/);
  assert.match(paginationCss, /min-height:\s*28px/);
  assert.doesNotMatch(paginationCss, /font-size:\s*0/);
});

test('numbered backend pages jump directly and keep the authoritative clamped page', async () => {
  const requests = [];
  const list = new PagedList(
    async (cursor, size) => {
      const requested = Number(cursor);
      requests.push(requested);
      const page = Math.min(requested, 11);
      return {
        items: [page],
        total: 101,
        page_index: page,
        next_cursor: page < 11 ? String(page + 1) : null,
      };
    },
    'numbered',
    10,
    true,
  );
  await list.loadFirst();
  await list.goTo(11);
  assert.deepEqual(requests, [1, 11]);
  assert.equal(list.getSnapshot().index, 11);
  assert.equal(list.getSnapshot().page.total, 101);
  await list.previous();
  assert.equal(list.getSnapshot().index, 10);
  await list.goTo(1000);
  assert.equal(list.getSnapshot().index, 11);
});

test('a newer numbered page wins when responses arrive out of order', async () => {
  const pending = new Map();
  const list = new PagedList(
    (cursor) => new Promise((resolve) => pending.set(Number(cursor), resolve)),
    'numbered',
    10,
    true,
  );
  const older = list.goTo(2);
  const newer = list.goTo(3);
  pending.get(3)({ items: ['new'], page_index: 3, total: 50, next_cursor: '4' });
  await newer;
  pending.get(2)({ items: ['old'], page_index: 2, total: 50, next_cursor: '3' });
  await older;
  assert.equal(list.getSnapshot().index, 3);
  assert.deepEqual(list.getSnapshot().page.items, ['new']);
});
