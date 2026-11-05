import test from 'node:test';
import assert from 'node:assert/strict';
import { CreateActions } from '../src/organization/create-actions.ts';

const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
const command = {
  kind: 'notes_create',
  payload: {
    command_id: 'create-a',
    note: {
      title: '未命名笔记',
      document_schema: 2,
      document: { type: 'doc', content: [{ type: 'paragraph', attrs: { id: 'node-a' } }] },
    },
  },
};
const result = {
  note: { id: 'note-a', revision_id: 'revision-a' },
  receipt: {
    command_id: 'create-a',
    object_id: 'note-a',
    revision_id: 'revision-a',
    replayed: false,
  },
};
function setup() {
  let record;
  const f = {
    sends: [],
    failWrite: false,
    failConfirm: false,
    failRemove: false,
    lost: false,
    wrong: false,
    failure: null,
  };
  const storage = {
    async read() {
      return structuredClone(record);
    },
    async write(value) {
      if (f.failWrite || (value.result && f.failConfirm)) throw Error('disk full');
      record = structuredClone(value);
    },
    async remove() {
      if (f.failRemove) throw Error('remove failed');
      record = undefined;
    },
  };
  f.open = () =>
    CreateActions.open(identity, storage, async (sent) => {
      assert.deepEqual(sent, record.command);
      f.sends.push(structuredClone(sent));
      if (f.failure) throw f.failure;
      if (f.lost) {
        f.lost = false;
        throw Error('receipt lost');
      }
      return f.wrong ? { ...result, receipt: { ...result.receipt, command_id: 'wrong' } } : result;
    });
  return f;
}
test('creation persists before send and remains recoverable until successful open acknowledgement', async () => {
  const f = setup();
  let c = await f.open();
  assert.deepEqual(await c.create(command), result);
  c = await f.open();
  assert.deepEqual(
    await c.create({ ...command, payload: { ...command.payload, command_id: 'new-id' } }),
    result,
  );
  assert.equal(f.sends.length, 1);
  assert.equal(await c.acknowledgeOpened('other-note'), false);
  assert.equal(await c.acknowledgeOpened('note-a'), true);
  assert.equal((await f.open()).getSnapshot().pending, null);
});
test('reload does not send automatically and receipt loss replays original localized payload', async () => {
  const f = setup();
  f.lost = true;
  let c = await f.open();
  assert.equal(await c.create(command), null);
  c = await f.open();
  assert.equal(f.sends.length, 1);
  await c.create({
    ...command,
    payload: {
      ...command.payload,
      command_id: 'new-id',
      note: { ...command.payload.note, title: 'Untitled note' },
    },
  });
  assert.deepEqual(f.sends[1], command);
});
test('failed local persistence never sends', async () => {
  const f = setup();
  f.failWrite = true;
  const c = await f.open();
  assert.equal(await c.create(command), null);
  assert.equal(f.sends.length, 0);
  f.failWrite = false;
  await c.create();
  assert.deepEqual(f.sends[0], command);
});
test('success persistence failure keeps original request and recovers same note', async () => {
  const f = setup();
  f.failConfirm = true;
  let c = await f.open();
  assert.equal(await c.create(command), null);
  f.failConfirm = false;
  c = await f.open();
  assert.deepEqual(await c.create(), result);
  assert.deepEqual(f.sends[1], command);
});
test('receipt mismatch is unknown and cannot clear the original request', async () => {
  const f = setup();
  f.wrong = true;
  const c = await f.open();
  assert.equal(await c.create(command), null);
  assert.equal(c.getSnapshot().pending.command.payload.command_id, 'create-a');
  assert.equal(await c.acknowledgeOpened('note-a'), false);
});
test('ack storage failure leaves the successful note available without another create', async () => {
  const f = setup();
  const c = await f.open();
  await c.create(command);
  f.failRemove = true;
  assert.equal(await c.acknowledgeOpened('note-a'), false);
  assert.deepEqual(await c.create(), result);
  assert.equal(f.sends.length, 1);
});

const collectionCommand = {
  ...command,
  payload: {
    ...command.payload,
    note: { ...command.payload.note, collection_id: 'topic-a', base_collection_row_version: 3 },
  },
};
for (const code of ['metadata_conflict', 'object_trashed', 'not_found']) {
  test(`definitive collection rejection ${code} releases the old create intent`, async () => {
    const f = setup();
    f.failure = Object.assign(Error('Collection changed'), { details: { code } });
    let c = await f.open();
    assert.equal(await c.create(collectionCommand), null);
    assert.equal(c.getSnapshot().pending, null);
    assert.equal(c.getSnapshot().code, code);
    c = await f.open();
    assert.equal(c.getSnapshot().pending, null);
    f.failure = null;
    const refreshed = structuredClone(collectionCommand);
    refreshed.payload.note.base_collection_row_version = 4;
    assert.deepEqual(await c.create(refreshed), result);
    assert.equal(f.sends.at(-1).payload.note.base_collection_row_version, 4);
  });
}
test('collection creation lost receipt preserves exact membership and revision on reload', async () => {
  const f = setup();
  f.lost = true;
  let c = await f.open();
  assert.equal(await c.create(collectionCommand), null);
  c = await f.open();
  assert.deepEqual(await c.create(command), result);
  assert.deepEqual(f.sends, [collectionCommand, collectionCommand]);
});
test('failure to clear a rejected collection intent preserves it for safe retry', async () => {
  const f = setup();
  f.failure = Object.assign(Error('Collection changed'), {
    details: { code: 'metadata_conflict' },
  });
  f.failRemove = true;
  let c = await f.open();
  assert.equal(await c.create(collectionCommand), null);
  c = await f.open();
  assert.deepEqual(c.getSnapshot().pending.command, collectionCommand);
});
