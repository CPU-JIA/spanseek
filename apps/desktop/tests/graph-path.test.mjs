import test from 'node:test';
import assert from 'node:assert/strict';
import { graphPathSegments, shortestGraphPath } from '../src/organization/graph-path.ts';

const nodes = ['a', 'b', 'c', 'd', 'e'].map((id) => ({ id }));
const edges = [
  { source_id: 'a', target_id: 'b' },
  { source_id: 'a', target_id: 'c' },
  { source_id: 'b', target_id: 'd' },
  { source_id: 'c', target_id: 'd' },
  { source_id: 'd', target_id: 'e' },
];

test('shortest path is deterministic and works in both edge directions', () => {
  assert.deepEqual(shortestGraphPath(nodes, edges, 'e', 'a'), ['e', 'd', 'b', 'a']);
  assert.deepEqual(shortestGraphPath(nodes, edges, 'a', 'e'), ['a', 'b', 'd', 'e']);
  assert.deepEqual(shortestGraphPath(nodes, edges, 'b', 'b'), ['b']);
});

test('missing endpoints or disconnected nodes return null instead of a partial path', () => {
  assert.equal(shortestGraphPath(nodes, edges, 'a', 'missing'), null);
  assert.equal(shortestGraphPath(nodes, edges, null, 'a'), null);
  assert.equal(shortestGraphPath(nodes, [{ source_id: 'a', target_id: 'b' }], 'a', 'e'), null);
});

test('filtered nodes and self edges are ignored', () => {
  assert.deepEqual(
    shortestGraphPath(
      [{ id: 'a' }, { id: 'b' }],
      [
        { source_id: 'a', target_id: 'a' },
        { source_id: 'a', target_id: 'b' },
        { source_id: 'b', target_id: 'c' },
      ],
      'a',
      'b',
    ),
    ['a', 'b'],
  );
});

test('path segments are canonical for canvas highlighting', () => {
  assert.deepEqual(graphPathSegments(['b', 'a', 'c']), ['a\u0000b', 'a\u0000c']);
  assert.deepEqual(graphPathSegments(['a']), []);
});

test('ten thousand node chain stays bounded and reachable', () => {
  const chainNodes = Array.from({ length: 10_000 }, (_, index) => ({ id: `n${index}` }));
  const chainEdges = Array.from({ length: 9_999 }, (_, index) => ({
    source_id: `n${index}`,
    target_id: `n${index + 1}`,
  }));
  const path = shortestGraphPath(chainNodes, chainEdges, 'n0', 'n9999');
  assert.equal(path?.length, 10_000);
  assert.equal(path?.[0], 'n0');
  assert.equal(path?.at(-1), 'n9999');
});
