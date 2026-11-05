import assert from 'node:assert/strict';
import test from 'node:test';
import { filterAnnotations } from '../src/sources/annotation-filter.ts';

const annotation = (id, quote, comment, pageIndex = null, pageLabel = null) => ({
  id,
  source_version_id: 'version',
  locator_schema: 2,
  locator: { kind: 'pdf', quote, page_index: pageIndex, page_label: pageLabel },
  comment,
  row_version: 1,
  trashed: false,
});

const rows = [
  annotation('a', 'Memory is a reconstruction.', '连接记忆与叙事', 9, '10'),
  annotation('b', 'The archive is a promise.', '来源可靠性', 11, 'xii'),
  annotation('c', '沉默 also carries information.', '非文本线索', 3, null),
];

test('an empty query keeps the loaded annotation order unchanged', () => {
  assert.deepEqual(filterAnnotations(rows, '  '), rows);
});

test('quotes, notes, page numbers, and labels are searchable together', () => {
  assert.deepEqual(
    filterAnnotations(rows, 'reconstruction').map((row) => row.id),
    ['a'],
  );
  assert.deepEqual(
    filterAnnotations(rows, '叙事').map((row) => row.id),
    ['a'],
  );
  assert.deepEqual(
    filterAnnotations(rows, '12').map((row) => row.id),
    ['b'],
  );
  assert.deepEqual(
    filterAnnotations(rows, 'XII').map((row) => row.id),
    ['b'],
  );
  assert.deepEqual(
    filterAnnotations(rows, '4').map((row) => row.id),
    ['c'],
  );
});

test('NFKC, case folding, and whitespace do not hide a match', () => {
  assert.deepEqual(
    filterAnnotations(rows, 'ＡＲＣＨＩＶＥ').map((row) => row.id),
    ['b'],
  );
  assert.deepEqual(
    filterAnnotations(rows, 'memory   is').map((row) => row.id),
    ['a'],
  );
  assert.deepEqual(filterAnnotations(rows, 'missing'), []);
});
