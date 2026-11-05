import test from 'node:test';
import assert from 'node:assert/strict';
import { rerankSearchResults } from '../src/inference/search-rerank.ts';

function item(id, lexical, cosine, fusion) {
  return {
    id,
    target: { kind: 'note' },
    title: id,
    text: id,
    lexical_rank: lexical,
    cosine_similarity: cosine,
    fusion_rank: fusion,
    generation_id: null,
    annotation: null,
  };
}

test('fusion order follows the hybrid score and falls back to lexical rank', () => {
  const items = [item('a', 2, 0.2, 0.01), item('b', 1, 0.4, 0.03), item('c', 3, 0.9, null)];
  assert.deepEqual(
    rerankSearchResults(items, 'fusion').map((value) => value.id),
    ['b', 'a', 'c'],
  );
});

test('keyword order keeps items without lexical ranks after ranked items', () => {
  const items = [
    item('missing', null, 0.99, 0.9),
    item('second', 2, 0.1, 0.05),
    item('first', 1, 0.2, 0.04),
  ];
  assert.deepEqual(
    rerankSearchResults(items, 'keyword').map((value) => value.id),
    ['first', 'second', 'missing'],
  );
});

test('semantic order keeps items without cosine scores after scored items', () => {
  const items = [
    item('missing', 1, null, 0.9),
    item('lower', 3, 0.2, 0.05),
    item('higher', 2, 0.8, 0.04),
  ];
  assert.deepEqual(
    rerankSearchResults(items, 'semantic').map((value) => value.id),
    ['higher', 'lower', 'missing'],
  );
});

test('re-ranking is deterministic and does not mutate the source array', () => {
  const items = [item('b', 2, 0.1, 0.01), item('a', 1, 0.2, 0.02)];
  const ordered = rerankSearchResults(items, 'keyword');
  assert.deepEqual(
    items.map((value) => value.id),
    ['b', 'a'],
  );
  assert.deepEqual(
    ordered.map((value) => value.id),
    ['a', 'b'],
  );
  assert.deepEqual(rerankSearchResults(items, 'keyword'), ordered);
});
