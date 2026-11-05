import test from 'node:test';
import assert from 'node:assert/strict';
import {
  invalidateReviewIndex,
  readReviewEntries,
  reviewAddBaseVersion,
  reviewEntriesFor,
  selectReviewStatus,
} from '../src/review/review-status.ts';

const identity = { local_instance_id: 'local-a', workspace_id: 'workspace' };
const target = (id, revision = 'revision-1', block = null) => ({
  target_id: id,
  target_revision_id: revision,
  target_block_id: block,
});
const entry = (overrides = {}) => ({
  id: 'entry',
  target: target('note'),
  kind: 'note',
  state: 'pending',
  due_at: '2026-10-15T00:00:00.000Z',
  row_version: 1,
  created_at: '2026-10-14T00:00:00.000Z',
  updated_at: '2026-10-14T00:00:00.000Z',
  ...overrides,
});
const page = (entries, next_cursor = null) => ({
  items: entries.map((value) => ({ entry: value, resolved: {} })),
  next_cursor,
});
const tick = () => new Promise((resolve) => setImmediate(resolve));

test('the queue read covers every bucket and follows cursors', async () => {
  const calls = [];
  const entries = await readReviewEntries(async (bucket, cursor) => {
    calls.push(`${bucket}:${cursor ?? 'start'}`);
    if (bucket === 'later') return page([entry({ id: 'later-entry' })]);
    if (bucket === 'completed')
      return cursor
        ? page([entry({ id: 'completed-b', state: 'completed' })])
        : page([entry({ id: 'completed-a', state: 'completed' })], 'cursor-1');
    return page([]);
  });
  assert.deepEqual(calls, [
    'due:start',
    'later:start',
    'completed:start',
    'completed:cursor-1',
    'removed:start',
  ]);
  assert.deepEqual(
    entries.map((value) => value.id),
    ['later-entry', 'completed-a', 'completed-b'],
  );
});

test('a cursor that never advances is refused instead of looping', async () => {
  await assert.rejects(
    () => readReviewEntries(async () => page([entry()], 'same-cursor')),
    /回顾队列分页异常/,
  );
});

test('the entry freezing the submitted revision is the one a click reuses', () => {
  const queued = entry({ id: 'queued', row_version: 3 });
  const other = entry({
    id: 'other',
    state: 'completed',
    row_version: 9,
    target: target('note', 'revision-2'),
  });
  const status = selectReviewStatus([other, queued], target('note'));
  assert.deepEqual(status, { state: 'pending', entry: queued });
  assert.equal(reviewAddBaseVersion(status, target('note')), 3);
});

test('a completed entry keeps its version so the next click can requeue it', () => {
  const done = entry({ id: 'done', state: 'completed', row_version: 4 });
  const status = selectReviewStatus([done], target('note'));
  assert.deepEqual(status, { state: 'completed', entry: done });
  assert.equal(reviewAddBaseVersion(status, target('note')), 4);
  // A revision this button would not submit creates a new entry: no version.
  assert.equal(reviewAddBaseVersion(status, target('note', 'revision-2')), null);
});

test('another revision reports the queue state without a version to reuse', () => {
  const removed = entry({
    id: 'removed',
    state: 'removed',
    target: target('note', 'revision-2'),
  });
  const status = selectReviewStatus([removed], target('note'));
  assert.deepEqual(status, { state: 'removed', entry: null });
  assert.equal(reviewAddBaseVersion(status, target('note')), null);
  assert.deepEqual(selectReviewStatus([], target('note')), { state: 'none', entry: null });
  assert.equal(reviewAddBaseVersion(null, target('note')), null);
});

test('several revisions report the state that still needs attention', () => {
  const removed = entry({
    id: 'removed',
    state: 'removed',
    target: target('note', 'revision-2'),
  });
  const queued = entry({ id: 'queued', target: target('note', 'revision-3') });
  assert.deepEqual(selectReviewStatus([removed, queued], target('note')), {
    state: 'pending',
    entry: null,
  });
});

test('a block target and the whole note are different review subjects', () => {
  const block = entry({ id: 'block', target: target('note', 'revision-1', 'block-1') });
  assert.deepEqual(selectReviewStatus([block], target('note')), { state: 'none', entry: null });
  assert.deepEqual(selectReviewStatus([block], target('note', 'revision-1', 'block-1')), {
    state: 'pending',
    entry: block,
  });
});

test('one queue read serves every control until a mutation invalidates it', async () => {
  invalidateReviewIndex(identity);
  let reads = 0;
  const read = async (bucket) => {
    reads += 1;
    return page(bucket === 'due' ? [entry({ id: 'queued' })] : []);
  };
  const [first, second] = await Promise.all([
    reviewEntriesFor(identity, read),
    reviewEntriesFor(identity, read),
  ]);
  assert.equal(reads, 4);
  assert.deepEqual(first, second);
  assert.equal((await reviewEntriesFor(identity, read)).length, 1);
  assert.equal(reads, 4);
  invalidateReviewIndex(identity);
  await reviewEntriesFor(identity, read);
  assert.equal(reads, 8);
  invalidateReviewIndex(identity);
});

test('a failed read is retried instead of being remembered as an answer', async () => {
  invalidateReviewIndex(identity);
  let attempts = 0;
  const failing = async () => {
    attempts += 1;
    throw new Error('读取失败 / read failed');
  };
  await assert.rejects(() => reviewEntriesFor(identity, failing), /读取失败/);
  await tick();
  await assert.rejects(() => reviewEntriesFor(identity, failing), /读取失败/);
  assert.equal(attempts, 2);
  const recovered = await reviewEntriesFor(identity, async (bucket) =>
    page(bucket === 'due' ? [entry()] : []),
  );
  assert.equal(recovered.length, 1);
  invalidateReviewIndex(identity);
});
