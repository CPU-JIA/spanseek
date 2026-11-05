import test from 'node:test';
import assert from 'node:assert/strict';
import {
  prepareSourceBulk,
  SourceBulkActions,
  sourceBulkKey,
  validateSourceBulk,
} from '../src/sources/source-bulk-actions.ts';

const identity = { local_instance_id: 'local-A', workspace_id: 'workspace-A' };
const selected = [
  { id: 'source-A', title: '甲' },
  { id: 'source-B', title: '乙' },
  { id: 'source-C', title: '丙' },
];
const metadata = (id) => ({ id, kind: 'source', row_version: 7, trashed: false });
const collection = (id) => ({ id, metadata: { row_version: 9, trashed: false } });
const prepare = (items, action) =>
  prepareSourceBulk(
    items,
    action,
    async (id) => metadata(id),
    async (id) => collection(id),
  );
function storage() {
  let record;
  const value = {
    read: async () => structuredClone(record),
    write: async (next) => {
      if (record) assert.equal(record.id, next.id);
      record = structuredClone(next);
    },
    remove: async (current) => {
      assert.equal(record.id, current.id);
      record = undefined;
    },
    get: () => structuredClone(record),
  };
  return value;
}

test('tag and collection membership keep every selected source in one atomic command', async () => {
  const tags = await prepare(selected, { kind: 'tag', name: '  新标签  ' });
  assert.equal(tags.length, 1);
  assert.equal(tags[0].expected, 'metadata_commit');
  assert.deepEqual(tags[0].command.payload.request, {
    targets: selected.map((item) => ({ object_id: item.id, base_row_version: 7 })),
    tag: { kind: 'name', name: '新标签' },
  });
  const collections = await prepare(selected, {
    kind: 'collection',
    id: 'collection-A',
    name: '主题',
  });
  assert.equal(collections.length, 1);
  assert.equal(collections[0].expected, 'collection_commit');
  assert.deepEqual(collections[0].command.payload.request, {
    collection_id: 'collection-A',
    base_row_version: 9,
    member_ids: selected.map((item) => item.id),
  });
});

test('unknown reply pauses the batch and restart replays exact command, skipping completed sources', async () => {
  const disk = storage();
  const sent = [];
  const first = new SourceBulkActions(identity, disk, prepare, async (step) => {
    assert.equal(
      disk
        .get()
        .steps.find((item) => item.command.payload.command_id === step.command.payload.command_id)
        .status,
      'pending',
    );
    sent.push(step.command);
    if (sent.length === 2) throw new Error('lost reply after commit');
  });
  await first.load();
  await first.start(selected, { kind: 'search', value: false });
  assert.deepEqual(
    first.getSnapshot().record.steps.map((item) => item.status),
    ['done', 'pending', 'ready'],
  );
  assert.equal(await first.dismiss(), false);
  await first.start(selected, { kind: 'trash', value: true });
  assert.equal(sent.length, 2);
  const reopened = new SourceBulkActions(
    identity,
    disk,
    async () => {
      throw new Error('must not rebuild');
    },
    async (step) => sent.push(step.command),
  );
  await reopened.load();
  await reopened.resume();
  assert.deepEqual(sent[1], sent[2]);
  assert.equal(sent.length, 4);
  assert.equal(sent[3].payload.request.object_id, 'source-C');
  assert.deepEqual(
    reopened.getSnapshot().record.steps.map((item) => item.status),
    ['done', 'done', 'done'],
  );
  assert.equal(await reopened.dismiss(), true);
  assert.equal(disk.get(), undefined);
});

test('definitive per-source failure stays failed while remaining sources continue; trash can restore', async () => {
  const disk = storage();
  const sent = [];
  const controller = new SourceBulkActions(identity, disk, prepare, async (step) => {
    sent.push(step);
    if (step.ids[0] === 'source-B')
      throw Object.assign(new Error('changed by another view'), {
        details: { code: 'metadata_conflict' },
      });
  });
  await controller.load();
  await controller.start(selected, { kind: 'trash', value: true });
  assert.deepEqual(
    controller.getSnapshot().record.steps.map((item) => item.status),
    ['done', 'failed', 'done'],
  );
  assert.ok(
    sent.every(
      (step) => step.command.kind === 'sources_set_trashed' && step.expected === 'source_mutation',
    ),
  );
  await controller.resume();
  assert.equal(sent.length, 3);
  const restore = await prepare(selected, { kind: 'trash', value: false });
  assert.ok(restore.every((step) => step.command.payload.request.value === false));
});

test('atomic tag rejection marks all targets failed and never splits into partial retries', async () => {
  const disk = storage();
  let sends = 0;
  const controller = new SourceBulkActions(identity, disk, prepare, async () => {
    sends++;
    throw Object.assign(new Error('one target changed'), {
      details: { code: 'metadata_conflict' },
    });
  });
  await controller.load();
  await controller.start(selected, { kind: 'tag', name: '标签' });
  const record = controller.getSnapshot().record;
  assert.equal(record.steps.length, 1);
  assert.equal(record.steps[0].status, 'failed');
  assert.deepEqual(
    record.steps[0].ids,
    selected.map((item) => item.id),
  );
  await controller.resume();
  assert.equal(sends, 1);
});

test('failed initial local persistence prevents sending any command', async () => {
  let sends = 0;
  const disk = storage();
  disk.write = async () => {
    throw new Error('disk full');
  };
  const controller = new SourceBulkActions(identity, disk, prepare, async () => sends++);
  await controller.load();
  await controller.start(selected, { kind: 'search', value: false });
  assert.equal(sends, 0);
  assert.equal(controller.getSnapshot().record, null);
  assert.match(controller.getSnapshot().error, /disk full/);
});

test('failure while storing a successful acknowledgement retains original pending command', async () => {
  const disk = storage();
  const write = disk.write;
  let fail = true;
  disk.write = async (record) => {
    if (fail && record.steps[0].status === 'done')
      throw Object.assign(new Error('ack disk failed'), { details: { code: 'invalid_input' } });
    await write(record);
  };
  const sent = [];
  const controller = new SourceBulkActions(identity, disk, prepare, async (step) =>
    sent.push(step.command),
  );
  await controller.load();
  await controller.start(selected, { kind: 'search', value: true });
  assert.deepEqual(
    controller.getSnapshot().record.steps.map((item) => item.status),
    ['pending', 'ready', 'ready'],
  );
  assert.equal(sent.length, 1);
  fail = false;
  await controller.resume();
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(sent.length, 4);
  assert.ok(controller.getSnapshot().record.steps.every((item) => item.status === 'done'));
});

test('pending storage is isolated by local instance and validates exact action, targets and response', async () => {
  const disk = storage();
  const controller = new SourceBulkActions(identity, disk, prepare, async () => {
    throw new Error('lost reply');
  });
  await controller.load();
  await controller.start(selected, { kind: 'search', value: false });
  const record = disk.get();
  assert.equal(record.key, sourceBulkKey(identity));
  validateSourceBulk(record, identity);
  assert.throws(
    () => validateSourceBulk(record, { ...identity, local_instance_id: 'restored-copy' }),
    /Invalid document batch/,
  );
  for (const mutate of [
    (value) => {
      value.steps[0].command.payload.request.object_id = 'another-source';
    },
    (value) => {
      value.steps[0].expected = 'source_mutation';
    },
    (value) => {
      value.steps[0].command.payload.request.value = true;
    },
    (value) => {
      value.steps[1].command.payload.command_id = value.steps[0].command.payload.command_id;
    },
    (value) => {
      value.selected.push(value.selected[0]);
    },
  ]) {
    const changed = structuredClone(record);
    mutate(changed);
    assert.throws(() => validateSourceBulk(changed, identity), /Invalid document batch/);
  }
  let sends = 0;
  const other = new SourceBulkActions(
    { ...identity, workspace_id: 'other' },
    disk,
    prepare,
    async () => sends++,
  );
  await other.load();
  await other.resume();
  assert.equal(other.getSnapshot().ready, false);
  assert.equal(sends, 0);
});

test('a selected batch is frozen before asynchronous metadata reads', async () => {
  const disk = storage();
  const selection = structuredClone(selected);
  const action = { kind: 'tag', name: '原始标签' };
  let release;
  const controller = new SourceBulkActions(
    identity,
    disk,
    async (items, chosen) => {
      await new Promise((resolve) => {
        release = resolve;
      });
      return prepare(items, chosen);
    },
    async () => {
      throw new Error('retain original');
    },
  );
  await controller.load();
  const started = controller.start(selection, action);
  selection[0].id = 'changed';
  action.name = 'later label';
  release();
  await started;
  assert.equal(disk.get().selected[0].id, selected[0].id);
  assert.equal(disk.get().steps[0].command.payload.request.tag.name, '原始标签');
});
