import test from 'node:test';
import assert from 'node:assert/strict';
import { chatUsageParts } from '../src/inference/ai-usage-format.ts';

test('chat usage formats provider token counts without inventing missing values', () => {
  assert.deepEqual(chatUsageParts({ input_tokens: 120n, output_tokens: 30n }), {
    input: 120,
    output: 30,
    total: 150,
  });
  assert.deepEqual(chatUsageParts({ input_tokens: null, output_tokens: 30n }), {
    input: null,
    output: 30,
    total: 30,
  });
  assert.deepEqual(chatUsageParts({ input_tokens: null, output_tokens: null }), {
    input: null,
    output: null,
    total: null,
  });
});
