import test from 'node:test';
import assert from 'node:assert/strict';
import {
  activeBookChapterIndex,
  adjacentBookChapterIndex,
  bookChapterTarget,
  isBookScrollIntentKey,
} from '../src/sources/book-navigation.ts';

const toc = [
  { title: 'Start', level: 1, block_id: 'b0', block_position: 0 },
  { title: 'Middle', level: 2, block_id: 'b8', block_position: 8 },
  { title: 'End', level: 1, block_id: null, block_position: null },
];

test('the active chapter is the last heading at or before the visible block', () => {
  assert.equal(activeBookChapterIndex(toc, -1), -1);
  assert.equal(activeBookChapterIndex(toc, 0), 0);
  assert.equal(activeBookChapterIndex(toc, 7), 0);
  assert.equal(activeBookChapterIndex(toc, 8), 1);
  assert.equal(activeBookChapterIndex(toc, 999), 1);
  assert.equal(activeBookChapterIndex([], 10), -1);
});

test('chapter movement is bounded and starts at the first heading', () => {
  assert.equal(adjacentBookChapterIndex(toc, -1, 1), 0);
  assert.equal(adjacentBookChapterIndex(toc, -1, -1), null);
  assert.equal(adjacentBookChapterIndex(toc, 1, 1), null);
  assert.equal(adjacentBookChapterIndex(toc, 2, 1), null);
  assert.equal(adjacentBookChapterIndex(toc, 0, -1), null);
  assert.equal(adjacentBookChapterIndex([], -1, 1), null);
});

test('a contents target keeps a short lead-in and handles missing positions', () => {
  assert.deepEqual(bookChapterTarget(toc, 1), {
    blockId: 'b8',
    position: 8,
    offset: 0,
  });
  assert.equal(bookChapterTarget(toc, 2, 8), null);
  assert.deepEqual(
    bookChapterTarget([{ title: 'Block only', level: 1, block_id: 'b9', block_position: null }], 0),
    { blockId: 'b9', position: 0, offset: 0 },
  );
  assert.equal(bookChapterTarget(toc, 9), null);
  assert.equal(bookChapterTarget(toc, 0, -5).offset, 0);
});
test('keyboard scrolling releases a programmatic chapter anchor', () => {
  for (const key of ['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' ']) {
    assert.equal(isBookScrollIntentKey(key), true, key);
  }
  for (const key of ['Tab', 'Escape', 'ArrowLeft', 'ArrowRight', 'a']) {
    assert.equal(isBookScrollIntentKey(key), false, key);
  }
});
