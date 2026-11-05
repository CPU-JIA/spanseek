import test from 'node:test';
import assert from 'node:assert/strict';
import { ReadingQueue } from '../src/sources/reading-queue.ts';
const location = (version, text) => ({
  kind: 'text',
  source_version_id: version,
  block_id: 'block',
  start_codepoint: 0,
  end_codepoint: text.length,
  quote: text,
  prefix: '',
  suffix: '',
});
test('close barrier drains latest position received while an earlier save is pending', async () => {
  let state = { locator: null, row_version: 0 },
    release;
  const saves = [];
  const queue = new ReadingQueue(
    async () => state,
    async (current, locator) => {
      saves.push([current.row_version, locator.quote]);
      if (saves.length === 1) await new Promise((resolve) => (release = resolve));
      state = { locator, row_version: current.row_version + 1 };
    },
  );
  queue.activate('pane', 'v');
  queue.record('pane', location('v', 'first'));
  const barrier = queue.flush();
  await new Promise((resolve) => setImmediate(resolve));
  queue.record('pane', location('v', 'last'));
  release();
  await barrier;
  assert.deepEqual(saves, [
    [0, 'first'],
    [1, 'last'],
  ]);
  assert.equal(queue.hasPending(), false);
});
test('old pane and old version cannot overwrite foreground position', async () => {
  const saved = [];
  const queue = new ReadingQueue(
    async () => ({ locator: null, row_version: 0 }),
    async (_, locator) => saved.push(locator),
  );
  queue.activate('old-pane', 'v1');
  queue.activate('foreground', 'v2');
  queue.record('old-pane', location('v1', 'stale'));
  queue.record('foreground', location('v1', 'stale version'));
  queue.record('foreground', location('v2', 'current'));
  await queue.flush();
  assert.equal(saved.length, 1);
  assert.equal(saved[0].quote, 'current');
});
test('failed save keeps the exact latest location for a later close retry', async () => {
  let fail = true;
  const saved = [];
  const queue = new ReadingQueue(
    async () => ({ locator: null, row_version: 4 }),
    async (_, locator) => {
      if (fail) throw new Error('disk full');
      saved.push(locator);
    },
  );
  queue.activate('pane', 'v');
  queue.record('pane', location('v', 'keep'));
  await assert.rejects(queue.flush(), /disk full/);
  assert.equal(queue.hasPending(), true);
  fail = false;
  await queue.flush();
  assert.equal(saved[0].quote, 'keep');
  assert.equal(queue.hasPending(), false);
});

test('close during a settled drain waits for a position recorded before flight cleanup', async () => {
  let state = { locator: null, row_version: 0 };
  let closing;
  const saves = [];
  const queue = new ReadingQueue(
    async () => state,
    async (current, locator) => {
      saves.push(locator.quote);
      state = { locator, row_version: current.row_version + 1 };
      if (saves.length === 1)
        queueMicrotask(() =>
          queueMicrotask(() => {
            queue.record('pane', location('v', 'last instant'));
            closing = queue.flush();
          }),
        );
    },
  );
  queue.activate('pane', 'v');
  queue.record('pane', location('v', 'first'));
  await queue.flush();
  await closing;
  assert.deepEqual(saves, ['first', 'last instant']);
  assert.equal(queue.hasPending(), false);
});

test('background mounts cannot steal ownership and unmount retains the last position', async () => {
  const saved = [];
  const queue = new ReadingQueue(
    async () => ({ locator: null, row_version: 0 }),
    async (_, locator) => saved.push(locator.quote),
  );
  queue.register('foreground', 'v1');
  queue.record('foreground', location('v1', 'foreground position'));
  queue.register('background', 'v2');
  queue.record('background', location('v2', 'background initial render'));
  queue.release('background');
  queue.record('foreground', location('v1', 'last foreground position'));
  queue.release('foreground');
  await queue.flush();
  assert.deepEqual(saved, ['last foreground position']);
  queue.register('background', 'v2');
  queue.record('background', location('v2', 'new foreground position'));
  await queue.flush();
  assert.deepEqual(saved, ['last foreground position', 'new foreground position']);
});
