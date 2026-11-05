import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { reviewTabCounts } from '../src/review/review-tab-counts.ts';

/**
 * The tab badges used to be derived by paging the whole queue in JavaScript
 * (bounded at 10,000 rows, then 「—」). They now come from one kernel read whose
 * buckets use the same `as_of` boundary the lists filter with.
 */
const counts = (overrides = {}) => ({
  due: 0,
  later: 0,
  completed: 0,
  removed: 0,
  as_of: '2026-10-17T00:00:00.000Z',
  ...overrides,
});

test('no counts yet keeps every tab unknown instead of claiming zero', () => {
  assert.equal(reviewTabCounts(null), null);
});

test('each kernel bucket lands on its own tab', () => {
  assert.deepEqual(reviewTabCounts(counts({ due: 3, later: 1, completed: 7, removed: 2 })), {
    due: 3,
    later: 1,
    completed: 7,
    removed: 2,
  });
});

test('the page reads the kernel count command instead of scanning the queue', async () => {
  const page = await readFile(new URL('../src/review/ReviewPage.tsx', import.meta.url), 'utf8');
  assert.match(page, /kind: 'review_counts'/);
  assert.match(page, /'review_counts',/);
  assert.match(page, /reviewTabCounts\(counts\)/);
  assert.doesNotMatch(page, /readReviewBucketCounts/, 'the badge must not page the queue itself');
  assert.doesNotMatch(page, /readReviewEntries/, 'the badge must not walk every bucket');
});
