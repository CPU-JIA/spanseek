import test from 'node:test';
import assert from 'node:assert/strict';
import { PageActions, pageKey } from '../src/organization/page-actions.ts';
import { ScopedQuery } from '../src/organization/scoped-query.ts';
import { reorderedMembers } from '../src/organization/collection-order.ts';

const identity = { local_instance_id: 'local-a', workspace_id: 'logical-a' };
const command = () => ({
  kind: 'collections_create',
  payload: {
    command_id: crypto.randomUUID(),
    request: { name: '研究', overview_note_id: null, is_pinned: false, pin_position: null },
  },
});
const fault = (code) => Object.assign(new Error(code), { details: { code } });
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
function fixture(area = 'collections') {
  const records = new Map();
  const calls = [];
  const receipts = new Map();
  const f = {
    records,
    calls,
    receipts,
    effects: 0,
    writeFail: false,
    removeFail: false,
    lostReceipt: false,
  };
  f.storage = {
    async read(key) {
      return structuredClone(records.get(key));
    },
    async write(intent) {
      if (f.writeFail) throw Error('storage full');
      records.set(intent.key, structuredClone(intent));
    },
    async remove(key) {
      if (f.removeFail) throw Error('cleanup failure');
      records.delete(key);
    },
  };
  f.send = async (request) => {
    assert.ok(records.has(pageKey(identity, area)), 'exact intent must be durable before sending');
    calls.push(structuredClone(request));
    const id = request.payload.command_id;
    if (!receipts.has(id)) {
      f.effects++;
      receipts.set(id, { collection: { id: `created-${f.effects}` }, receipt: { command_id: id } });
    }
    if (f.lostReceipt) throw Error('connection lost after commit');
    return structuredClone(receipts.get(id));
  };
  f.open = () => PageActions.open(identity, area, f.storage, (request) => f.send(request));
  return f;
}

test('lost collection creation receipt survives reopening and retries the exact ID without duplicate creation', async () => {
  const f = fixture();
  f.lostReceipt = true;
  const request = command();
  const first = await f.open();
  assert.equal(await first.run(request), null);
  request.payload.request.name = 'later edit must not alter saved intent';
  const reopened = await f.open();
  assert.equal(reopened.getSnapshot().pending.command.payload.request.name, '研究');
  assert.equal(
    await reopened.run(command()),
    null,
    'a new command cannot replace the unresolved command',
  );
  f.lostReceipt = false;
  assert.ok(await reopened.run());
  assert.deepEqual(f.calls[0], f.calls[1]);
  assert.equal(f.effects, 1);
  assert.equal(reopened.getSnapshot().pending, null);
});

test('failed local persistence never sends, and the same command is used after storage recovers', async () => {
  const f = fixture();
  f.writeFail = true;
  const action = await f.open();
  const request = command();
  assert.equal(await action.run(request), null);
  assert.equal(f.calls.length, 0);
  assert.equal(action.getSnapshot().pending.command.payload.command_id, request.payload.command_id);
  f.writeFail = false;
  await action.run();
  assert.deepEqual(f.calls[0], request);
});

test('canvas layout, deletion and relation writes retain the exact request after a lost receipt', async () => {
  for (const request of [
    {
      kind: 'canvases_save',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          canvas_id: null,
          base_row_version: null,
          name: '新画布',
          nodes: [{ note_id: 'note-a', x: 12, y: 30, width: 200, height: 100, z_index: 2 }],
          config_schema: 1,
          is_pinned: false,
          pin_position: null,
        },
      },
    },
    {
      kind: 'canvases_remove',
      payload: {
        command_id: crypto.randomUUID(),
        request: { canvas_id: 'canvas-a', base_row_version: 9 },
      },
    },
    {
      kind: 'notes_save',
      payload: {
        command_id: crypto.randomUUID(),
        note: {
          note_id: 'note-a',
          base_revision_id: 'rev-1',
          title: '关系来源',
          document_schema: 3,
          document: { type: 'doc', content: [] },
        },
      },
    },
  ]) {
    const f = fixture('canvas');
    f.lostReceipt = true;
    const action = await f.open();
    const expected = structuredClone(request);
    assert.equal(await action.run(request), null);
    action.clearError();
    assert.ok(action.getSnapshot().pending);
    request.payload.command_id = 'later-input';
    const reopened = await f.open();
    f.lostReceipt = false;
    assert.ok(await reopened.run());
    assert.deepEqual(f.calls, [expected, expected]);
    assert.equal(f.effects, 1);
  }
});

test('canvas relation revision conflicts are terminal and other page areas cannot send canvas writes', async () => {
  const f = fixture('canvas');
  f.send = async () => {
    throw fault('revision_conflict');
  };
  const action = await f.open();
  await action.run({ kind: 'notes_save', payload: { command_id: crypto.randomUUID(), note: {} } });
  assert.equal(action.getSnapshot().code, 'revision_conflict');
  assert.equal(action.getSnapshot().pending, null);
  assert.equal(f.records.size, 0);
  action.clearError();
  assert.equal(action.getSnapshot().error, null);
  const other = fixture('collections');
  const wrongArea = await other.open();
  assert.equal(
    await wrongArea.run({
      kind: 'canvases_remove',
      payload: { command_id: crypto.randomUUID(), request: {} },
    }),
    null,
  );
  assert.equal(other.calls.length, 0);
  assert.equal(other.records.size, 0);
});

test('saved view update preserves exact filters, base version, sort and layout across an unknown result', async () => {
  const f = fixture('views');
  f.lostReceipt = true;
  const request = {
    kind: 'views_save',
    payload: {
      command_id: crypto.randomUUID(),
      request: {
        view_id: 'view-a',
        base_row_version: 4,
        name: '阅读',
        filter_schema: 1,
        filter: {
          tag_ids: ['tag-a', 'tag-b'],
          inbox_state: 'pending',
          collection_id: 'collection-a',
          archived: false,
          search_enabled: true,
        },
        sort: 'title_desc',
        layout: 'table',
        is_pinned: true,
        pin_position: 0,
      },
    },
  };
  const action = await f.open();
  await action.run(request);
  const expected = structuredClone(request);
  request.payload.request.filter.tag_ids.pop();
  request.payload.request.base_row_version = 99;
  const reopened = await f.open();
  f.lostReceipt = false;
  await reopened.run();
  assert.deepEqual(f.calls[1], expected);
  assert.equal(f.effects, 1);
  assert.equal(f.records.size, 0);
});

test('failure to remove a confirmed intent leaves it retryable and does not allow a second mutation', async () => {
  const f = fixture();
  f.removeFail = true;
  const action = await f.open();
  await action.run(command());
  assert.equal(action.getSnapshot().generation, 0);
  assert.equal(f.effects, 1);
  assert.equal(await action.run(command()), null);
  assert.equal(f.calls.length, 1);
  f.removeFail = false;
  const reopened = await f.open();
  await reopened.run();
  assert.equal(f.effects, 1);
  assert.deepEqual(f.calls[0], f.calls[1]);
  assert.equal(reopened.getSnapshot().generation, 1);
});

test('explicit metadata conflict is exposed and never retried with a changed base version', async () => {
  const f = fixture();
  const request = {
    kind: 'collections_update',
    payload: {
      command_id: crypto.randomUUID(),
      request: {
        collection_id: 'collection-a',
        base_row_version: 3,
        name: 'new',
        overview_note_id: null,
        is_pinned: false,
        pin_position: null,
      },
    },
  };
  f.send = async (value) => {
    f.calls.push(structuredClone(value));
    throw fault('metadata_conflict');
  };
  const action = await f.open();
  await action.run(request);
  assert.equal(action.getSnapshot().code, 'metadata_conflict');
  assert.equal(action.getSnapshot().pending, null);
  assert.equal(f.records.size, 0);
  assert.equal(await action.run(), null);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].payload.request.base_row_version, 3);
});

test('another workspace identity cannot adopt an existing pending command', async () => {
  const f = fixture();
  f.lostReceipt = true;
  const action = await f.open();
  await action.run(command());
  const pending = [...f.records.values()][0];
  const storage = { ...f.storage, read: async () => pending };
  await assert.rejects(
    PageActions.open(
      { ...identity, local_instance_id: 'restored-copy' },
      'collections',
      storage,
      f.send,
    ),
    /校验失败/,
  );
  assert.equal(f.calls.length, 1);
});

test('concurrent actions cannot overwrite an intent while its durable write is pending', async () => {
  const f = fixture();
  const gate = deferred();
  const write = f.storage.write;
  f.storage.write = async (intent) => {
    await gate.promise;
    await write(intent);
  };
  const action = await f.open();
  const first = command();
  const running = action.run(first);
  assert.equal(await action.run(command()), null);
  assert.equal(f.calls.length, 0);
  gate.resolve();
  await running;
  assert.equal(f.calls.length, 1);
  assert.deepEqual(f.calls[0], first);
});

test('read responses cannot replace a newer refresh or publish after query disposal', async () => {
  const older = deferred();
  const newer = deferred();
  let count = 0;
  const query = new ScopedQuery(() => (++count === 1 ? older.promise : newer.promise));
  const a = query.load();
  const b = query.load();
  newer.resolve({ items: ['new'], next_cursor: null });
  await b;
  older.resolve({ items: ['old'], next_cursor: 'old-cursor' });
  await a;
  assert.deepEqual(query.getSnapshot().page.items, ['new']);
  const late = deferred();
  const disposed = new ScopedQuery(() => late.promise);
  const running = disposed.load();
  disposed.invalidate();
  late.resolve({ items: ['wrong-workspace'], next_cursor: null });
  await running;
  assert.equal(disposed.getSnapshot().page, null);
});

test('expired pagination restarts from the first page without appending it to the old page', async () => {
  const calls = [];
  let first = true;
  const query = new ScopedQuery(async (cursor) => {
    calls.push(cursor);
    if (cursor) throw fault('query_expired');
    if (first) {
      first = false;
      return { items: ['old'], next_cursor: 'c' };
    }
    return { items: ['current'], next_cursor: null };
  });
  await query.load();
  await query.load(true);
  assert.deepEqual(calls, [null, 'c', null]);
  assert.deepEqual(query.getSnapshot().page.items, ['current']);
  assert.equal(query.getSnapshot().error, null);
});

test('member reordering includes every page and trashed members', async () => {
  const member = (id, trashed = false) => ({ object: { id, trashed } });
  const pages = {
    first: {
      collection_row_version: 8,
      items: [member('a'), member('trash', true)],
      next_cursor: 'next',
    },
    next: { collection_row_version: 8, items: [member('b')], next_cursor: null },
  };
  const ids = await reorderedMembers(async (cursor) => pages[cursor ?? 'first'], 8, 'b', -1);
  assert.deepEqual(ids, ['a', 'b', 'trash']);
});

test('member reordering rejects version changes and duplicated members before any command is sent', async () => {
  let calls = 0;
  await assert.rejects(
    reorderedMembers(
      async () => ({
        collection_row_version: ++calls === 1 ? 8 : 9,
        items: [{ object: { id: `note-${calls}` } }],
        next_cursor: calls === 1 ? 'next' : null,
      }),
      8,
      'note-1',
      1,
    ),
    /已有更新/,
  );
  await assert.rejects(
    reorderedMembers(
      async () => ({
        collection_row_version: 8,
        items: [{ object: { id: 'a' } }, { object: { id: 'a' } }],
        next_cursor: null,
      }),
      8,
      'a',
      1,
    ),
    /成员已变化/,
  );
});
