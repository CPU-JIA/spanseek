import test from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { SessionQueue } from '../src/layout/session-queue.ts';

const clone = structuredClone;
const conflict = () =>
  Object.assign(new Error('version conflict'), { details: { code: 'app_state_conflict' } });
function fixture() {
  let stored = {
    identity: { local_instance_id: randomUUID(), workspace_id: randomUUID() },
    state_version: 0,
    layout: {
      window: null,
      panes: [{ tabs: [], active_object_id: null, scroll_offsets: [] }],
      active_pane: 0,
      navigation: 'notes',
      sidebar_visible: true,
      sidebar_width: null,
      note_list_width: null,
    },
  };
  const writes = [];
  const options = {
    read: async () => clone(stored),
    save: async (version, layout) => {
      writes.push({ version, layout: clone(layout) });
      if (version !== stored.state_version) throw conflict();
      stored = { ...stored, state_version: version + 1, layout: clone(layout) };
      return clone(stored);
    },
  };
  const initial = clone(stored);
  return {
    initial,
    options,
    writes,
    stored: () => clone(stored),
    remote(change) {
      stored = {
        ...stored,
        state_version: stored.state_version + 1,
        layout: change(clone(stored.layout)),
      };
    },
  };
}
const hideSidebar = (layout) => ({ ...layout, sidebar_visible: false });
const width = (value) => (layout) => ({ ...layout, sidebar_width: value });

test('accepted save with a lost reply is reconciled without another write or double toggle', async () => {
  const f = fixture();
  const save = f.options.save;
  f.options.save = async (...args) => {
    await save(...args);
    throw new Error('lost receipt');
  };
  const queue = new SessionQueue(f.initial, f.options);
  await queue.enqueue(hideSidebar);
  assert.equal(f.writes.length, 1);
  assert.equal(queue.getSnapshot().pending, 0);
  assert.equal(queue.getSnapshot().session.layout.sidebar_visible, false);
  assert.equal(queue.getSnapshot().error, null);
});

test('version conflict rebases a target while preserving unrelated concurrent changes', async () => {
  const f = fixture();
  const queue = new SessionQueue(f.initial, f.options);
  f.remote((layout) => ({ ...layout, note_list_width: 400 }));
  await queue.enqueue(width(240));
  assert.equal(f.writes.length, 2);
  assert.equal(f.stored().layout.note_list_width, 400);
  assert.equal(f.stored().layout.sidebar_width, 240);
  assert.equal(f.stored().state_version, 2);
});

test('accepted change followed by a concurrent unrelated edit is not written again', async () => {
  const f = fixture();
  const save = f.options.save;
  f.options.save = async (...args) => {
    await save(...args);
    f.remote((layout) => ({ ...layout, note_list_width: 420 }));
    throw new Error('lost receipt');
  };
  const queue = new SessionQueue(f.initial, f.options);
  await queue.enqueue(hideSidebar);
  assert.equal(f.writes.length, 1);
  assert.equal(queue.getSnapshot().session.layout.note_list_width, 420);
});

test('unavailable network preserves the head and prevents later changes overtaking it; flush recovers', async () => {
  const f = fixture();
  const save = f.options.save;
  const read = f.options.read;
  let offline = true;
  f.options.save = async (...args) => {
    if (offline) throw new Error('offline write');
    return save(...args);
  };
  f.options.read = async () => {
    if (offline) throw new Error('offline read');
    return read();
  };
  const queue = new SessionQueue(f.initial, f.options);
  await assert.rejects(queue.enqueue(hideSidebar), /offline/);
  await assert.rejects(queue.enqueue(width(250)), /offline/);
  assert.equal(queue.getSnapshot().pending, 2);
  assert.equal(f.writes.length, 0);
  assert.ok(queue.getSnapshot().error);
  offline = false;
  await queue.flush();
  assert.equal(queue.getSnapshot().pending, 0);
  assert.equal(queue.getSnapshot().error, null);
  assert.equal(f.writes.length, 2);
  assert.equal(f.writes[0].layout.sidebar_visible, false);
  assert.equal(f.writes[0].layout.sidebar_width, null);
  assert.equal(f.writes[1].layout.sidebar_width, 250);
});

test('disk write failure remains visible, and retrying flush uses the retained target', async () => {
  const f = fixture();
  const save = f.options.save;
  let full = true;
  f.options.save = async (...args) => {
    if (full) throw new Error('disk full');
    return save(...args);
  };
  const queue = new SessionQueue(f.initial, f.options);
  await assert.rejects(queue.enqueue(width(300)), /disk full/);
  assert.equal(queue.getSnapshot().session.layout.sidebar_width, null);
  assert.equal(queue.getSnapshot().pending, 1);
  full = false;
  await queue.flush();
  assert.equal(f.stored().layout.sidebar_width, 300);
  assert.equal(f.writes.length, 1);
});

test('flush after commit and failed reconciliation confirms the old request before another change', async () => {
  const f = fixture();
  const save = f.options.save;
  const read = f.options.read;
  let failRead = true;
  f.options.save = async (...args) => {
    const result = await save(...args);
    if (failRead) throw new Error('lost');
    return result;
  };
  f.options.read = async () => {
    if (failRead) throw new Error('read failed');
    return read();
  };
  const queue = new SessionQueue(f.initial, f.options);
  await assert.rejects(queue.enqueue(hideSidebar), /read failed/);
  failRead = false;
  await queue.enqueue(width(270));
  assert.equal(f.writes.length, 2);
  assert.equal(f.stored().state_version, 2);
  assert.equal(f.stored().layout.sidebar_visible, false);
});

test('two workspace queues stay isolated and a wrong-workspace reconciliation cannot clear pending work', async () => {
  const a = fixture();
  const b = fixture();
  const queueB = new SessionQueue(b.initial, b.options);
  const readA = a.options.read;
  const saveA = a.options.save;
  a.options.save = async () => {
    throw new Error('unavailable');
  };
  a.options.read = b.options.read;
  const queueA = new SessionQueue(a.initial, a.options);
  await assert.rejects(queueA.enqueue(hideSidebar), /identity/);
  assert.equal(queueA.getSnapshot().pending, 1);
  assert.deepEqual(queueA.getSnapshot().session.identity, a.initial.identity);
  await queueB.enqueue(width(280));
  assert.equal(b.stored().layout.sidebar_visible, true);
  a.options.read = readA;
  a.options.save = saveA;
  await queueA.flush();
  assert.equal(a.stored().layout.sidebar_visible, false);
  assert.equal(a.stored().layout.sidebar_width, null);
});

test('non-idempotent toggles are rejected before enqueue, and slow writes serialize changes', async () => {
  const f = fixture();
  const queue = new SessionQueue(f.initial, f.options);
  await assert.rejects(
    queue.enqueue((layout) => ({ ...layout, sidebar_visible: !layout.sidebar_visible })),
    /idempotent/,
  );
  assert.equal(queue.getSnapshot().pending, 0);
  const save = f.options.save;
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  f.options.save = async (...args) => {
    await gate;
    return save(...args);
  };
  const first = queue.enqueue(hideSidebar);
  const second = queue.enqueue(width(290));
  release();
  await Promise.all([first, second, queue.flush()]);
  assert.deepEqual(
    f.writes.map((write) => write.version),
    [0, 1],
  );
  assert.equal(f.stored().layout.sidebar_visible, false);
  assert.equal(f.stored().layout.sidebar_width, 290);
});

test('persistent concurrent conflicts are bounded and retain the operation for a later flush', async () => {
  const f = fixture();
  const save = f.options.save;
  f.options.save = async (...args) => {
    f.remote((layout) => ({ ...layout, note_list_width: (layout.note_list_width ?? 200) + 1 }));
    return save(...args);
  };
  const queue = new SessionQueue(f.initial, f.options);
  await assert.rejects(queue.enqueue(width(300)), /version conflict/);
  assert.equal(f.writes.length, 3);
  assert.equal(queue.getSnapshot().pending, 1);
  f.options.save = save;
  await queue.flush();
  assert.equal(f.stored().layout.sidebar_width, 300);
  assert.equal(f.stored().layout.note_list_width, 203);
});

test('a cached no-op verifies current state instead of silently dropping the desired target', async () => {
  const f = fixture();
  const queue = new SessionQueue(f.initial, f.options);
  f.remote(hideSidebar);
  await queue.enqueue((layout) => ({ ...layout, sidebar_visible: true }));
  assert.equal(f.stored().layout.sidebar_visible, true);
  assert.equal(f.writes.length, 1);
});

test('enqueue and flush during the preceding drain settlement wait for the new write', async () => {
  const f = fixture();
  const queue = new SessionQueue(f.initial, f.options);
  let late;
  let barrier;
  const first = queue.flush();
  queueMicrotask(() => {
    late = queue.enqueue(width(310));
    barrier = queue.flush();
  });
  await first;
  await barrier;
  assert.equal(f.stored().layout.sidebar_width, 310);
  await late;
  assert.equal(queue.getSnapshot().pending, 0);
});

test('a regressing read is rejected and cannot clear the retained operation', async () => {
  const f = fixture();
  const queue = new SessionQueue(f.initial, f.options);
  await queue.enqueue(width(320));
  const read = f.options.read;
  const save = f.options.save;
  f.options.save = async () => {
    throw new Error('lost');
  };
  f.options.read = async () => clone(f.initial);
  await assert.rejects(queue.enqueue(hideSidebar), /version mismatch/);
  assert.equal(queue.getSnapshot().session.state_version, 1);
  assert.equal(queue.getSnapshot().pending, 1);
  f.options.read = read;
  f.options.save = save;
  await queue.flush();
  assert.equal(f.stored().layout.sidebar_visible, false);
});
