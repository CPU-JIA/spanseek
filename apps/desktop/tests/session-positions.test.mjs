import test from 'node:test';
import assert from 'node:assert/strict';
import { SessionQueue } from '../src/layout/session-queue.ts';
import { SessionPositions } from '../src/layout/session-positions.ts';

function fixture(id = 'a') {
  let stored = {
    identity: { local_instance_id: id, workspace_id: 'shared-logical-workspace' },
    state_version: 0,
    layout: {
      window: null,
      navigation: 'notes',
      active_pane: 0,
      sidebar_visible: true,
      sidebar_width: null,
      note_list_width: null,
      future_layout: { untouched: true },
      panes: [0, 1].map(() => ({
        tabs: [{ object_id: 'note', kind: 'note' }],
        active_object_id: 'note',
        scroll_offsets: [],
        future_pane: 'keep',
      })),
    },
  };
  const writes = [],
    errors = [];
  let fail = false,
    lost = false;
  const queue = new SessionQueue(stored, {
    read: async () => structuredClone(stored),
    save: async (version, layout) => {
      if (fail) throw Error('disk full');
      assert.equal(version, stored.state_version);
      writes.push(structuredClone(layout));
      stored = { ...stored, state_version: version + 1, layout: structuredClone(layout) };
      if (lost) throw Error('receipt lost');
      return structuredClone(stored);
    },
  });
  const positions = new SessionPositions(queue, (error) => errors.push(error));
  return {
    queue,
    positions,
    writes,
    errors,
    state: () => stored,
    fail: (value) => {
      fail = value;
    },
    lost: () => {
      lost = true;
    },
  };
}
const geometry = { width: 1401, height: 901, left: -1510, top: 45, maximized: false };

test('scroll bursts and native geometry merge once without replacing unrelated layout fields', async () => {
  const f = fixture();
  try {
    f.positions.recordScroll(0, 'note', 0, 100);
    f.positions.recordScroll(0, 'note', 12.7, 1200.3);
    f.positions.recordWindow(geometry);
    assert.equal(f.positions.scroll(0, 'note').y, 1200);
    await f.positions.flush();
    assert.equal(f.writes.length, 1);
    assert.deepEqual(f.state().layout.panes[0].scroll_offsets, [
      { object_id: 'note', x: 13, y: 1200 },
    ]);
    assert.deepEqual(f.state().layout.window, geometry);
    assert.deepEqual(f.state().layout.future_layout, { untouched: true });
    assert.equal(f.state().layout.panes[0].future_pane, 'keep');
    f.positions.recordWindow({ ...geometry });
    f.positions.recordScroll(0, 'note', 13, 1200);
    await f.positions.flush();
    assert.equal(f.writes.length, 1, 'unchanged page must not write again');
  } finally {
    f.positions.dispose();
  }
});

test('normal-close capture includes the final scroll tick and same note remains independent in two panes', async () => {
  const f = fixture();
  try {
    let y = 550;
    const unregister = f.positions.registerCapture(() => f.positions.recordScroll(0, 'note', 0, y));
    f.positions.recordScroll(1, 'note', 4, 1700);
    y = 601;
    await f.positions.flush();
    unregister();
    assert.equal(f.state().layout.panes[0].scroll_offsets[0].y, 601);
    assert.equal(f.state().layout.panes[1].scroll_offsets[0].y, 1700);
  } finally {
    f.positions.dispose();
  }
});

test('late scroll for a closed tab cannot resurrect its tab or invalid offset', async () => {
  const f = fixture();
  try {
    f.positions.recordScroll(1, 'note', 0, 1700);
    await f.queue.enqueue((layout) => ({ ...layout, panes: [layout.panes[0]] }));
    await f.positions.flush();
    assert.equal(f.state().layout.panes.length, 1);
    assert.deepEqual(f.state().layout.panes[0].scroll_offsets, []);
    assert.equal(f.writes.length, 1);
  } finally {
    f.positions.dispose();
  }
});

test('lost position receipt is reconciled and a later sidebar change keeps the captured offsets', async () => {
  const f = fixture();
  try {
    f.lost();
    f.positions.recordScroll(0, 'note', 0, 1900);
    f.positions.recordWindow(geometry);
    await f.positions.flush();
    await f.queue.enqueue((layout) => ({ ...layout, sidebar_visible: false }));
    assert.equal(f.writes.length, 2);
    assert.equal(f.state().layout.panes[0].scroll_offsets[0].y, 1900);
    assert.deepEqual(f.state().layout.window, geometry);
    assert.equal(f.state().layout.sidebar_visible, false);
  } finally {
    f.positions.dispose();
  }
});

test('position failure keeps the queued change and workspace instances never share offsets', async () => {
  const a = fixture('original'),
    b = fixture('restored-copy');
  try {
    a.fail(true);
    a.positions.recordScroll(0, 'note', 0, 810);
    await assert.rejects(a.positions.flush(), /disk full/);
    assert.equal(a.queue.getSnapshot().pending, 1);
    b.positions.recordScroll(0, 'note', 0, 120);
    await b.positions.flush();
    a.fail(false);
    await a.positions.flush();
    assert.equal(a.state().layout.panes[0].scroll_offsets[0].y, 810);
    assert.equal(b.state().layout.panes[0].scroll_offsets[0].y, 120);
    a.positions.dispose();
    a.positions.recordScroll(0, 'note', 0, 999);
    assert.equal(a.positions.scroll(0, 'note').y, 810);
  } finally {
    a.positions.dispose();
    b.positions.dispose();
  }
});
