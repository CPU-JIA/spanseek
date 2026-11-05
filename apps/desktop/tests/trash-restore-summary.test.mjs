import test from 'node:test';
import assert from 'node:assert/strict';
import {
  groupTrashFailures,
  summarizeTrashSelection,
} from '../src/organization/trash-restore-summary.ts';

const item = (object_id, kind = 'note', extra = {}) => ({
  object_id,
  kind,
  title: `Item ${object_id}`,
  row_version: 2,
  preview: 'Synthetic fixture only',
  updated_at: '2026-10-14T08:00:00Z',
  parent_source_id: null,
  source_trashed: false,
  ...extra,
});

test('a restore batch reports its size by kind before it starts', () => {
  const summary = summarizeTrashSelection([
    item('note-1'),
    item('note-2'),
    item('source-1', 'source'),
    item('excerpt-1', 'annotation', { parent_source_id: 'source-1', source_trashed: true }),
  ]);
  assert.equal(summary.total, 4);
  assert.deepEqual(summary.counts, [
    { kind: 'note', count: 2 },
    { kind: 'source', count: 1 },
    { kind: 'annotation', count: 1 },
  ]);
  // The source is in the same batch, so the excerpt is not waiting for it.
  assert.equal(summary.excerptsWithoutSource, 0);
});

test('an excerpt whose trashed source is not in the batch is called out', () => {
  const summary = summarizeTrashSelection([
    item('excerpt-1', 'annotation', { parent_source_id: 'source-outside', source_trashed: true }),
    item('excerpt-2', 'annotation', { parent_source_id: null, source_trashed: true }),
    item('excerpt-3', 'annotation', { parent_source_id: 'source-1', source_trashed: false }),
  ]);
  assert.deepEqual(summary.counts, [{ kind: 'annotation', count: 3 }]);
  assert.equal(summary.excerptsWithoutSource, 2);
  assert.deepEqual(summarizeTrashSelection([]), { total: 0, counts: [], excerptsWithoutSource: 0 });
});

const failed = (index, code, error) => ({
  object_id: `object-${index}`,
  title: `Deleted item ${index}`,
  code,
  error,
});

test('a large failed batch renders one row per reason instead of one row per item', () => {
  const failures = [
    ...Array.from({ length: 180 }, (_, index) =>
      failed(index, 'object_trashed', 'The item is in Trash / 项目在回收站'),
    ),
    ...Array.from({ length: 120 }, (_, index) =>
      failed(1000 + index, 'metadata_conflict', 'Content changed / 内容已有变化'),
    ),
  ];
  // Before: `failed.map(...)` rendered 300 list rows, most of them identical.
  const legacyRows = failures.map((entry) => entry.code);
  assert.equal(legacyRows.length, 300);

  const groups = groupTrashFailures(failures);
  assert.equal(groups.length, 2);
  assert.deepEqual(
    groups.map((group) => [group.code, group.count, group.titles.length, group.remaining]),
    [
      ['object_trashed', 180, 3, 177],
      ['metadata_conflict', 120, 3, 117],
    ],
  );
  assert.equal(groups[0].titles[0], 'Deleted item 0');
  assert.equal(groups[0].message, 'The item is in Trash / 项目在回收站');
});

test('failures without a code group by their message, and the order is stable', () => {
  const groups = groupTrashFailures([
    failed(1, null, 'Destination is read-only'),
    failed(2, null, 'Destination is read-only'),
    failed(3, 'not_found', null),
  ]);
  assert.deepEqual(
    groups.map((group) => [group.key, group.code, group.message, group.count]),
    [
      ['message:Destination is read-only', null, 'Destination is read-only', 2],
      ['not_found', 'not_found', 'not_found', 1],
    ],
  );
  // The same input always renders the same order, so results do not shuffle.
  assert.deepEqual(groupTrashFailures([failed(3, 'not_found', null), failed(1, null, 'a')]), [
    {
      key: 'message:a',
      code: null,
      message: 'a',
      count: 1,
      titles: ['Deleted item 1'],
      remaining: 0,
    },
    {
      key: 'not_found',
      code: 'not_found',
      message: 'not_found',
      count: 1,
      titles: ['Deleted item 3'],
      remaining: 0,
    },
  ]);
  assert.deepEqual(groupTrashFailures([]), []);
});
