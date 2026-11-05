import test from 'node:test';
import assert from 'node:assert/strict';
import { PagedList } from '../src/layout/pagination.ts';
import { ScopedQuery } from '../src/organization/scoped-query.ts';

const ids = (page) => (page?.items ?? []).map((item) => item.id);

/** Stands in for `tags_list` / `notes_list` / `sources_list` / `collections_list`. */
function tagFetcher(count = 75) {
  const tags = Array.from({ length: count }, (_, index) => ({
    id: `tag-${index + 1}`,
    display_name: `标签 ${index + 1}`,
  }));
  const reads = [];
  const read = async (cursor, size) => {
    reads.push({ cursor, size });
    const start = cursor === null ? 0 : tags.findIndex((tag) => tag.id === cursor) + 1;
    const items = tags.slice(start, start + size);
    return {
      items,
      next_cursor: start + items.length < tags.length ? items.at(-1).id : null,
    };
  };
  return { tags, reads, read };
}

test('a converted list reads the page size it offers, and its pages never merge', async () => {
  const { tags, reads, read } = tagFetcher();
  const list = new PagedList(read, 'tags/ws-a', 30);
  await list.loadFirst();
  assert.deepEqual(reads, [{ cursor: null, size: 30 }]);
  assert.deepEqual(
    ids(list.getSnapshot().page),
    tags.slice(0, 30).map((tag) => tag.id),
  );

  // The per-page picker is honoured by the read, not just by the label.
  await list.setSize(50);
  assert.deepEqual(reads.at(-1), { cursor: null, size: 50 });
  assert.equal(list.getSnapshot().size, 50);
  await list.next();
  assert.equal(reads.at(-1).size, 50);
  assert.deepEqual(
    ids(list.getSnapshot().page),
    tags.slice(50).map((tag) => tag.id),
  );

  // Walking back and forward again covers every tag once, in order.
  await list.goTo(1);
  assert.deepEqual(
    ids(list.getSnapshot().page),
    tags.slice(0, 50).map((tag) => tag.id),
  );
  assert.deepEqual(
    reads.filter((entry) => entry.cursor === null).length,
    3,
    'each return to page 1 re-reads page 1',
  );
});

test('the append path it replaced kept one growing list and ignored the offered size', async () => {
  const { tags, reads, read } = tagFetcher(250);
  // `ScopedQuery` is the legacy shape still used by the sources collection picker:
  // its read takes a cursor only, so the size offered by the UI never reaches the
  // query, and every page is appended to the one list on screen.
  const legacy = new ScopedQuery((cursor) => read(cursor, 100));
  await legacy.load();
  assert.equal(legacy.getSnapshot().page.items.length, 100);
  await legacy.load(true);
  assert.equal(legacy.getSnapshot().page.items.length, 200);
  await legacy.load(true);
  assert.equal(legacy.getSnapshot().page.items.length, tags.length);
  assert.deepEqual(
    reads.map((entry) => entry.size),
    [100, 100, 100],
  );
  assert.deepEqual(
    reads.map((entry) => entry.cursor),
    [null, 'tag-100', 'tag-200'],
  );
  // 250 rows in one column, and nothing on screen says which page they came from.
  assert.equal(legacy.getSnapshot().page.next_cursor, null);
});

test('each page of a converted list holds its own rows only', async () => {
  const { tags, read } = tagFetcher(64);
  const list = new PagedList(read, 'tags/ws-a', 30);
  await list.loadFirst();
  const first = ids(list.getSnapshot().page);
  await list.next();
  const second = ids(list.getSnapshot().page);
  await list.next();
  const third = ids(list.getSnapshot().page);
  assert.deepEqual([first.length, second.length, third.length], [30, 30, 4]);
  assert.equal(new Set([...first, ...second, ...third]).size, tags.length);
  assert.deepEqual(
    [...first, ...second, ...third],
    tags.map((tag) => tag.id),
  );
  assert.equal(list.getSnapshot().canNext, false);
});
