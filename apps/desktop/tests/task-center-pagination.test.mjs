import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

/**
 * The task centre used to hold one fixed "latest 50" array per list and replace
 * it wholesale on every poll, which is why the page a reader was on could drift
 * and why neither list had real page numbers. Both lists now read through the
 * shared pager, and the poll re-reads the page on screen.
 */
const source = await readFile(
  new URL('../src/portability/TaskCenter.tsx', import.meta.url),
  'utf8',
);

test('task lists read through the shared page-number pager', () => {
  assert.match(source, /usePageNavigationPaging</);
  assert.match(source, /kind: 'operations_page'/);
  assert.match(source, /kind: 'jobs_page'/);
  assert.match(source, /readResearchPage\(identity!, cursor, size\)/);
  assert.match(source, /<ListPagination list={jobs} \/>/);
  assert.match(source, /<ListPagination list={operations} \/>/);
  assert.match(source, /<ListPagination list={research} \/>/);
});

test('the fixed-size list and its label are gone', () => {
  assert.doesNotMatch(source, /limit: 50/, 'no list may be capped at a fixed 50 rows');
  assert.doesNotMatch(source, /Latest 50|最近 50 项/, 'the page number replaced the tagline');
});

test('a poll refreshes the page on screen instead of restarting the list', () => {
  assert.match(source, /operationsRef\.current\.refresh\(\)/);
  assert.match(source, /jobsRef\.current\.refresh\(\)/);
  assert.doesNotMatch(source, /loadFirst\(\)/);
});

test('list errors come from the lists themselves, not a second copy', () => {
  assert.doesNotMatch(source, /loadErrors/);
  assert.match(source, /operations\.error/);
  assert.match(source, /jobs\.error/);
});
