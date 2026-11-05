import test from 'node:test';
import assert from 'node:assert/strict';
import { resolvePaneIndex } from '../src/layout/pane-target.ts';

test('a requested pane is kept while it still exists', () => {
  assert.equal(resolvePaneIndex(2, 1, 0), 1);
  assert.equal(resolvePaneIndex(2, 0, 1), 0);
});

test('stale pane requests fall back to the active pane instead of claiming it is closed', () => {
  assert.equal(resolvePaneIndex(1, 1, 0), 0);
  assert.equal(resolvePaneIndex(1, 99, 0), 0);
  assert.equal(resolvePaneIndex(2, 4, 1), 1);
});

test('a stale active pane also falls back to the first pane', () => {
  assert.equal(resolvePaneIndex(1, undefined, 3), 0);
  assert.equal(resolvePaneIndex(1, null, null), 0);
});

test('no navigation target exists without a pane', () => {
  assert.equal(resolvePaneIndex(0, 0, 0), null);
  assert.equal(resolvePaneIndex(-1, 0, 0), null);
  assert.equal(resolvePaneIndex(Number.NaN, 0, 0), null);
});
