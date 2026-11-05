import test from 'node:test';
import assert from 'node:assert/strict';
import { latestSourceJob } from '../src/portability/source-task-state.ts';
const job = (status, attempt = 1, updated_at = '2026-10-03T00:00:00Z') => ({
  id: 'job',
  status,
  attempt,
  updated_at,
});
test('a delayed running poll cannot undo a confirmed cancellation', () => {
  const canceled = job('canceled');
  assert.equal(latestSourceJob(canceled, job('running')), canceled);
});
test('older attempts and checkpoints cannot overwrite newer worker progress', () => {
  const newer = job('running', 2, '2026-10-03T00:01:00Z');
  assert.equal(latestSourceJob(newer, job('failed', 1, '2026-10-03T00:02:00Z')), newer);
  assert.equal(latestSourceJob(newer, job('running', 2)), newer);
});
test('a new attempt and a confirmed terminal result are accepted', () => {
  const newer = job('running', 2);
  assert.equal(latestSourceJob(job('failed'), newer), newer);
  const completed = job('succeeded', 2);
  assert.equal(latestSourceJob(newer, completed), completed);
});
