import test from 'node:test';
import assert from 'node:assert/strict';
import { latestChatProgress } from '../src/inference/chat-progress.ts';
const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
const initial = {
  run_id: 'run',
  stage: 'searching',
  model_calls: 1,
  tool_calls: 1,
  elapsed_ms: 200,
};
test('chat activity is isolated by run and physical workspace, and cannot revive a stopped run', () => {
  assert.equal(latestChatProgress(null, initial, identity, identity, 'run', 'running'), initial);
  for (const [eventIdentity, runId, status] of [
    [{ ...identity, local_instance_id: 'copy' }, 'run', 'running'],
    [{ ...identity, workspace_id: 'other' }, 'run', 'running'],
    [identity, 'old-run', 'running'],
    [identity, 'run', 'canceled'],
    [identity, 'run', 'completed'],
  ])
    assert.equal(latestChatProgress(null, initial, identity, eventIdentity, runId, status), null);
});
test('late chat activity cannot roll back cumulative calls or elapsed time', () => {
  for (const incoming of [
    { ...initial, model_calls: 0 },
    { ...initial, tool_calls: 0 },
    { ...initial, elapsed_ms: 100 },
  ])
    assert.equal(
      latestChatProgress(initial, incoming, identity, identity, 'run', 'running'),
      initial,
    );
  const next = { ...initial, stage: 'reading', tool_calls: 2, elapsed_ms: 350 };
  assert.equal(latestChatProgress(initial, next, identity, identity, 'run', 'running'), next);
});
