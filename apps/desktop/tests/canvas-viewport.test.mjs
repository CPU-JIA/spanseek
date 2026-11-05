import test from 'node:test';
import assert from 'node:assert/strict';
import {
  canvasViewportKey,
  canvasViewportPreference,
  readCanvasViewport,
  restoreCanvasViewport,
} from '../src/organization/canvas-viewport.ts';

test('a saved canvas camera keeps its world center and zoom across viewport sizes', () => {
  const original = { x: -210, y: 82, scale: 1.2 };
  const preference = canvasViewportPreference(original, 960, 720);
  const saved = readCanvasViewport({ getItem: () => JSON.stringify(preference) }, 'key');
  assert.deepEqual(restoreCanvasViewport(saved, 960, 720), original);
  const smaller = restoreCanvasViewport(saved, 360, 270);
  assert.deepEqual(canvasViewportPreference(smaller, 360, 270), preference);
});

test('restored copies and different canvases cannot adopt another local camera', () => {
  const identity = { local_instance_id: 'local-a', workspace_id: 'workspace-a' };
  const key = canvasViewportKey(identity, 'canvas-a');
  assert.notEqual(
    key,
    canvasViewportKey({ ...identity, local_instance_id: 'restored' }, 'canvas-a'),
  );
  assert.notEqual(key, canvasViewportKey(identity, 'canvas-b'));
  assert.equal(readCanvasViewport({ getItem: () => null }, key), null);
});

test('invalid or unreadable camera preferences are surfaced rather than used as coordinates', () => {
  for (const value of [
    'bad JSON',
    '{}',
    'null',
    JSON.stringify({ schema: 2, centerX: 0, centerY: 0, scale: 1 }),
    JSON.stringify({ schema: 1, centerX: 0, centerY: 0, scale: 0 }),
    JSON.stringify({ schema: 1, centerX: 9e99, centerY: 0, scale: 1 }),
  ]) {
    assert.throws(() => readCanvasViewport({ getItem: () => value }, 'key'));
  }
  assert.throws(
    () =>
      readCanvasViewport(
        {
          getItem: () => {
            throw new Error('storage denied');
          },
        },
        'key',
      ),
    /storage denied/,
  );
});
