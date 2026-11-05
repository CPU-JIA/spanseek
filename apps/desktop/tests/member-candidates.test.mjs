import test from 'node:test';
import assert from 'node:assert/strict';
import {
  addableCount,
  collectionMemberIds,
  memberCandidateRows,
} from '../src/organization/member-candidates.ts';

const member = (id, kind = 'note', trashed = false) => ({
  position: 0,
  object: { id, kind, trashed, search_enabled: true },
  note: null,
  source: null,
  annotation: null,
});
const page = (items, next_cursor, version = 7) => ({
  collection_id: 'collection-a',
  collection_row_version: version,
  items,
  next_cursor,
});
const candidate = (id) => ({ id, title: id });

/** How the picker answered "already in this collection" before this change. */
const legacyExcluded = (memberPage) => memberPage.items.map((entry) => entry.object.id);
/** How the picker turned that answer into rows: drop those rows from the page. */
const legacyRows = (items, excluded) => items.filter((item) => !excluded.includes(item.id));

test('the member set walks every page, including trashed members', async () => {
  const cursors = [];
  const ids = await collectionMemberIds(async (cursor) => {
    cursors.push(cursor);
    return cursor
      ? page([member('note-2'), member('trash-1', 'note', true)], null)
      : page([member('note-1'), member('source-1', 'source')], 'next-page');
  }, 7);
  assert.deepEqual(cursors, [null, 'next-page']);
  assert.deepEqual([...ids].sort(), ['note-1', 'note-2', 'source-1', 'trash-1']);
});

test('a member on a later page is still a member, so it is never offered as addable', async () => {
  const first = page([member('note-1'), member('note-2')], 'next-page');
  const later = page([member('note-99')], null);
  const read = async (cursor) => (cursor ? later : first);

  // Before: the picker asked only the member page it happened to display, so a
  // note that is a member on page 2 looked addable. The core drops ids that are
  // already members, so that "add" reported success and changed nothing.
  const legacy = new Set(legacyExcluded(first));
  assert.equal(legacy.has('note-99'), false);
  assert.deepEqual(
    memberCandidateRows([candidate('note-99')], legacy).map((row) => row.inCollection),
    [false],
  );

  const complete = await collectionMemberIds(read, 7);
  assert.equal(complete.has('note-99'), true);
  assert.deepEqual(
    memberCandidateRows([candidate('note-99')], complete).map((row) => row.inCollection),
    [true],
  );
});

test('a page of rows that are all members keeps its page size instead of emptying out', () => {
  const items = Array.from({ length: 30 }, (_, index) => candidate(`note-${index + 1}`));
  const members = new Set(items.map((item) => item.id));

  // Before: exclusion ran after the read, so a full page rendered zero rows and
  // the picker told the user to load more of the same page.
  assert.equal(legacyRows(items, [...members]).length, 0);

  const rows = memberCandidateRows(items, members);
  assert.equal(rows.length, 30);
  assert.equal(addableCount(rows), 0);
  assert.deepEqual(
    rows.map((row) => row.inCollection),
    items.map(() => true),
  );
  // Page order is the command's order: the page renders the rows it was given.
  assert.deepEqual(
    rows.map((row) => row.item.id),
    items.map((item) => item.id),
  );
});

test('mixed pages count only the rows a user may still pick', () => {
  const items = [candidate('a'), candidate('b'), candidate('c')];
  const rows = memberCandidateRows(items, ['b', 'c']);
  assert.deepEqual(
    rows.map((row) => row.inCollection),
    [false, true, true],
  );
  assert.equal(addableCount(rows), 1);
  assert.equal(addableCount(memberCandidateRows(items, new Set())), 3);
});

test('a changed collection, duplicated members and a cursor loop are refused before anything is sent', async () => {
  await assert.rejects(
    collectionMemberIds(
      async (cursor) => page([member('a')], cursor ? null : 'next-page', cursor ? 8 : 7),
      7,
    ),
    /已有更新/,
  );
  await assert.rejects(
    collectionMemberIds(async () => page([member('a'), member('a')], null), 7),
    /成员已变化/,
  );
  let reads = 0;
  await assert.rejects(
    collectionMemberIds(async () => page([member(`note-${++reads}`)], 'repeated'), 7),
    /分页异常/,
  );
});
