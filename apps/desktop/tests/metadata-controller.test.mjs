import test from 'node:test';
import assert from 'node:assert/strict';
import { MetadataController } from '../src/organization/metadata-controller.ts';

const identity = { local_instance_id: 'local-a', workspace_id: 'workspace-a' };
const makeMetadata = (version = 1) => ({
  id: 'note-a',
  kind: 'note',
  row_version: version,
  archived: false,
  trashed: false,
  search_enabled: true,
  inbox_state: 'pending',
  tags: [],
});
const build = (metadata) => ({
  kind: 'objects_set_inbox_state',
  payload: {
    command_id: 'intent-a',
    request: {
      object_id: metadata.id,
      base_row_version: metadata.row_version,
      target: 'processed',
    },
  },
});
function setup() {
  let persisted;
  const fixture = {
    value: makeMetadata(),
    sends: [],
    writes: 0,
    actions: 0,
    failWrite: false,
    failRemove: false,
  };
  fixture.editor = {
    async flush() {},
    beginAction() {
      fixture.actions++;
      return () => {
        fixture.actions--;
      };
    },
  };
  fixture.storage = {
    async read() {
      return structuredClone(persisted);
    },
    async write(value) {
      fixture.writes++;
      if (fixture.failWrite) throw Error('disk full');
      persisted = structuredClone(value);
    },
    async remove() {
      if (fixture.failRemove) throw Error('clear failed');
      persisted = undefined;
    },
  };
  fixture.read = async () => structuredClone(fixture.value);
  fixture.send = async (command) => {
    fixture.sends.push(structuredClone(command));
    fixture.value = { ...makeMetadata(2), inbox_state: 'processed' };
  };
  fixture.open = () =>
    MetadataController.open(
      identity,
      'note-a',
      fixture.editor,
      fixture.storage,
      fixture.read,
      (command) => fixture.send(command),
    );
  return fixture;
}
test('a lost receipt survives reopening and replays exactly, then reads current metadata', async () => {
  const f = setup();
  const send = f.send;
  f.send = async (command) => {
    await send(command);
    throw Error('receipt lost');
  };
  const first = await f.open();
  assert.equal(await first.mutate(build), false);
  assert.equal(first.getSnapshot().pending, true);
  const reopened = await f.open();
  f.value = makeMetadata(4);
  f.send = async (command) => {
    f.sends.push(structuredClone(command));
    return { objects: [makeMetadata(2)] };
  };
  assert.equal(await reopened.mutate(), true);
  assert.deepEqual(f.sends[0], f.sends[1]);
  assert.equal(reopened.getSnapshot().metadata.row_version, 4);
  assert.equal(reopened.getSnapshot().pending, false);
  assert.equal(f.actions, 0);
});
test('failed local intent persistence prevents any backend write', async () => {
  const f = setup();
  const controller = await f.open();
  f.failWrite = true;
  assert.equal(await controller.mutate(build), false);
  assert.equal(f.sends.length, 0);
  assert.equal(controller.getSnapshot().pending, false);
  assert.match(controller.getSnapshot().error, /disk full/);
});
test('shared controller serializes two panes and blocks a new action while uncertain', async () => {
  const f = setup();
  let finish;
  f.send = async (command) => {
    f.sends.push(command);
    await new Promise((resolve) => {
      finish = resolve;
    });
    throw Error('lost');
  };
  const controller = await f.open();
  const first = controller.mutate(build);
  while (!finish) await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(await controller.mutate(build), false);
  finish();
  await first;
  assert.equal(await controller.mutate(build), false);
  assert.equal(f.sends.length, 1);
  assert.equal(f.writes, 1);
});
test('metadata conflict refreshes current state without silently retrying or preserving stale intent', async () => {
  const f = setup();
  const controller = await f.open();
  f.send = async () => {
    f.value = makeMetadata(5);
    throw Object.assign(Error('conflict'), { details: { code: 'metadata_conflict' } });
  };
  assert.equal(await controller.mutate(build), false);
  assert.equal(controller.getSnapshot().pending, false);
  assert.equal(controller.getSnapshot().metadata.row_version, 5);
  assert.match(controller.getSnapshot().error, /已有更新/);
  assert.equal((await f.open()).getSnapshot().pending, false);
});
test('failed receipt cleanup keeps exact request available for safe replay', async () => {
  const f = setup();
  const controller = await f.open();
  f.failRemove = true;
  assert.equal(await controller.mutate(build), false);
  assert.equal(controller.getSnapshot().pending, true);
  f.failRemove = false;
  assert.equal(await controller.mutate(), true);
  assert.deepEqual(f.sends[0], f.sends[1]);
});
test('the editors own save is drained before the metadata baseline is captured', async () => {
  const f = setup();
  const controller = await f.open();
  f.editor.flush = async () => {
    f.value = makeMetadata(8);
  };
  await controller.mutate(build);
  assert.equal(f.sends[0].payload.request.base_row_version, 8);
});
