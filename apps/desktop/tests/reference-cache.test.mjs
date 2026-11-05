import assert from 'node:assert/strict';
import test from 'node:test';
import { REFERENCE_CACHE_LIMIT, ReferenceCache } from '../src/editor/reference-cache.ts';

const clock = (start = 1_000) => {
  let now = start;
  return { now: () => now, advance: (ms) => (now += ms) };
};

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};

/** Values that never settle the same way as the loader that produced them. */
const loader = (value) => () => Promise.resolve(value);

/**
 * The implementation this replaced kept only in-flight promises: the entry was
 * deleted as soon as the request settled, so every later re-resolution asked
 * again. Kept here as the before side of the observation below.
 */
function inFlightOnlyMap() {
  const requests = new Map();
  let calls = 0;
  return {
    get calls() {
      return calls;
    },
    resolve(key, load) {
      const existing = requests.get(key);
      if (existing) return existing;
      calls++;
      const request = load().then(
        (value) => {
          requests.delete(key);
          return value;
        },
        (error) => {
          requests.delete(key);
          throw error;
        },
      );
      requests.set(key, request);
      return request;
    },
  };
}

test('callers that ask at the same time share one request for the same target', async () => {
  const cache = new ReferenceCache();
  let loads = 0;
  const load = () => {
    loads++;
    return Promise.resolve({ title: 'shared' });
  };
  const [first, second] = await Promise.all([
    cache.resolve('note-1', load),
    cache.resolve('note-1', load),
  ]);
  assert.equal(loads, 1);
  assert.equal(first, second);
  assert.equal(cache.size, 1);
});

test('a save no longer re-asks every reference in the note (in-flight-only map asked 4x more)', async () => {
  const CHIPS = 12;
  const SAVES = 3;
  const targets = Array.from({ length: CHIPS }, (_, index) => `note-${index + 1}`);
  const cache = new ReferenceCache();
  const legacy = inFlightOnlyMap();
  let cachedCalls = 0;
  let legacyCalls = 0;

  // Each pass is one pane generation: mounting the note, then one commit per save.
  const mount = async (resolve, count) => {
    await Promise.all(
      targets.map((target) => resolve(target, () => (count(), Promise.resolve(target)))),
    );
  };
  for (let pass = 0; pass <= SAVES; pass++) {
    await mount(
      (key, load) => cache.resolve(key, load),
      () => cachedCalls++,
    );
    await mount(
      (key, load) => legacy.resolve(key, load),
      () => legacyCalls++,
    );
  }

  assert.equal(cachedCalls, CHIPS);
  assert.equal(legacyCalls, CHIPS * (SAVES + 1));
  assert.equal(legacyCalls / cachedCalls, SAVES + 1);
});

test('a remembered answer is reused inside the TTL and asked again after it', async () => {
  const time = clock();
  const cache = new ReferenceCache({ ttlMs: 500, now: time.now });
  let loads = 0;
  const load = (value) => () => {
    loads++;
    return Promise.resolve(value);
  };

  assert.equal(await cache.resolve('note-1', load('first')), 'first');
  time.advance(499);
  assert.equal(await cache.resolve('note-1', load('second')), 'first');
  assert.equal(loads, 1);
  time.advance(2);
  assert.equal(await cache.resolve('note-1', load('second')), 'second');
  assert.equal(loads, 2);
});

test('an expired answer is neither returned nor kept', async () => {
  const time = clock();
  const cache = new ReferenceCache({ ttlMs: 10, now: time.now });
  await cache.resolve('note-1', loader('value'));
  assert.equal(cache.peek('note-1'), 'value');
  time.advance(11);
  assert.equal(cache.peek('note-1'), null);
  assert.equal(cache.size, 0);
});

test('a failure is never remembered, so the next resolve really asks again', async () => {
  const cache = new ReferenceCache();
  let attempts = 0;
  const load = () => {
    attempts++;
    return attempts === 1 ? Promise.reject(new Error('offline')) : Promise.resolve('recovered');
  };
  await assert.rejects(cache.resolve('note-1', load), /offline/);
  assert.equal(cache.peek('note-1'), null);
  assert.equal(await cache.resolve('note-1', load), 'recovered');
  assert.equal(attempts, 2);
});

test('fresh asks again and replaces what was remembered', async () => {
  const cache = new ReferenceCache();
  await cache.resolve('note-1', loader('remembered'));
  assert.equal(await cache.resolve('note-1', loader('latest'), { fresh: true }), 'latest');
  assert.equal(cache.peek('note-1'), 'latest');
  assert.equal(
    await cache.resolve('note-1', () => Promise.reject(new Error('must not load'))),
    'latest',
  );
});

test('a slow older answer cannot replace a newer fresh one', async () => {
  const cache = new ReferenceCache();
  const slow = deferred();
  const older = cache.resolve('note-1', () => slow.promise);
  const newer = cache.resolve('note-1', loader('new'), { fresh: true });
  assert.equal(await newer, 'new');
  slow.resolve('old');
  assert.equal(await older, 'old');
  assert.equal(cache.peek('note-1'), 'new');
});

test('invalidating drops one target, ignores a pending answer and can clear everything', async () => {
  const cache = new ReferenceCache();
  await cache.resolve('note-1', loader('a'));
  cache.invalidate('note-1');
  assert.equal(cache.peek('note-1'), null);
  assert.equal(await cache.resolve('note-1', loader('b')), 'b');

  const pending = deferred();
  const inFlight = cache.resolve('note-2', () => pending.promise);
  cache.invalidate('note-2');
  pending.resolve('late');
  assert.equal(await inFlight, 'late');
  assert.equal(cache.peek('note-2'), null);

  cache.invalidate();
  assert.equal(cache.size, 0);
  assert.equal(cache.peek('note-1'), null);
});

test('the remembered set stays bounded by dropping the oldest entries', async () => {
  assert.ok(REFERENCE_CACHE_LIMIT > 0);
  const cache = new ReferenceCache({ limit: 2 });
  await cache.resolve('a', loader('A'));
  await cache.resolve('b', loader('B'));
  await cache.resolve('c', loader('C'));
  assert.equal(cache.size, 2);
  assert.equal(cache.peek('a'), null);
  assert.equal(cache.peek('b'), 'B');
  assert.equal(cache.peek('c'), 'C');
});
