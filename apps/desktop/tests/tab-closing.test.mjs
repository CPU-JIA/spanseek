import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applyTabClose,
  closeTabsAfterBarriers,
  handleTabCloseKey,
  planTabClose,
} from '../src/layout/tab-closing.ts';
import { SessionQueue } from '../src/layout/session-queue.ts';

const pane = (ids, active) => ({
  tabs: ids.map((object_id, index) => ({ object_id, kind: index % 2 ? 'source' : 'note' })),
  active_object_id: active,
  scroll_offsets: ids.map((object_id, index) => ({ object_id, offset: index + 10 })),
});
const layout = () => ({
  window: null,
  panes: [pane(['a', 'b', 'c'], 'b'), pane(['a', 'z'], 'z')],
  active_pane: 0,
  navigation: 'notes',
  sidebar_visible: true,
  sidebar_width: 256,
  note_list_width: null,
});
const ids = (value, index = 0) => value.panes[index].tabs.map((tab) => tab.object_id);
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => (resolve = done));
  return { promise, resolve };
};

test('closing this, other or all tabs changes only the requested pane and its reading positions', () => {
  const before = layout();
  const original = structuredClone(before);
  for (const [mode, remaining, active] of [
    ['this', ['a', 'c'], 'c'],
    ['others', ['b'], 'b'],
    ['all', [], null],
  ]) {
    const plan = planTabClose(before, 0, 'b', mode);
    const after = applyTabClose(before, plan);
    assert.deepEqual(ids(after), remaining);
    assert.deepEqual(
      after.panes[0].scroll_offsets.map((offset) => offset.object_id),
      remaining,
    );
    assert.equal(after.panes[0].active_object_id, active);
    assert.strictEqual(after.panes[1], before.panes[1]);
    assert.equal(after.active_pane, before.active_pane);
    assert.deepEqual(applyTabClose(after, plan), after, 'replay is idempotent');
  }
  assert.deepEqual(before, original, 'planning and applying never mutate the old layout');
});

test('inactive-tab close preserves the active document; close-others selects the kept tab when needed', () => {
  const before = layout();
  const one = applyTabClose(before, planTabClose(before, 0, 'a', 'this'));
  assert.equal(one.panes[0].active_object_id, 'b');
  const others = applyTabClose(before, planTabClose(before, 0, 'a', 'others'));
  assert.equal(others.panes[0].active_object_id, 'a');
  assert.deepEqual(ids(others), ['a']);
});

test('a frozen batch preserves subsequently opened tabs and does not close others after its keeper disappears', () => {
  const before = layout();
  const all = planTabClose(before, 0, 'b', 'all');
  const changed = structuredClone(before);
  changed.panes[0] = pane(['a', 'b', 'c', 'new'], 'new');
  assert.deepEqual(ids(applyTabClose(changed, all)), ['new']);
  const others = planTabClose(before, 0, 'b', 'others');
  changed.panes[0] = pane(['a', 'c', 'new'], 'new');
  assert.strictEqual(applyTabClose(changed, others), changed);
});

test('missing documents, missing panes and closing others on a single tab produce no operation', () => {
  const before = layout();
  assert.equal(planTabClose(before, 2, 'b', 'all'), null);
  assert.equal(planTabClose(before, 0, 'missing', 'all'), null);
  before.panes[0] = pane(['only'], 'only');
  assert.equal(planTabClose(before, 0, 'only', 'others'), null);
});

test('a batch waits for draft and reading barriers exactly once, then commits a single layout', async () => {
  let value = layout();
  const before = structuredClone(value);
  const plan = planTabClose(value, 0, 'b', 'all');
  const saving = deferred();
  const calls = [];
  const running = closeTabsAfterBarriers(
    plan,
    [
      () => {
        calls.push('drafts');
        return saving.promise;
      },
      async () => {
        calls.push('reading');
        assert.deepEqual(value, before);
      },
    ],
    () => true,
    async (change) => {
      calls.push('layout');
      value = change(value);
    },
  );
  assert.deepEqual(value, before);
  assert.deepEqual(calls, ['drafts']);
  saving.resolve();
  assert.equal(await running, true);
  assert.deepEqual(calls, ['drafts', 'reading', 'layout']);
  assert.deepEqual(ids(value), []);
  assert.deepEqual(value.panes[1], before.panes[1]);
});

for (const failure of ['drafts', 'reading']) {
  test(`${failure} failure leaves the whole close set and active document intact`, async () => {
    const value = layout();
    const before = structuredClone(value);
    let writes = 0;
    await assert.rejects(
      closeTabsAfterBarriers(
        planTabClose(value, 0, 'a', 'others'),
        ['drafts', 'reading'].map((stage) => async () => {
          if (stage === failure) throw new Error(`${stage} failed`);
        }),
        () => true,
        async () => {
          writes++;
        },
      ),
      new RegExp(failure),
    );
    assert.equal(writes, 0);
    assert.deepEqual(value, before);
  });
}

test('a navigation during the save cancels the entire batch before any layout write', async () => {
  const value = layout();
  const before = structuredClone(value);
  const saving = deferred();
  let current = true;
  let writes = 0;
  const closing = closeTabsAfterBarriers(
    planTabClose(value, 0, 'a', 'all'),
    [() => saving.promise],
    () => current,
    async () => {
      writes++;
    },
  );
  current = false;
  saving.resolve();
  assert.equal(await closing, false);
  assert.equal(writes, 0);
  assert.deepEqual(value, before);
});

test('a queued close rechecks navigation at application, preserving the newer layout', async () => {
  const value = layout();
  let current = true;
  let result;
  const closed = await closeTabsAfterBarriers(
    planTabClose(value, 0, 'a', 'all'),
    [],
    () => current,
    async (change) => {
      current = false;
      result = change(value);
    },
  );
  assert.equal(closed, false);
  assert.strictEqual(result, value);
});

test('failed session persistence keeps every tab visible and an expired retry cannot close them later', async () => {
  const initial = {
    identity: { local_instance_id: 'local', workspace_id: 'workspace' },
    state_version: 1,
    layout: layout(),
  };
  let visible = structuredClone(initial);
  let current = true;
  let saves = 0;
  const queue = new SessionQueue(initial, {
    read: async () => structuredClone(initial),
    save: async () => {
      saves++;
      throw new Error('session storage failed');
    },
    onConfirmed: (value) => {
      visible = value;
    },
  });
  await assert.rejects(
    closeTabsAfterBarriers(
      planTabClose(initial.layout, 0, 'b', 'all'),
      [],
      () => current,
      (change) => queue.enqueue(change),
    ),
    /session storage failed/,
  );
  assert.deepEqual(visible, initial);
  assert.equal(queue.getSnapshot().pending, 1);
  current = false;
  await queue.flush();
  assert.equal(queue.getSnapshot().pending, 0);
  assert.equal(saves, 1);
  assert.deepEqual(visible, initial);
});

const key = (patch = {}) => {
  const value = {
    key: 'w',
    ctrlKey: true,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    isComposing: false,
    keyCode: 87,
    repeat: false,
    defaultPrevented: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    ...patch,
  };
  return value;
};

test('Ctrl+W closes only the active actual tab and does not match Shift/Alt variants', () => {
  const closed = [];
  const context = {
    layout: layout(),
    contentActive: true,
    blocked: false,
    close: (...args) => closed.push(args),
  };
  for (const event of [key(), key({ key: 'W' }), key({ ctrlKey: false, metaKey: true })]) {
    assert.equal(handleTabCloseKey(event, context), true);
    assert.equal(event.defaultPrevented, true);
  }
  assert.deepEqual(closed, [
    [0, 'b'],
    [0, 'b'],
    [0, 'b'],
  ]);
  for (const patch of [{ shiftKey: true }, { altKey: true }, { ctrlKey: false }, { key: 'q' }]) {
    const event = key(patch);
    assert.equal(handleTabCloseKey(event, context), false);
    assert.equal(event.defaultPrevented, false);
  }
  assert.equal(closed.length, 3);
});

test('IME, repeat, modals and non-content or empty pages consume Ctrl+W without closing anything', () => {
  const closed = [];
  const context = {
    layout: layout(),
    contentActive: true,
    blocked: false,
    close: (...args) => closed.push(args),
  };
  for (const patch of [
    { isComposing: true },
    { keyCode: 229 },
    { repeat: true },
    { defaultPrevented: true },
  ]) {
    const event = key(patch);
    assert.equal(handleTabCloseKey(event, context), true);
    assert.equal(event.defaultPrevented, true);
  }
  const empty = layout();
  empty.panes[0] = pane([], null);
  const missing = layout();
  missing.panes[0].active_object_id = 'missing';
  for (const patch of [
    { blocked: true },
    { contentActive: false },
    { layout: null },
    { layout: empty },
    { layout: missing },
  ]) {
    const event = key();
    assert.equal(handleTabCloseKey(event, { ...context, ...patch }), true);
    assert.equal(event.defaultPrevented, true);
  }
  assert.deepEqual(closed, []);
});
