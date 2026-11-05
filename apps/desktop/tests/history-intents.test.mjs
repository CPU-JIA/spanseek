import assert from 'node:assert/strict';
import test from 'node:test';
import { randomUUID } from 'node:crypto';
import { DraftController } from '../src/editor/draft-controller.ts';
import { draftKey } from '../src/editor/drafts.ts';

function fixture(t) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const identity = { local_instance_id: randomUUID(), workspace_id: randomUUID() };
  const base = {
    id: randomUUID(),
    revision_id: randomUUID(),
    title: 'Original',
    revision: 1,
    document: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          attrs: { id: randomUUID() },
          content: [{ type: 'text', text: 'Original text' }],
        },
      ],
    },
    updated_at: '',
    trashed: false,
  };
  let stored;
  const storage = {
    read: async () => structuredClone(stored),
    write: async (value) => {
      stored = structuredClone(value);
    },
  };
  const open = (note = base) =>
    DraftController.open(identity, note, storage, async () => {
      throw new Error('unexpected save');
    });
  const receipt = (command, note) => ({
    note,
    receipt: {
      command_id: command.payload.command_id,
      object_id: note.id,
      revision_id: note.revision_id,
    },
  });
  return { identity, base, storage, open, receipt, stored: () => stored };
}

test('restore lost receipt survives restart and replays exact ID before adopting current head', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  const restored = { ...f.base, revision_id: randomUUID(), title: 'Restored' };
  let original;
  await assert.rejects(
    controller.runHistory(
      async (command) => {
        original = command;
        assert.deepEqual(f.stored().historyIntent.command, command);
        throw new Error('lost receipt');
      },
      async () => restored,
      { restoreRevision: randomUUID() },
    ),
    /lost receipt/,
  );
  assert.throws(() => controller.update('unsafe', f.base.document), /版本操作/);
  assert.throws(() => controller.freeze(), /版本操作/);
  await assert.rejects(controller.flush(), /版本操作/);
  const restarted = await f.open(restored);
  assert.equal(restarted.getSnapshot().record.title, 'Original');
  const newest = { ...restored, revision_id: randomUUID(), title: 'Later remote edit' };
  await restarted.runHistory(
    async (command) => {
      assert.deepEqual(command, original);
      return f.receipt(command, restored);
    },
    async () => newest,
  );
  assert.equal(restarted.getSnapshot().record.title, newest.title);
  assert.equal(f.stored().historyIntent, null);
  restarted.update('editable again', newest.document);
});

test('conflict preservation replays the original remapped document and creates only one copy', async (t) => {
  const f = fixture(t);
  await f.storage.write({
    key: draftKey(f.identity, f.base.id),
    identity: f.identity,
    base: f.base,
    title: 'Local draft',
    document: f.base.document,
    generation: 1,
    acknowledged: 0,
    pending: null,
    imageImports: {},
  });
  const current = { ...f.base, revision_id: randomUUID(), title: 'Remote current' };
  const controller = await f.open(current);
  let command;
  let created;
  await assert.rejects(
    controller.runHistory(
      async (request) => {
        command = request;
        created = {
          ...f.base,
          id: randomUUID(),
          revision_id: randomUUID(),
          ...request.payload.note,
        };
        throw new Error('lost receipt');
      },
      async () => current,
      { conflictSuffix: ' (conflict)' },
    ),
    /lost receipt/,
  );
  assert.notEqual(
    command.payload.note.document.content[0].attrs.id,
    f.base.document.content[0].attrs.id,
  );
  const restarted = await f.open(current);
  await restarted.runHistory(
    async (request) => {
      assert.deepEqual(request, command);
      return f.receipt(request, created);
    },
    async () => current,
  );
  assert.equal(restarted.getSnapshot().record.title, 'Remote current');
});

test('intent persistence failure never sends and retries persistence with the same request', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  const write = f.storage.write;
  f.storage.write = async (record) => {
    if (record.historyIntent) throw new Error('disk failure');
    await write(record);
  };
  await assert.rejects(
    controller.runHistory(
      async () => assert.fail('must not send'),
      async () => f.base,
      { restoreRevision: randomUUID() },
    ),
    /disk failure/,
  );
  const original = controller.getSnapshot().record.historyIntent.command;
  f.storage.write = write;
  await controller.runHistory(
    async (request) => {
      assert.deepEqual(request, original);
      return f.receipt(request, f.base);
    },
    async () => f.base,
  );
});

test('failure adopting the result retains original draft and intent across restart', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  const write = f.storage.write;
  const current = { ...f.base, title: 'Restored', revision_id: randomUUID() };
  let original;
  f.storage.write = async (record) => {
    if (record.historyIntent === null) throw new Error('adoption disk failure');
    await write(record);
  };
  await assert.rejects(
    controller.runHistory(
      async (command) => {
        original = command;
        return f.receipt(command, current);
      },
      async () => current,
      { restoreRevision: randomUUID() },
    ),
    /adoption disk failure/,
  );
  assert.equal(controller.getSnapshot().record.title, f.base.title);
  f.storage.write = write;
  const restarted = await f.open(current);
  await restarted.runHistory(
    async (command) => {
      assert.deepEqual(command, original);
      return f.receipt(command, current);
    },
    async () => current,
  );
  assert.equal(restarted.getSnapshot().record.title, 'Restored');
});

test('two panes cannot overlap history operations or edit during request preparation', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  let finish;
  const gate = new Promise((resolve) => {
    finish = resolve;
  });
  const first = controller.runHistory(
    async (command) => {
      await gate;
      return f.receipt(command, f.base);
    },
    async () => f.base,
    { restoreRevision: randomUUID() },
  );
  assert.equal(controller.getSnapshot().historyBusy, true);
  assert.throws(() => controller.update('lost edit', f.base.document), /版本操作/);
  await assert.rejects(
    controller.runHistory(
      async () => assert.fail(),
      async () => f.base,
      { restoreRevision: randomUUID() },
    ),
    /正在进行/,
  );
  finish();
  await first;
});

test('invalid receipts remain blocked and definitive conflicts preserve the draft', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  await assert.rejects(
    controller.runHistory(
      async (command) => ({
        ...f.receipt(command, f.base),
        receipt: { command_id: 'wrong' },
      }),
      async () => f.base,
      { restoreRevision: randomUUID() },
    ),
    /回执校验/,
  );
  assert.ok(controller.getSnapshot().record.historyIntent);
  await assert.rejects(
    controller.runHistory(
      async () => {
        throw Object.assign(new Error('conflict'), { details: { code: 'revision_conflict' } });
      },
      async () => f.base,
    ),
    /conflict/,
  );
  assert.equal(controller.getSnapshot().record.historyIntent, null);
  assert.equal(controller.getSnapshot().status, 'conflict');
  assert.equal(controller.getSnapshot().record.title, f.base.title);
});

test('IME and in-flight image actions prevent starting history mutations', async (t) => {
  const f = fixture(t);
  const controller = await f.open();
  const run = () =>
    controller.runHistory(
      async () => assert.fail(),
      async () => f.base,
      { restoreRevision: randomUUID() },
    );
  controller.setComposing('right-title', true);
  await assert.rejects(run(), /输入法/);
  controller.setComposing('right-title', false);
  const finish = controller.beginAction();
  await assert.rejects(run(), /图片/);
  finish();
});
