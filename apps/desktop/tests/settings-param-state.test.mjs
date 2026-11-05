import assert from 'node:assert/strict';
import test from 'node:test';
import { cleaningParamsEqual, retrievalParamsEqual } from '../src/settings-param-state.ts';

const retrieval = {
  top_k: 8,
  citation_count: 5,
  minimum_cosine: null,
  semantic_enabled: true,
  remote_consent: false,
  cost_per_million_input: null,
  cost_per_million_output: null,
  answer_timeout_seconds: null,
};

test('retrieval settings compare effective values, not object identity', () => {
  assert.equal(retrievalParamsEqual(retrieval, { ...retrieval }), true);
  assert.equal(
    retrievalParamsEqual(retrieval, {
      ...retrieval,
      minimum_cosine: undefined,
      answer_timeout_seconds: undefined,
      cost_per_million_input: undefined,
      cost_per_million_output: undefined,
      semantic_enabled: undefined,
    }),
    true,
  );
  assert.equal(retrievalParamsEqual(retrieval, { ...retrieval, top_k: 9 }), false);
  assert.equal(retrievalParamsEqual(retrieval, { ...retrieval, minimum_cosine: 0.35 }), false);
  assert.equal(retrievalParamsEqual(retrieval, { ...retrieval, semantic_enabled: false }), false);
  assert.equal(retrievalParamsEqual(null, retrieval), false);
});

test('cleaning settings compare all three toggles', () => {
  const cleaning = { headers_footers: true, watermark: true, control_chars: true };
  assert.equal(cleaningParamsEqual(cleaning, { ...cleaning }), true);
  assert.equal(cleaningParamsEqual(cleaning, { ...cleaning, watermark: false }), false);
  assert.equal(cleaningParamsEqual(undefined, cleaning), false);
});
