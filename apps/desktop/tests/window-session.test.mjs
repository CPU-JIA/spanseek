import test from 'node:test';
import assert from 'node:assert/strict';
import { fitWindowGeometry, WindowSession } from '../src/layout/window-session.ts';

const frame = { frameWidth: 16, frameHeight: 39, scale: 1 };
const monitor = { left: 0, top: 0, width: 1920, height: 1040, scale: 1 };
const normal = { width: 1201, height: 801, left: 110, top: 60, maximized: false };
function fixture(saved = normal) {
  let actual = { ...normal, ...frame, minimized: false, fullscreen: false };
  let nativeNormal = { ...normal };
  const calls = [];
  const port = {
    read: async () => ({ ...actual }),
    monitors: async () => [monitor],
    position: async (left, top) => {
      calls.push(['position', left, top]);
      Object.assign(actual, { left, top });
    },
    size: async (width, height) => {
      calls.push(['size', width, height]);
      Object.assign(actual, { width, height });
    },
    maximize: async (maximized) => {
      calls.push(['maximize', maximized]);
      if (maximized) {
        nativeNormal = { ...actual };
        Object.assign(actual, { width: 1920, height: 1040, left: 0, top: 0, maximized: true });
      } else Object.assign(actual, nativeNormal, { maximized: false });
    },
  };
  const records = [];
  const positions = {
    queue: { getSnapshot: () => ({ session: { layout: { window: saved } } }) },
    recordWindow: (value) => records.push(value),
  };
  return {
    port,
    positions,
    records,
    calls,
    controller: new WindowSession(port),
    actual: () => actual,
    set: (patch) => Object.assign(actual, patch),
  };
}

test('physical odd pixel geometry and negative monitor coordinates survive unchanged when visible', () => {
  const negative = { left: -1920, top: -200, width: 1920, height: 1080, scale: 1 };
  const saved = { width: 1201, height: 801, left: -1701, top: -101, maximized: false };
  assert.deepEqual(fitWindowGeometry(saved, [monitor, negative], frame), saved);
});

test('removed monitor and oversized window return to the nearest work area without changing monitor DPI', () => {
  const saved = { width: 4000, height: 2200, left: 5000, top: 2000, maximized: true };
  const before = structuredClone(monitor);
  assert.deepEqual(fitWindowGeometry(saved, [monitor], frame), {
    width: 1904,
    height: 1001,
    left: 0,
    top: 0,
    maximized: true,
  });
  assert.deepEqual(monitor, before);
  assert.throws(() => fitWindowGeometry(saved, [], frame), /Monitor work area/);
});

test('higher DPI respects native logical minimum while preserving a reachable titlebar on a small display', () => {
  const result = fitWindowGeometry(
    { ...normal, width: 720, height: 540, left: 2500, top: -1000 },
    [{ ...monitor, width: 1024, height: 768, scale: 1.5 }],
    frame,
  );
  assert.equal(result.width, 1080);
  assert.equal(result.height, 810);
  assert.equal(result.left, 0);
  assert.equal(result.top, 0);
});

test('the home screen keeps the initial window inside the work area and leaves a fitting one alone', async () => {
  const oversized = fixture(null);
  oversized.set({ width: 4000, height: 2200, left: 5000, top: 2000 });
  await oversized.controller.bind(null);
  assert.deepEqual(oversized.calls, [
    ['position', 0, 0],
    ['size', 1904, 1001],
    ['position', 0, 0],
  ]);
  assert.deepEqual(oversized.actual(), {
    width: 1904,
    height: 1001,
    left: 0,
    top: 0,
    maximized: false,
    minimized: false,
    fullscreen: false,
    frameWidth: 16,
    frameHeight: 39,
    scale: 1,
  });

  const fitting = fixture(null);
  fitting.set({ ...normal, minimized: false, fullscreen: false });
  await fitting.controller.bind(null);
  assert.deepEqual(fitting.calls, []);
  assert.equal(fitting.actual().width, normal.width);
  assert.equal(fitting.actual().height, normal.height);
});

test('maximized restart restores the normal rectangle before maximizing, and minimize never replaces it', async () => {
  const f = fixture({ ...normal, maximized: true });
  await f.controller.bind(f.positions);
  assert.deepEqual(f.calls, [
    ['position', 110, 60],
    ['size', 1201, 801],
    ['position', 110, 60],
    ['maximize', true],
  ]);
  assert.deepEqual(f.records.at(-1), { ...normal, maximized: true });
  f.set({ minimized: true, width: 0, height: 0, left: -32000, top: -32000 });
  const count = f.records.length;
  await f.controller.sample();
  assert.equal(f.records.length, count);
});

test('native restore bounds capture immediate resize then maximize without a normal frontend event', async () => {
  const f = fixture();
  await f.controller.bind(f.positions);
  const resized = { width: 1317, height: 853, left: 203, top: 87, maximized: true };
  f.set({
    width: 1920,
    height: 1040,
    left: 0,
    top: 0,
    maximized: true,
    restoreGeometry: resized,
  });
  await f.controller.sample();
  assert.deepEqual(f.records.at(-1), resized);
});

test('maximized before opening the first workspace uses native restore bounds instead of window defaults', async () => {
  const f = fixture(null);
  const restored = { width: 1403, height: 897, left: 305, top: 103, maximized: true };
  f.set({
    width: 1920,
    height: 1040,
    left: 0,
    top: 0,
    maximized: true,
    restoreGeometry: restored,
  });
  await f.controller.bind(null);
  assert.equal(f.records.length, 0);
  await f.controller.bind(f.positions);
  assert.deepEqual(f.records, [restored]);
  assert.deepEqual(f.calls, []);
});

test('restore-generated native events read only after restore has completed', async () => {
  const f = fixture();
  const samples = [];
  const position = f.port.position;
  f.port.position = async (...args) => {
    await position(...args);
    samples.push(f.controller.sample());
  };
  await f.controller.bind(f.positions);
  await Promise.all(samples);
  assert.ok(f.records.every((value) => JSON.stringify(value) === JSON.stringify(normal)));
});

test('a late workspace restore cannot overwrite the newer workspace or send it the old geometry', async () => {
  const f = fixture();
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const position = f.port.position;
  let entered;
  const started = new Promise((resolve) => {
    entered = resolve;
  });
  f.port.position = async (...args) => {
    entered();
    await gate;
    await position(...args);
  };
  const old = f.controller.bind(f.positions);
  await started;
  const records = [],
    target = { ...normal, width: 1307, left: 350 };
  const next = {
    queue: { getSnapshot: () => ({ session: { layout: { window: target } } }) },
    recordWindow: (value) => records.push(value),
  };
  const changed = f.controller.bind(next);
  release();
  await Promise.all([old, changed]);
  assert.equal(f.records.length, 0);
  assert.deepEqual(records.at(-1), target);
});

test('native restore failure stays visible and later native events cannot save the intermediate position', async () => {
  const f = fixture();
  f.port.size = async () => {
    throw Error('native size failure');
  };
  await assert.rejects(f.controller.bind(f.positions), /native size failure/);
  await f.controller.sample();
  assert.equal(f.records.length, 0);
});

for (const operation of ['read', 'position', 'size']) {
  test(`explicit close capture retries a failed ${operation} restore without persisting intermediate geometry`, async () => {
    const f = fixture();
    f.set({ width: 1400, height: 900, left: 500, top: 130 });
    const original = f.port[operation];
    f.port[operation] = async () => {
      throw Error(`temporary ${operation} failure`);
    };
    const error = new RegExp(`temporary ${operation} failure`);
    await assert.rejects(f.controller.bind(f.positions), error);
    await f.controller.sample();
    assert.equal(f.records.length, 0);
    await assert.rejects(f.controller.captureCurrent(), error);
    await f.controller.sample();
    assert.equal(f.records.length, 0);

    f.port[operation] = original;
    await f.controller.captureCurrent();
    assert.ok(f.records.length > 0);
    assert.ok(f.records.every((value) => JSON.stringify(value) === JSON.stringify(normal)));
    const moved = { ...normal, left: 250, top: 110 };
    f.set(moved);
    const mutations = f.calls.length;
    await f.controller.captureCurrent();
    assert.equal(f.calls.length, mutations);
    assert.deepEqual(f.records.at(-1), moved);
  });
}
