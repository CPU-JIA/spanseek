import test from 'node:test';
import assert from 'node:assert/strict';
import {
  WORKSPACE_CANCEL_SETTLE_MS,
  WORKSPACE_OPERATION_MAX_MS,
  WORKSPACE_OPERATION_POLL_MS,
  operationWaitDeadline,
} from '../src/app/operation-wait.ts';

test('workspace polling has a finite default deadline', () => {
  assert.ok(WORKSPACE_OPERATION_POLL_MS > 0);
  assert.ok(WORKSPACE_OPERATION_MAX_MS > 0);
  assert.equal(operationWaitDeadline(1_000, null), 1_000 + WORKSPACE_OPERATION_MAX_MS);
});

test('a cancellation request shortens the remaining wait to a visible settle budget', () => {
  assert.equal(WORKSPACE_CANCEL_SETTLE_MS, 15_000);
  assert.equal(operationWaitDeadline(1_000, 20_000), 35_000);
  assert.ok(operationWaitDeadline(1_000, 20_000) < operationWaitDeadline(1_000, null));
});
