import assert from 'node:assert/strict';
import test from 'node:test';
import {
  headingFoldSectionAt,
  headingFoldSections,
  headingFoldStorageKey,
  loadHeadingFoldIds,
  parseHeadingFoldIds,
  saveHeadingFoldIds,
  storedHeadingFoldIds,
} from '../src/editor/heading-fold-model.ts';

const blocks = [
  { type: 'heading', level: 1, id: 'h1', from: 0, to: 10 },
  { type: 'paragraph', level: 0, id: null, from: 10, to: 20 },
  { type: 'heading', level: 2, id: 'h2', from: 20, to: 30 },
  { type: 'paragraph', level: 0, id: null, from: 30, to: 40 },
  { type: 'heading', level: 3, id: 'h3', from: 40, to: 50 },
  { type: 'paragraph', level: 0, id: null, from: 50, to: 60 },
  { type: 'heading', level: 2, id: 'h2b', from: 60, to: 70 },
  { type: 'paragraph', level: 0, id: null, from: 70, to: 80 },
  { type: 'heading', level: 1, id: 'h1b', from: 80, to: 90 },
  { type: 'paragraph', level: 0, id: null, from: 90, to: 100 },
];

test('a heading section stops at the next heading of the same or a higher level', () => {
  const sections = headingFoldSections(blocks);
  assert.deepEqual(
    sections.map(({ id, nextIndex, foldedCount }) => ({ id, nextIndex, foldedCount })),
    [
      { id: 'h1', nextIndex: 8, foldedCount: 7 },
      { id: 'h2', nextIndex: 6, foldedCount: 3 },
      { id: 'h3', nextIndex: 6, foldedCount: 1 },
      { id: 'h2b', nextIndex: 8, foldedCount: 1 },
      { id: 'h1b', nextIndex: 10, foldedCount: 1 },
    ],
  );
});

test('the nearest preceding heading owns the selection, including paragraphs and nested headings', () => {
  assert.equal(headingFoldSectionAt(blocks, 5)?.id, 'h1');
  assert.equal(headingFoldSectionAt(blocks, 25)?.id, 'h2');
  assert.equal(headingFoldSectionAt(blocks, 45)?.id, 'h3');
  assert.equal(headingFoldSectionAt(blocks, 55)?.id, 'h3');
  assert.equal(headingFoldSectionAt(blocks, 65)?.id, 'h2b');
  assert.equal(headingFoldSectionAt(blocks, 75)?.id, 'h2b');
  assert.equal(headingFoldSectionAt(blocks, 85)?.id, 'h1b');
});

test('headings without an identifier still close a parent section but cannot be folded', () => {
  const mixed = [
    { type: 'heading', level: 1, id: 'parent', from: 0, to: 10 },
    { type: 'paragraph', level: 0, id: null, from: 10, to: 20 },
    { type: 'heading', level: 2, id: null, from: 20, to: 30 },
    { type: 'paragraph', level: 0, id: null, from: 30, to: 40 },
    { type: 'heading', level: 1, id: 'next', from: 40, to: 50 },
    { type: 'paragraph', level: 0, id: null, from: 50, to: 60 },
  ];
  assert.deepEqual(
    headingFoldSections(mixed).map(({ id, nextIndex, foldedCount }) => ({
      id,
      nextIndex,
      foldedCount,
    })),
    [
      { id: 'parent', nextIndex: 4, foldedCount: 3 },
      { id: 'next', nextIndex: 6, foldedCount: 1 },
    ],
  );
  assert.equal(headingFoldSectionAt(mixed, 25), null);
});

test('a document without headings has no foldable section', () => {
  assert.deepEqual(headingFoldSections(blocks.slice(1, 2)), []);
  assert.equal(headingFoldSectionAt(blocks.slice(1, 2), 15), null);
});
test('folded identifiers are validated and intersected with the current document', () => {
  assert.deepEqual([...parseHeadingFoldIds('["a","bad id","a","b-2",4]')], ['a', 'b-2']);
  assert.deepEqual(parseHeadingFoldIds('{"not":"an array"}'), new Set());
  assert.deepEqual(parseHeadingFoldIds('not json'), new Set());
  assert.deepEqual(storedHeadingFoldIds(['z', 'a', 'missing', 'a'], ['a', 'z']), ['a', 'z']);
});

test('folding view state is scoped by local instance, workspace and note', () => {
  assert.equal(
    headingFoldStorageKey('local-1', 'workspace-1', 'note-1'),
    'pkms.heading-fold.v1/local-1/workspace-1/note-1',
  );
});

test('optional folding storage round-trips and removes an empty state', () => {
  const values = new Map([['k', '["old"]']]);
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  assert.deepEqual([...loadHeadingFoldIds(storage, 'k')], ['old']);
  saveHeadingFoldIds(storage, 'k', new Set(['b', 'a', 'stale']), new Set(['a', 'b']));
  assert.equal(values.get('k'), '["a","b"]');
  assert.deepEqual([...loadHeadingFoldIds(storage, 'k')], ['a', 'b']);
  saveHeadingFoldIds(storage, 'k', new Set(), new Set(['a', 'b']));
  assert.equal(values.has('k'), false);
});
