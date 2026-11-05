import test from 'node:test';
import assert from 'node:assert/strict';
import { WindowZoom, nextZoom } from '../src/layout/window-zoom.ts';

test('zoom steps, wheel units, reverse direction and reset stay within bounds', async () => {
  const writes = [],
    zoom = new WindowZoom(async (value) => {
      writes.push(value);
    });
  await zoom.reset();
  await zoom.wheel(-40, 0);
  await zoom.wheel(-60, 0);
  await zoom.wheel(3, 1);
  await zoom.wheel(-1, 2);
  await zoom.reset();
  assert.deepEqual(writes, [1, 1.1, 1, 1.1, 1]);
  assert.equal(nextZoom(3, 1), 3);
  assert.equal(nextZoom(0.5, -1), 0.5);
});

test('rapid input coalesces and never applies native zoom out of order', async () => {
  let release;
  const writes = [],
    gate = new Promise((resolve) => {
      release = resolve;
    });
  const zoom = new WindowZoom(async (value) => {
    writes.push(value);
    if (writes.length === 1) await gate;
  });
  const first = zoom.step(1);
  await Promise.resolve();
  zoom.step(1);
  zoom.step(1);
  zoom.step(-1);
  release();
  await first;
  assert.deepEqual(writes, [1.1, 1.25]);
});

test('native failure is reported and retry starts from the last applied scale', async () => {
  const writes = [];
  let fail = false;
  const zoom = new WindowZoom(async (value) => {
    if (fail) throw Error('native failure');
    writes.push(value);
  });
  await zoom.reset();
  fail = true;
  await assert.rejects(zoom.step(1), /native failure/);
  fail = false;
  await zoom.step(1);
  assert.deepEqual(writes, [1, 1.1]);
});
