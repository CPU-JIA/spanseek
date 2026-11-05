import test from 'node:test';
import assert from 'node:assert/strict';
import { BulkActions } from '../src/organization/bulk-actions.ts';
import { MetadataController } from '../src/organization/metadata-controller.ts';

const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
function setup() {
  let record;
  const intents = new Map();
  const metadata = new Map();
  const controllers = new Map();
  const receipts = new Map();
  const fixture = { sends: [], loseReceipt: false, failWrite: false, failDoneWrite: false };
  const storage = {
    async read() {
      return structuredClone(record);
    },
    async write(value) {
      if (
        fixture.failWrite ||
        (fixture.failDoneWrite && value.items.some((item) => item.status === 'done'))
      )
        throw Error('disk full');
      record = structuredClone(value);
    },
    async remove() {
      record = undefined;
    },
  };
  const get = async (id) => {
    if (!controllers.has(id))
      controllers.set(
        id,
        await MetadataController.open(
          identity,
          id,
          {
            async flush() {},
            beginAction() {
              return () => {};
            },
          },
          {
            async read(key) {
              return structuredClone(intents.get(key));
            },
            async write(intent) {
              intents.set(intent.key, structuredClone(intent));
            },
            async remove(key) {
              intents.delete(key);
            },
          },
          async () => metadata.get(id) ?? { id, row_version: 1, tags: [], inbox_state: 'pending' },
          async (command) => {
            fixture.sends.push(structuredClone(command));
            assert.ok(
              record.items.some((item) => JSON.stringify(item.command) === JSON.stringify(command)),
              'exact command persisted before send',
            );
            if (!receipts.has(command.payload.command_id)) {
              receipts.set(command.payload.command_id, command);
              metadata.set(id, {
                id,
                row_version: 2,
                tags: [],
                inbox_state: command.payload.request.target,
              });
            } else assert.deepEqual(receipts.get(command.payload.command_id), command);
            if (fixture.loseReceipt) {
              fixture.loseReceipt = false;
              throw Error('lost receipt');
            }
          },
        ),
      );
    return controllers.get(id);
  };
  fixture.open = () => BulkActions.open(identity, storage, get);
  fixture.get = get;
  fixture.reopen = () => {
    controllers.clear();
    return fixture.open();
  };
  return fixture;
}
test('bulk persists explicit selection and reports each result', async () => {
  const f = setup();
  const bulk = await f.open();
  await bulk.start(
    [
      { id: 'one', title: 'One' },
      { id: 'two', title: 'Two' },
    ],
    { kind: 'inbox', target: 'processed' },
  );
  assert.equal(f.sends.length, 2);
  assert.deepEqual(
    bulk.getSnapshot().record.items.map((item) => item.status),
    ['done', 'done'],
  );
});
test('lost receipt survives reload and retries exact command only', async () => {
  const f = setup();
  f.loseReceipt = true;
  let bulk = await f.open();
  await bulk.start([{ id: 'one', title: 'One' }], { kind: 'inbox', target: 'snoozed' });
  assert.equal(bulk.getSnapshot().record.items[0].status, 'pending');
  bulk = await f.reopen();
  await bulk.start([{ id: 'two', title: 'Two' }], { kind: 'inbox', target: 'processed' });
  assert.equal(f.sends.length, 1, 'pending batch cannot be replaced');
  await bulk.resume();
  assert.deepEqual(f.sends[0], f.sends[1]);
  assert.equal(bulk.getSnapshot().record.items[0].status, 'done');
});
test('crash boundary after commit but before batch confirmation reuses exact receipt', async () => {
  const f = setup();
  f.failDoneWrite = true;
  let bulk = await f.open();
  await bulk.start([{ id: 'one', title: 'One' }], { kind: 'inbox', target: 'processed' });
  assert.equal(bulk.getSnapshot().record.items[0].status, 'pending');
  f.failDoneWrite = false;
  bulk = await f.reopen();
  await bulk.resume();
  assert.deepEqual(f.sends[0], f.sends[1]);
});
test('batch persistence failure sends nothing', async () => {
  const f = setup();
  f.failWrite = true;
  const bulk = await f.open();
  await bulk.start([{ id: 'one', title: 'One' }], { kind: 'tag', name: 'Read' });
  assert.equal(f.sends.length, 0);
  assert.match(bulk.getSnapshot().error, /disk full/);
});
test('bulk will not replay another pending single-note operation', async () => {
  const f = setup();
  const controller = await f.get('one');
  // Simulate an existing shared queue intent; bulk must leave it to its own confirmation UI.
  controller.getPendingCommand = () => ({
    kind: 'objects_set_inbox_state',
    payload: {
      command_id: 'other',
      request: { object_id: 'one', base_row_version: 1, target: 'snoozed' },
    },
  });
  const bulk = await f.open();
  await bulk.start([{ id: 'one', title: 'One' }], { kind: 'inbox', target: 'processed' });
  assert.equal(f.sends.length, 0);
  assert.equal(bulk.getSnapshot().record.items[0].status, 'ready');
});
