import assert from 'node:assert/strict';
import test from 'node:test';
import { fitGraphViewport, layoutKnowledgeGraph } from '../src/organization/graph-layout.ts';

test('fit includes manually moved nodes outside the original layout, at any viewport size', () => {
  const nodes = [
    { id: 'left', x: -1800, y: -650, radius: 20 },
    { id: 'right', x: 7000, y: 4100, radius: 40 },
  ];
  for (const [width, height] of [
    [1120, 780],
    [320, 200],
  ]) {
    const viewport = fitGraphViewport(nodes, width, height);
    assert.ok(viewport.k > 0 && viewport.k < 1);
    for (const node of nodes) {
      assert.ok((node.x - node.radius) * viewport.k + viewport.x >= 0);
      assert.ok((node.x + node.radius) * viewport.k + viewport.x <= width);
      assert.ok((node.y - node.radius) * viewport.k + viewport.y >= 0);
      assert.ok((node.y + node.radius) * viewport.k + viewport.y <= height);
    }
  }
  assert.deepEqual(fitGraphViewport([], 800, 600), { x: 0, y: 0, k: 1 });
});

test('graph layout is deterministic, finite and stays inside the canvas', () => {
  const nodes = Array.from({ length: 80 }, (_, index) => ({
    id: `note-${index}`,
    degree: index % 5,
  }));
  const edges = Array.from({ length: 110 }, (_, index) => ({
    source: `note-${index % 80}`,
    target: `note-${(index * 7 + 11) % 80}`,
    weight: (index % 3) + 1,
  }));
  const first = layoutKnowledgeGraph(nodes, edges, 900, 620);
  const second = layoutKnowledgeGraph(nodes, edges, 900, 620);
  assert.deepEqual(first, second);
  assert.equal(first.length, nodes.length);
  for (const node of first) {
    assert.ok(Number.isFinite(node.x) && Number.isFinite(node.y));
    assert.ok(node.x >= 0 && node.x <= 900);
    assert.ok(node.y >= 0 && node.y <= 620);
  }
});

test('graph layout handles an empty or disconnected graph', () => {
  assert.deepEqual(layoutKnowledgeGraph([], [], 640, 480), []);
  const nodes = Array.from({ length: 12 }, (_, index) => ({ id: `orphan-${index}`, degree: 0 }));
  const points = layoutKnowledgeGraph(nodes, [], 640, 480);
  assert.equal(points.length, 12);
  assert.ok(points.every((point) => Number.isFinite(point.x) && Number.isFinite(point.y)));
});

test('ten thousand nodes and fifty thousand links complete within the bounded layout budget', () => {
  const nodes = Array.from({ length: 10_000 }, (_, index) => ({
    id: `scale-${index}`,
    degree: 10,
  }));
  const edges = Array.from({ length: 50_000 }, (_, index) => ({
    source: `scale-${index % 10_000}`,
    target: `scale-${(index * 97 + 13) % 10_000}`,
    weight: (index % 4) + 1,
  }));
  const started = performance.now();
  const points = layoutKnowledgeGraph(nodes, edges, 1600, 1000);
  const elapsed = performance.now() - started;
  assert.equal(points.length, 10_000);
  assert.ok(points.every((point) => Number.isFinite(point.x) && Number.isFinite(point.y)));
  assert.ok(elapsed < 15_000, `layout took ${elapsed.toFixed(1)}ms`);
});
