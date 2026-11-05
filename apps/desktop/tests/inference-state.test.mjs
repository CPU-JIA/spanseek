import test from 'node:test';
import assert from 'node:assert/strict';
import { latestAnswer, latestRebuild } from '../src/inference/inference-state.ts';
test('an older answer snapshot cannot replace a confirmed cancel or completion', () => {
  const canceled = { id: 'answer', last_event_seq: 8, status: 'canceled' };
  assert.equal(
    latestAnswer(canceled, { id: 'answer', last_event_seq: 7, status: 'running' }),
    canceled,
  );
  const other = { id: 'other', last_event_seq: 1, status: 'queued' };
  assert.equal(latestAnswer(canceled, other), other);
});
test('a delayed rebuild poll cannot revive a canceled job or regress progress', () => {
  const canceled = { id: 'job', status: 'canceled', updated_at: '2026-10-03T01:00:00Z' };
  assert.equal(latestRebuild(canceled, { ...canceled, status: 'running' }), canceled);
  const current = { ...canceled, status: 'running', processed: 10 };
  assert.equal(
    latestRebuild(current, { ...current, updated_at: '2026-10-03T00:00:00Z', processed: 9 }),
    current,
  );
});
