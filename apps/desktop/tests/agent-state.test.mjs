import test from 'node:test';
import assert from 'node:assert/strict';
import { latestAgentProgress } from '../src/inference/agent-state.ts';

const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
const progress = {
  answer_id: 'answer',
  stage: 'searching',
  model_calls: 1,
  tool_calls: 1,
  evidence_count: 2,
  elapsed_ms: 1000,
};

test('agent events cannot cross answer, workspace or restored-instance boundaries', () => {
  for (const [scope, answer] of [
    [{ ...identity, workspace_id: 'other' }, 'answer'],
    [{ ...identity, local_instance_id: 'restored' }, 'answer'],
    [identity, 'other-answer'],
    [identity, null],
  ])
    assert.equal(latestAgentProgress(null, progress, identity, scope, answer), null);
  assert.equal(latestAgentProgress(null, progress, identity, identity, 'answer'), progress);
});

test('delayed progress cannot roll back time or cumulative budgets', () => {
  for (const incoming of [
    { ...progress, elapsed_ms: 900 },
    { ...progress, elapsed_ms: 1100, model_calls: 0 },
    { ...progress, elapsed_ms: 1100, tool_calls: 0 },
  ])
    assert.equal(latestAgentProgress(progress, incoming, identity, identity, 'answer'), progress);
  const next = { ...progress, stage: 'generating', model_calls: 2, elapsed_ms: 2000 };
  assert.equal(latestAgentProgress(progress, next, identity, identity, 'answer'), next);
});
