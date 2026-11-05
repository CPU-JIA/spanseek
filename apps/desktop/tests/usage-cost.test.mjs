import assert from 'node:assert/strict';
import test from 'node:test';
import { estimateCost } from '../src/diagnostics/usage-cost.ts';

const usage = {
  calls: 3,
  estimated_calls: 1,
  calls_without_usage: 0,
  input_tokens: 2_000_000,
  output_tokens: 500_000,
};

test('no reference price means no estimate instead of a fabricated one', () => {
  assert.equal(estimateCost(usage, { input: null, output: null }), null);
});

test('an estimate uses only the prices the user entered', () => {
  assert.equal(estimateCost(usage, { input: 1, output: null }), 2);
  assert.equal(estimateCost(usage, { input: null, output: 2 }), 1);
  assert.equal(estimateCost(usage, { input: 1, output: 2 }), 3);
  assert.equal(
    estimateCost({ ...usage, input_tokens: 0, output_tokens: 0 }, { input: 1, output: 1 }),
    0,
  );
});
