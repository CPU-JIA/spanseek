import test from 'node:test';
import assert from 'node:assert/strict';
import {
  canvasBounds,
  fitCanvasViewport,
  initialCanvasViewport,
  visibleCanvasNodes,
} from '../src/organization/canvas-geometry.ts';

function nodes(count) {
  return Array.from({ length: count }, (_, index) => ({
    note_id: `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
    x: (index % 40) * 280,
    y: Math.floor(index / 40) * 190,
    width: 250,
    height: 148,
    z_index: index,
  }));
}

test('canvas viewport culls 100 and 1,000 node layouts without dropping the world bounds', () => {
  for (const count of [100, 1000]) {
    const all = nodes(count);
    const visible = visibleCanvasNodes(all, { x: 0, y: 0, scale: 1, width: 900, height: 620 });
    assert.ok(visible.length > 0);
    assert.ok(visible.length < count);
    assert.equal(new Set(visible.map((node) => node.note_id)).size, visible.length);
    const bounds = canvasBounds(all);
    assert.equal(bounds.x, 0);
    assert.equal(bounds.y, 0);
    assert.ok(bounds.width >= 250);
    assert.ok(bounds.height >= 148);
  }
});

test('fit keeps a 1,000 node canvas inside the viewport at a readable scale', () => {
  const fitted = fitCanvasViewport(nodes(1000), 1200, 800, 80);
  assert.ok(fitted.scale >= 0.25 && fitted.scale <= 1.5);
  assert.ok(Number.isFinite(fitted.x) && Number.isFinite(fitted.y));
});

test('small canvases open at a readable scale while fit remains available', () => {
  const small = initialCanvasViewport(nodes(15), 900, 620);
  assert.equal(small.scale, 0.8);
  assert.equal(small.x, 32);
  assert.equal(small.y, 32);

  const single = nodes(1);
  const fitted = fitCanvasViewport(single, 900, 620);
  const initial = initialCanvasViewport(single, 900, 620);
  assert.deepEqual(initial, fitted);
});

test('large canvases still open fitted for orientation', () => {
  const large = nodes(1000);
  const fitted = fitCanvasViewport(large, 1200, 800);
  const initial = initialCanvasViewport(large, 1200, 800);
  assert.deepEqual(initial, fitted);
});
