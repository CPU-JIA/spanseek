import assert from 'node:assert/strict';
import test from 'node:test';
import { randomUUID } from 'node:crypto';
import { DraftController } from '../src/editor/draft-controller.ts';
import { draftKey } from '../src/editor/drafts.ts';
import { compareConflict } from '../src/editor/conflict-resolution.ts';

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
async function fixture(t) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const identity = { local_instance_id: randomUUID(), workspace_id: randomUUID() };
  const document = (text) => ({
    type: 'doc',
    content: [
      { type: 'paragraph', attrs: { id: randomUUID() }, content: [{ type: 'text', text }] },
    ],
  });
  const base = {
    id: randomUUID(),
    revision_id: randomUUID(),
    revision: 1,
    title: 'Base',
    document: document('Original'),
    trashed: false,
  };
  const current = {
    ...base,
    revision_id: randomUUID(),
    revision: 2,
    title: 'Saved latest',
    document: document('External change'),
  };
  let stored = {
    key: draftKey(identity, base.id),
    identity,
    base,
    title: 'Local draft',
    document: document('My uncommitted change'),
    generation: 1,
    acknowledged: 0,
    pending: null,
    imageImports: {},
  };
  const storage = {
    read: async () => structuredClone(stored),
    write: async (value) => {
      stored = structuredClone(value);
    },
  };
  const save = async () => assert.fail('discard must never save a core revision');
  const controller = await DraftController.open(identity, current, storage, save);
  return {
    controller,
    current,
    storage,
    document,
    saved: () => stored,
    reopen: () => DraftController.open(identity, current, storage, save),
  };
}

test('comparison preserves both full documents; confirmed discard persists only the adopted draft state', async (t) => {
  const f = await fixture(t);
  const before = structuredClone(f.saved());
  const comparison = await compareConflict(f.controller, async () => f.current);
  assert.deepEqual(comparison.local.document, before.document);
  assert.deepEqual(comparison.current.document, f.current.document);
  assert.deepEqual(f.saved(), before);
  await f.controller.discardConflict(
    comparison.local,
    comparison.current.revision_id,
    async () => f.current,
  );
  assert.equal(f.controller.getSnapshot().status, 'saved');
  assert.deepEqual(f.saved().document, f.current.document);
  assert.equal(f.saved().pending, null);
  assert.equal(f.saved().generation, f.saved().acknowledged);
  const reopened = await f.reopen();
  assert.equal(reopened.getSnapshot().status, 'saved');
  assert.deepEqual(reopened.getSnapshot().record.document, f.current.document);
});

test('local edits arriving while the latest revision is read invalidate comparison', async (t) => {
  const f = await fixture(t);
  const gate = deferred();
  const reading = compareConflict(f.controller, () => gate.promise);
  f.controller.update('Newer local draft', f.document('Keep this latest edit'));
  gate.resolve(f.current);
  await assert.rejects(reading, /本地草稿已变化/);
  assert.equal(f.controller.getSnapshot().record.title, 'Newer local draft');
});

test('a newer saved revision or a recycled note cannot discard the compared draft', async (t) => {
  const f = await fixture(t);
  const comparison = await compareConflict(f.controller, async () => f.current);
  const original = structuredClone(f.saved());
  for (const latest of [
    { ...f.current, revision_id: randomUUID(), revision: 3 },
    { ...f.current, id: randomUUID() },
    { ...f.current, trashed: true },
  ]) {
    await assert.rejects(
      f.controller.discardConflict(comparison.local, f.current.revision_id, async () => latest),
      /版本已变化|回收站/,
    );
    assert.deepEqual(f.saved(), original);
    assert.equal(f.controller.getSnapshot().status, 'conflict');
  }
});

test('local edits during discard revalidation survive; read failures do not remove the draft', async (t) => {
  const f = await fixture(t);
  const comparison = await compareConflict(f.controller, async () => f.current);
  await assert.rejects(
    f.controller.discardConflict(comparison.local, f.current.revision_id, async () => {
      throw new Error('read unavailable');
    }),
    /read unavailable/,
  );
  assert.equal(f.controller.getSnapshot().record, comparison.local);
  const gate = deferred();
  const discarding = f.controller.discardConflict(
    comparison.local,
    f.current.revision_id,
    () => gate.promise,
  );
  assert.throws(() => f.controller.freeze(), /图片或版本操作/);
  f.controller.update('Changed during confirmation', f.document('Keep newer local content'));
  gate.resolve(f.current);
  await assert.rejects(discarding, /又有修改/);
  assert.equal(f.controller.getSnapshot().record.title, 'Changed during confirmation');
});

test('failed local adoption retains the compared draft across reopening and permits explicit retry', async (t) => {
  const f = await fixture(t);
  const comparison = await compareConflict(f.controller, async () => f.current);
  const original = structuredClone(f.saved());
  const write = f.storage.write;
  f.storage.write = async () => {
    throw new Error('IndexedDB transaction aborted');
  };
  await assert.rejects(
    f.controller.discardConflict(comparison.local, f.current.revision_id, async () => f.current),
    /transaction aborted/,
  );
  assert.deepEqual(f.saved(), original);
  assert.equal(f.controller.getSnapshot().record, comparison.local);
  const reopened = await f.reopen();
  assert.equal(reopened.getSnapshot().status, 'conflict');
  assert.deepEqual(reopened.getSnapshot().record.document, original.document);
  f.storage.write = write;
  await f.controller.discardConflict(
    comparison.local,
    f.current.revision_id,
    async () => f.current,
  );
  assert.equal(f.controller.getSnapshot().status, 'saved');
});

test('IME and in-flight image actions block discard before any current-version read', async (t) => {
  const f = await fixture(t);
  const comparison = await compareConflict(f.controller, async () => f.current);
  const load = async () => assert.fail('must reject before reading');
  f.controller.setComposing('second-pane', true);
  await assert.rejects(
    f.controller.discardConflict(comparison.local, f.current.revision_id, load),
    /输入法/,
  );
  f.controller.setComposing('second-pane', false);
  const end = f.controller.beginAction();
  await assert.rejects(
    f.controller.discardConflict(comparison.local, f.current.revision_id, load),
    /图片或保存/,
  );
  end();
  assert.equal(f.controller.getSnapshot().record, comparison.local);
});

for (const fails of [false, true]) {
  test(`adoption holds a shared editing lock through its delayed durable write (${fails ? 'failure' : 'success'})`, async (t) => {
    const f = await fixture(t);
    const comparison = await compareConflict(f.controller, async () => f.current);
    const original = structuredClone(f.saved());
    const entered = deferred();
    const gate = deferred();
    const write = f.storage.write;
    let writes = 0;
    f.storage.write = async (record) => {
      writes += 1;
      entered.resolve();
      await gate.promise;
      if (fails) throw new Error('delayed transaction aborted');
      await write(record);
    };
    const observations = [];
    const unsubscribe = f.controller.subscribe(() => {
      observations.push(f.controller.getSnapshot().historyBusy);
    });
    const adopting = f.controller.discardConflict(
      comparison.local,
      f.current.revision_id,
      async () => f.current,
    );
    const completed = adopting.then(
      () => null,
      (error) => error,
    );
    await entered.promise;
    assert.equal(f.controller.getSnapshot().historyBusy, true);
    assert.deepEqual(f.saved(), original);
    assert.throws(
      () =>
        f.controller.update('Another pane edit', f.document('Must not be accepted while adopting')),
      /版本操作/,
    );
    await assert.rejects(f.controller.addImageImport('new-hash', 'new-command'), /版本操作/);
    assert.throws(() => f.controller.beginAction(), /版本操作/);
    assert.throws(() => f.controller.setComposing('other-pane-title', true), /版本操作/);
    assert.throws(() => f.controller.freeze(), /版本操作/);
    await assert.rejects(f.controller.flush(), /版本操作/);
    await assert.rejects(f.controller.retry(), /版本操作/);
    assert.equal(f.controller.getSnapshot().record, comparison.local);
    gate.resolve();
    const error = await completed;
    assert.equal(f.controller.getSnapshot().historyBusy, false);
    assert(observations.includes(true));
    assert.equal(observations.at(-1), false);
    assert.equal(writes, 1, 'adoption must never rely on a repair write');
    if (fails) {
      assert.match(error.message, /delayed transaction aborted/);
      assert.deepEqual(f.saved(), original);
      const reopened = await f.reopen();
      assert.equal(reopened.getSnapshot().status, 'conflict');
      assert.deepEqual(reopened.getSnapshot().record.document, original.document);
    } else {
      assert.equal(error, null);
      assert.equal(f.controller.getSnapshot().status, 'saved');
      assert.deepEqual(f.saved().document, f.current.document);
    }
    unsubscribe();
    f.storage.write = write;
    f.controller.update('Editing resumes after adoption finishes', f.document('Editable again'));
    assert.equal(
      f.controller.getSnapshot().record.title,
      'Editing resumes after adoption finishes',
    );
  });
}
