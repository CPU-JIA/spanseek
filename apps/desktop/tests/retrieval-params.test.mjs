import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_RETRIEVAL_PARAMS,
  MAX_ANSWER_TIMEOUT_SECONDS,
  MAX_CITATION_COUNT,
  MAX_COST_PER_MILLION,
  MAX_TOP_K,
  MIN_ANSWER_TIMEOUT_SECONDS,
  clampAnswerTimeout,
  clampParams,
  clampPrice,
  minimumCosineEnabled,
  suggestedCosine,
} from '../src/inference/retrieval-params.ts';

const base = {
  semantic_enabled: true,
  remote_consent: false,
  cost_per_million_input: null,
  cost_per_million_output: null,
  answer_timeout_seconds: null,
};

test('defaults match the core contract', () => {
  assert.deepEqual(DEFAULT_RETRIEVAL_PARAMS, {
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    ...base,
  });
  assert.equal(MAX_TOP_K, 50);
  assert.equal(MAX_CITATION_COUNT, 20);
});

test('clampParams keeps every field inside the accepted range', () => {
  assert.deepEqual(clampParams({ top_k: 0, citation_count: 0, minimum_cosine: null, ...base }), {
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    ...base,
  });
  assert.deepEqual(clampParams({ top_k: 999, citation_count: 999, minimum_cosine: 5, ...base }), {
    top_k: MAX_TOP_K,
    citation_count: MAX_CITATION_COUNT,
    minimum_cosine: 1,
    ...base,
  });
  assert.deepEqual(clampParams({ top_k: 3, citation_count: 9, minimum_cosine: -4, ...base }), {
    top_k: 3,
    citation_count: 3,
    minimum_cosine: -1,
    ...base,
  });
  assert.deepEqual(
    clampParams({ top_k: 12.6, citation_count: 4.4, minimum_cosine: 0.325, ...base }),
    {
      top_k: 13,
      citation_count: 4,
      minimum_cosine: 0.325,
      ...base,
    },
  );
  assert.equal(
    clampParams({ top_k: 8, citation_count: 5, minimum_cosine: Number.NaN, ...base })
      .minimum_cosine,
    null,
  );
  assert.equal(
    clampParams({ top_k: 8, citation_count: 5, minimum_cosine: undefined, ...base }).minimum_cosine,
    null,
  );
});

test('clampParams carries the library-level switches without inventing consent', () => {
  const off = clampParams({
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    semantic_enabled: false,
    remote_consent: true,
  });
  assert.equal(off.semantic_enabled, false);
  assert.equal(off.remote_consent, true);
  const repaired = clampParams({
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    semantic_enabled: undefined,
    remote_consent: undefined,
  });
  assert.equal(repaired.semantic_enabled, true);
  assert.equal(repaired.remote_consent, false);
});

test('a reference price is either usable or absent, never invented', () => {
  assert.equal(clampPrice(null), null);
  assert.equal(clampPrice(undefined), null);
  assert.equal(clampPrice(Number.NaN), null);
  assert.equal(clampPrice(Number.POSITIVE_INFINITY), null);
  assert.equal(clampPrice(-5), 0);
  assert.equal(clampPrice(0), 0);
  assert.equal(clampPrice(0.5), 0.5);
  assert.equal(clampPrice(MAX_COST_PER_MILLION + 1), MAX_COST_PER_MILLION);
  const kept = clampParams({
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    semantic_enabled: true,
    remote_consent: false,
    cost_per_million_input: 1.25,
    cost_per_million_output: -1,
  });
  assert.equal(kept.cost_per_million_input, 1.25);
  assert.equal(kept.cost_per_million_output, 0);
});

test('the reference similarity floor follows the embedding profile', () => {
  assert.equal(suggestedCosine('@cf/baai/bge-m3', 1024), 0.35);
  assert.equal(suggestedCosine('text-embedding-3-small', 1536), 0.3);
  assert.equal(suggestedCosine('text-embedding-3-large', 3072), 0.3);
  assert.equal(suggestedCosine('text-embedding-004', 768), 0.4);
  assert.equal(suggestedCosine('gemini-embedding-001', 3072), 0.35);
  assert.equal(suggestedCosine('unknown-model', 2048), 0.35);
  assert.equal(suggestedCosine(null, null), 0.35);
});

test('an answer budget is either usable or absent, never invented', () => {
  assert.equal(MIN_ANSWER_TIMEOUT_SECONDS, 10);
  assert.equal(MAX_ANSWER_TIMEOUT_SECONDS, 3600);
  assert.equal(clampAnswerTimeout(null), null);
  assert.equal(clampAnswerTimeout(undefined), null);
  assert.equal(clampAnswerTimeout(Number.NaN), null);
  assert.equal(clampAnswerTimeout(Number.POSITIVE_INFINITY), null);
  assert.equal(clampAnswerTimeout(0), MIN_ANSWER_TIMEOUT_SECONDS);
  assert.equal(clampAnswerTimeout(1.4), MIN_ANSWER_TIMEOUT_SECONDS);
  assert.equal(clampAnswerTimeout(180), 180);
  assert.equal(clampAnswerTimeout(3601), MAX_ANSWER_TIMEOUT_SECONDS);
  const kept = clampParams({
    top_k: 8,
    citation_count: 5,
    minimum_cosine: null,
    ...base,
    answer_timeout_seconds: 240,
  });
  assert.equal(kept.answer_timeout_seconds, 240);
  const absent = clampParams({ top_k: 8, citation_count: 5, minimum_cosine: null, ...base });
  assert.equal(absent.answer_timeout_seconds, null);
});
test('a similarity floor the core omits reads as off, never as an enabled filter', () => {
  // The core omits `minimum_cosine` when it is unset, so the interface sees
  // `undefined` next to `null`; showing either as "enabled" would claim a filter
  // that is not applied.
  assert.equal(minimumCosineEnabled(undefined), false);
  assert.equal(minimumCosineEnabled(null), false);
  assert.equal(minimumCosineEnabled(0), true);
  assert.equal(minimumCosineEnabled(0.35), true);
  assert.equal(minimumCosineEnabled(-1), true);
});
