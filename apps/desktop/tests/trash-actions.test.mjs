import test from 'node:test';
import assert from 'node:assert/strict';
import { TrashActions, trashKey } from '../src/organization/trash-actions.ts';
import { nextTrashFocus, withoutObjects } from '../src/organization/note-lifecycle.ts';

const identity = { local_instance_id: 'isolated-local', workspace_id: 'isolated-workspace' };
const item = (object_id, kind = 'note', extra = {}) => ({
  object_id,
  kind,
  title: `Fixture ${object_id}`,
  row_version: 3,
  preview: 'Synthetic fixture only',
  updated_at: '2026-10-10T08:00:00Z',
  parent_source_id: null,
  source_trashed: false,
  ...extra,
});
const preview = (
  items = [item('a')],
  selection = { kind: 'objects', object_ids: items.map((entry) => entry.object_id) },
) => ({
  selection,
  snapshot_token: 'frozen-fixture-snapshot',
  items,
  selected_count: items.length,
  dependent_count: 0,
  total_count: items.length,
});
function storage() {
  const records = new Map();
  return {
    records,
    read: async (key) => structuredClone(records.get(key)),
    write: async (record) => {
      records.set(record.key, structuredClone(record));
    },
    remove: async (key) => {
      records.delete(key);
    },
  };
}
function serviceError(code) {
  return Object.assign(new Error(code), { details: { code } });
}
function deleteCommit(command, items) {
  return {
    deleted_objects: items.map(({ object_id, kind }) => ({ object_id, kind })),
    receipt: { command_id: command.payload.command_id },
  };
}
function restoreCommit(command) {
  return {
    objects: [{ id: command.payload.request.object_id, trashed: false }],
    receipt: { command_id: command.payload.command_id },
  };
}
const unusedRestore = async () => {
  throw Error('unexpected restore');
};
const unusedDelete = async () => {
  throw Error('unexpected delete');
};

test('permanent deletion persists the exact confirmed scope before send and preserves a lost receipt across restart', async () => {
  const local = storage();
  const confirmed = preview([item('a'), item('b')], { kind: 'all' });
  const received = [];
  const receipts = new Map();
  let loseResponse = true;
  const transport = {
    restore: unusedRestore,
    delete: async (command) => {
      const persisted = local.records.get(trashKey(identity));
      assert.deepEqual(persisted.command, command);
      assert.equal(persisted.preview.total_count, 2);
      received.push(structuredClone(command));
      if (!receipts.has(command.payload.command_id))
        receipts.set(command.payload.command_id, deleteCommit(command, confirmed.items));
      if (loseResponse) throw Error('receipt was lost after commit');
      return receipts.get(command.payload.command_id);
    },
  };
  const first = await TrashActions.open(identity, local, transport);
  assert.equal(await first.delete(confirmed), null);
  assert.ok(first.getSnapshot().pending);
  assert.equal(await first.delete(preview([item('new-item')])), null);
  const reopened = await TrashActions.open(identity, local, transport);
  assert.equal(await reopened.restore([item('different')]), null);
  loseResponse = false;
  const result = await reopened.retry();
  assert.equal(receipts.size, 1);
  assert.deepEqual(received[0], received[1]);
  assert.deepEqual(
    result.items.map((entry) => entry.object_id),
    ['a', 'b'],
  );
  assert.equal(reopened.getSnapshot().pending, null);
  assert.equal(local.records.size, 0);
});

test('a local persistence failure prevents a delete from being sent', async () => {
  const local = storage();
  local.write = async () => {
    throw Error('disk unavailable');
  };
  let sends = 0;
  const controller = await TrashActions.open(identity, local, {
    restore: unusedRestore,
    delete: async (command) => {
      sends++;
      return deleteCommit(command, [item('a')]);
    },
  });
  assert.equal(await controller.delete(preview()), null);
  assert.equal(sends, 0);
  assert.ok(controller.getSnapshot().pending);
  local.write = async (record) => {
    local.records.set(record.key, structuredClone(record));
  };
  assert.ok(await controller.retry());
  assert.equal(sends, 1);
});

test('known commit plus failed local cleanup returns confirmed IDs while retaining the lifecycle barrier', async () => {
  const local = storage();
  const remove = local.remove;
  local.remove = async () => {
    throw Error('cannot clear confirmation');
  };
  let sends = 0;
  const transport = {
    restore: unusedRestore,
    delete: async (command) => {
      sends++;
      return deleteCommit(command, [item('a')]);
    },
  };
  const controller = await TrashActions.open(identity, local, transport);
  const result = await controller.delete(preview());
  assert.equal(result.items[0].status, 'done');
  assert.ok(controller.getSnapshot().pending.commit);
  const reopened = await TrashActions.open(identity, local, transport);
  assert.equal(reopened.getSnapshot().result.items[0].object_id, 'a');
  local.remove = remove;
  assert.ok(await reopened.retry());
  assert.equal(sends, 1, 'a persisted confirmation only needs local cleanup');
  assert.equal(reopened.getSnapshot().pending, null);
});

test('a changed deletion snapshot fails without broadening scope and requires a new explicit preview', async () => {
  const local = storage();
  let sends = 0;
  const controller = await TrashActions.open(identity, local, {
    restore: unusedRestore,
    delete: async () => {
      sends++;
      throw serviceError('query_expired');
    },
  });
  assert.equal(await controller.delete(preview([item('a')], { kind: 'all' })), null);
  assert.equal(controller.getSnapshot().pending, null);
  assert.equal(controller.getSnapshot().code, 'query_expired');
  assert.equal(await controller.retry(), null);
  assert.equal(sends, 1);
  assert.equal(local.records.size, 0);
});

test('a failed local draft cleanup retains the core receipt and restart retries cleanup without another delete', async () => {
  const local = storage();
  let sends = 0;
  let cleanupCalls = 0;
  let cleanupFails = true;
  const transport = {
    restore: unusedRestore,
    delete: async (command) => {
      sends++;
      return deleteCommit(command, [item('a'), item('source', 'source')]);
    },
    finalizeDelete: async (commit) => {
      cleanupCalls++;
      assert.deepEqual(
        commit.deleted_objects.map((entry) => entry.object_id),
        ['a', 'source'],
      );
      assert.ok(
        local.records.get(trashKey(identity)).commit,
        'persist core receipt before cross-storage cleanup',
      );
      if (cleanupFails) throw Error('local draft storage unavailable');
    },
  };
  const first = await TrashActions.open(identity, local, transport);
  const result = await first.delete(preview([item('a'), item('source', 'source')]));
  assert.equal(result.items.length, 2);
  assert.ok(first.getSnapshot().pending.commit);
  assert.equal(first.getSnapshot().error, 'local draft storage unavailable');
  const reopened = await TrashActions.open(identity, local, transport);
  cleanupFails = false;
  await reopened.retry();
  assert.equal(sends, 1);
  assert.equal(cleanupCalls, 2);
  assert.equal(reopened.getSnapshot().pending, null);
  assert.equal(local.records.size, 0);
});

test('terminal rejection with failed local cleanup is kept and cleanup retry does not resend', async () => {
  const local = storage();
  const remove = local.remove;
  local.remove = async () => {
    throw Error('cleanup failed');
  };
  let sends = 0;
  const controller = await TrashActions.open(identity, local, {
    restore: unusedRestore,
    delete: async () => {
      sends++;
      throw serviceError('query_expired');
    },
  });
  await controller.delete(preview());
  assert.equal(controller.getSnapshot().pending.rejection.code, 'query_expired');
  local.remove = remove;
  await controller.retry();
  assert.equal(sends, 1);
  assert.equal(controller.getSnapshot().code, 'query_expired');
  assert.equal(controller.getSnapshot().pending, null);
});

test('mismatched deletion receipt never unlocks the lifecycle or claims a result', async () => {
  const local = storage();
  const controller = await TrashActions.open(identity, local, {
    restore: unusedRestore,
    delete: async (command) => deleteCommit(command, [item('wrong')]),
  });
  assert.equal(await controller.delete(preview()), null);
  assert.ok(controller.getSnapshot().pending);
  assert.equal(controller.getSnapshot().result, null);
});

test('restore processes sources before excerpts and reports conflicts without repeating successful objects', async () => {
  const local = storage();
  const source = item('source', 'source');
  const excerpt = item('excerpt', 'annotation', {
    source_trashed: true,
    parent_source_id: source.object_id,
  });
  const sent = [];
  const controller = await TrashActions.open(identity, local, {
    delete: unusedDelete,
    restore: async (command) => {
      assert.ok(local.records.get(trashKey(identity)));
      sent.push(command.payload.request.object_id);
      if (command.payload.request.object_id === 'conflict') throw serviceError('metadata_conflict');
      return restoreCommit(command);
    },
  });
  const result = await controller.restore([excerpt, item('conflict'), source, item('other')]);
  assert.equal(sent[0], 'source');
  assert.deepEqual(
    result.items.filter((entry) => entry.status === 'done').map((entry) => entry.object_id),
    ['source', 'excerpt', 'other'],
  );
  assert.equal(result.items.find((entry) => entry.object_id === 'conflict').status, 'failed');
  assert.equal(controller.getSnapshot().pending, null);
  assert.equal(await controller.retry(), null);
  assert.equal(sent.length, 4);
});

test('a source restore conflict leaves its excerpt untouched and reports the dependency', async () => {
  const local = storage();
  const source = item('source', 'source');
  const excerpt = item('excerpt', 'annotation', {
    source_trashed: true,
    parent_source_id: 'source',
  });
  const sent = [];
  const controller = await TrashActions.open(identity, local, {
    delete: unusedDelete,
    restore: async (command) => {
      sent.push(command.payload.request.object_id);
      throw serviceError('metadata_conflict');
    },
  });
  const result = await controller.restore([excerpt, source]);
  assert.deepEqual(sent, ['source']);
  assert.equal(result.items.find((entry) => entry.object_id === 'excerpt').code, 'object_trashed');
});

test('an unknown restore result stops the batch and restart retries the exact command before continuing', async () => {
  const local = storage();
  let lost = true;
  const sent = [];
  const transport = {
    delete: unusedDelete,
    restore: async (command) => {
      sent.push(structuredClone(command));
      if (command.payload.request.object_id === 'b' && lost) throw Error('unknown commit result');
      return restoreCommit(command);
    },
  };
  const first = await TrashActions.open(identity, local, transport);
  await first.restore([item('a'), item('b'), item('c')]);
  assert.deepEqual(
    sent.map((command) => command.payload.request.object_id),
    ['a', 'b'],
  );
  assert.equal(first.getSnapshot().result.items.length, 1);
  const reopened = await TrashActions.open(identity, local, transport);
  lost = false;
  const result = await reopened.retry();
  assert.deepEqual(sent[1], sent[2]);
  assert.deepEqual(
    sent.map((command) => command.payload.request.object_id),
    ['a', 'b', 'b', 'c'],
  );
  assert.equal(result.items.length, 3);
  assert.equal(local.records.size, 0);
});

test('a failed acknowledgement write preserves the exact restore command for replay after restart', async () => {
  const local = storage();
  const write = local.write;
  let failAcknowledgedWrite = true;
  local.write = async (record) => {
    if (failAcknowledgedWrite && record.kind === 'restore' && record.items[0].status === 'done')
      throw Error('ack disk write failed');
    return write(record);
  };
  const sent = [];
  const transport = {
    delete: unusedDelete,
    restore: async (command) => {
      sent.push(structuredClone(command));
      return restoreCommit(command);
    },
  };
  const first = await TrashActions.open(identity, local, transport);
  const known = await first.restore([item('a')]);
  assert.equal(known.items[0].status, 'done');
  assert.equal(local.records.get(trashKey(identity)).items[0].status, 'pending');
  failAcknowledgedWrite = false;
  const reopened = await TrashActions.open(identity, local, transport);
  await reopened.retry();
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(reopened.getSnapshot().pending, null);
});

test('invalid scopes, duplicate IDs, and a pending intent from another local workspace cannot be sent', async () => {
  const local = storage();
  const controller = await TrashActions.open(identity, local, {
    delete: unusedDelete,
    restore: unusedRestore,
  });
  await assert.rejects(controller.delete({ ...preview(), total_count: 2 }), /invalid/);
  await assert.rejects(controller.restore([item('a'), item('a')]), /invalid/);
  await assert.rejects(
    controller.restore([
      item('excerpt', 'annotation', { source_trashed: true, parent_source_id: 'parent' }),
    ]),
    /source first/,
  );
  await controller.delete(preview());
  const pending = local.records.get(trashKey(identity));
  const tampered = { ...pending, identity: { ...identity, local_instance_id: 'other-local' } };
  await assert.rejects(
    TrashActions.open(
      identity,
      { ...local, read: async () => tampered },
      { delete: unusedDelete, restore: unusedRestore },
    ),
    /invalid/,
  );
  assert.equal(
    (
      await TrashActions.open({ ...identity, local_instance_id: 'other-local' }, local, {
        delete: unusedDelete,
        restore: unusedRestore,
      })
    ).getSnapshot().pending,
    null,
  );
});

test('confirmed mixed-object deletion removes both panes and picks the nearest remaining tab', () => {
  const pane = (ids, active) => ({
    tabs: ids.map((object_id) => ({ object_id, kind: object_id === 'source' ? 'source' : 'note' })),
    active_object_id: active,
    scroll_offsets: ids.map((object_id) => ({ object_id, x: 0, y: 20 })),
  });
  const layout = {
    panes: [
      pane(['before', 'source', 'after', 'last'], 'source'),
      pane(['source', 'note'], 'note'),
    ],
    active_pane: 0,
    sidebar_width: 256,
    note_list_width: 280,
  };
  const next = withoutObjects(layout, ['source', 'note']);
  assert.equal(next.panes[0].active_object_id, 'after');
  assert.equal(next.panes[1].active_object_id, null);
  assert.deepEqual(next.panes[1].tabs, []);
  assert.deepEqual(
    next.panes[0].scroll_offsets.map((entry) => entry.object_id),
    ['before', 'after', 'last'],
  );
  assert.equal(next.sidebar_width, 256);
  assert.equal(layout.panes[1].tabs.length, 2);
  assert.equal(nextTrashFocus(['a', 'b', 'c'], ['b'], 'b'), 'c');
  assert.equal(nextTrashFocus(['a', 'b', 'c'], ['b', 'c'], 'c'), 'a');
  assert.equal(nextTrashFocus(['a'], ['a'], 'a'), null);
});
