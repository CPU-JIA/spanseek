import test from 'node:test';
import assert from 'node:assert/strict';
import { withoutNote } from '../src/organization/note-lifecycle.ts';
import { PageActions } from '../src/organization/page-actions.ts';

test('recycling removes the note from both panes and keeps unrelated tabs and column preferences', () => {
  const pane = (ids, active) => ({
    tabs: ids.map((object_id) => ({ object_id, kind: 'note' })),
    active_object_id: active,
    scroll_offsets: ids.map((object_id) => ({ object_id, x: 0, y: 50 })),
  });
  const original = {
    panes: [pane(['keep', 'trash'], 'trash'), pane(['trash'], 'trash')],
    active_pane: 1,
    sidebar_width: 220,
    note_list_width: 310,
  };
  const result = withoutNote(original, 'trash');
  assert.equal(result.panes[0].active_object_id, 'keep');
  assert.equal(result.panes[1].active_object_id, null);
  assert.deepEqual(result.panes[1].tabs, []);
  assert.deepEqual(result.panes[0].scroll_offsets, [{ object_id: 'keep', x: 0, y: 50 }]);
  assert.equal(result.sidebar_width, 220);
  assert.equal(original.panes[0].tabs.length, 2);
});

test('trash intent survives a lost receipt and reopening; restore cannot replace the pending deletion', async () => {
  const identity = { local_instance_id: 'local', workspace_id: 'workspace' };
  let stored,
    applied = 0,
    lost = true;
  const received = [];
  const storage = {
    read: async () => structuredClone(stored),
    write: async (value) => {
      stored = structuredClone(value);
    },
    remove: async () => {
      stored = undefined;
    },
  };
  const receipts = new Map();
  const send = async (command) => {
    assert.ok(stored);
    received.push(structuredClone(command));
    const id = command.payload.command_id;
    if (!receipts.has(id)) {
      applied++;
      receipts.set(id, { receipt: { command_id: id } });
    }
    if (lost) throw Error('lost receipt');
    return receipts.get(id);
  };
  const command = {
    kind: 'objects_set_trashed',
    payload: {
      command_id: crypto.randomUUID(),
      request: { object_id: 'note', base_row_version: 3, value: true },
    },
  };
  const first = await PageActions.open(identity, 'note-trash', storage, send);
  assert.equal(await first.run(command), null);
  const reopened = await PageActions.open(identity, 'note-trash', storage, send);
  assert.equal(
    await reopened.run({
      ...command,
      payload: { ...command.payload, command_id: crypto.randomUUID() },
    }),
    null,
  );
  lost = false;
  assert.ok(await reopened.run());
  assert.deepEqual(received[0], received[1]);
  assert.equal(applied, 1);
  assert.equal(stored, undefined);
});
