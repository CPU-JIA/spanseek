import test from 'node:test';
import assert from 'node:assert/strict';
import { createIdentity, syncMobile } from '../sync.js';

const encoder = new TextEncoder();
const updatedAt = '2024-02-29T12:34:56.123456789+08:00';

function canonical(parts) {
  return Buffer.concat(
    parts.flatMap((part) => {
      const bytes = typeof part === 'string' ? Buffer.from(part, 'utf8') : Buffer.from(part);
      const size = Buffer.alloc(8);
      size.writeBigUInt64BE(BigInt(bytes.length));
      return [size, bytes];
    }),
  );
}

function i64(value) {
  const bytes = Buffer.alloc(8);
  bytes.writeBigInt64BE(BigInt(value));
  return bytes;
}

function note(text = '保留原始正文', schema = 3) {
  return {
    title: `正文版本 ${schema}`,
    document_schema: schema,
    document: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          attrs: { id: crypto.randomUUID() },
          content: text ? [{ type: 'text', text }] : [],
        },
      ],
    },
    updated_at: updatedAt,
  };
}

async function fixture(t, { pageSize = 200, batchOverride } = {}) {
  const identity = await createIdentity('controlled receive fixture');
  const rawKey = crypto.getRandomValues(new Uint8Array(32));
  const key = await crypto.subtle.importKey('raw', rawKey, 'AES-GCM', false, [
    'encrypt',
    'decrypt',
  ]);
  const profile = {
    ...identity,
    endpoint: 'http://controlled-sync.invalid',
    workspaceId: crypto.randomUUID(),
    deviceId: crypto.randomUUID(),
    epoch: 1,
    keys: { 1: Buffer.from(rawKey).toString('base64') },
    cursor: 0,
  };
  let operations = [];
  const pullRequests = [];
  const pushes = [];
  t.mock.method(globalThis, 'fetch', async (url, init) => {
    const body = JSON.parse(init.body);
    const path = new URL(url).pathname;
    if (path === '/v1/keys/epoch') return Response.json(1);
    if (path === '/v1/pull') {
      pullRequests.push(body.after_seq);
      const remaining = operations.filter((op) => op.server_seq > body.after_seq);
      const ops = remaining.slice(0, Math.min(pageSize, body.limit));
      const batch = {
        ops,
        next_cursor: ops.at(-1)?.server_seq ?? body.after_seq,
        has_more: remaining.length > ops.length,
        conflicts: [],
      };
      return Response.json(batchOverride ? batchOverride(batch) : batch);
    }
    if (path === '/v1/ops') {
      pushes.push(body.op);
      return Response.json({ status: 'applied', version: 1 });
    }
    throw new Error(`Unexpected controlled request: ${path}`);
  });
  async function op(payload, serverSeq, overrides = {}) {
    const objectId = overrides.object_id ?? crypto.randomUUID();
    const nonce = crypto.getRandomValues(new Uint8Array(12));
    const bytes = payload instanceof Uint8Array ? payload : encoder.encode(JSON.stringify(payload));
    const cipher = await crypto.subtle.encrypt(
      {
        name: 'AES-GCM',
        iv: nonce,
        additionalData: canonical([
          'spanseek-sync-object-aad-v1',
          profile.workspaceId,
          objectId,
          i64(1),
        ]),
      },
      key,
      bytes,
    );
    return {
      server_seq: serverSeq,
      op_id: crypto.randomUUID(),
      workspace_id: profile.workspaceId,
      object_id: objectId,
      kind: 'put',
      base_version: 0,
      resulting_version: 1,
      device_id: crypto.randomUUID(),
      key_epoch: 1,
      ciphertext: Buffer.concat([Buffer.from(nonce), Buffer.from(cipher)]).toString('base64'),
      tombstone: false,
      purged: false,
      deleted_at: null,
      deleted_by: null,
      client_time: updatedAt,
      ...overrides,
    };
  }
  return {
    profile,
    op,
    pullRequests,
    pushes,
    setOps(value) {
      operations = value;
    },
  };
}

test('mixed configuration and D1/D2/D3 operations create only actual notes and retain original timestamps', async (t) => {
  const server = await fixture(t, { pageSize: 2 });
  const kinds = ['view', 'canvas', 'workspace_metadata', 'tag', 'tag_assignments'];
  const configs = await Promise.all(
    kinds.map((sync_kind, index) =>
      server.op({ ...note('configuration must not become a note'), sync_kind }, index * 2 + 1),
    ),
  );
  const notes = await Promise.all(
    [1, 2, 3].map((schema, index) => server.op(note(`有效旧版本 ${schema}`, schema), 12 + index)),
  );
  const empty = await server.op({ ...note(''), title: '' }, 20);
  server.setOps([...configs, ...notes, empty]);
  const captures = [];
  const result = await syncMobile(server.profile, [], {
    onRemote: (value) => {
      captures.push(value);
    },
  });
  assert.deepEqual(
    result.remoteChanges,
    [...notes, empty].map((op) => op.object_id),
  );
  assert.deepEqual(
    captures.map((value) => value.body),
    ['有效旧版本 1', '有效旧版本 2', '有效旧版本 3', ''],
  );
  assert.ok(captures.every((value) => value.updatedAt === updatedAt));
  assert.equal(captures.at(-1).title, '');
  assert.equal(result.profile.cursor, 20);
  assert.equal(server.profile.cursor, 0, 'the caller-owned profile is not mutated');
  assert.deepEqual(result.conflicts, []);
  assert.equal(result.receiveBlocked, false);
});

test('unknown discriminators never fall through to note parsing, and retry starts after only the confirmed prefix', async (t) => {
  const server = await fixture(t);
  const first = await server.op(note('先成功'), 3);
  const blocked = await server.op({ ...note('看起来像笔记'), sync_kind: 'future_entity' }, 8);
  const last = await server.op(note('尚未处理'), 10);
  server.setOps([first, blocked, last]);
  const received = [];
  const reported = [];
  const hooks = {
    onRemote: (value) => {
      received.push(value.objectId);
    },
    onConflict: (value) => {
      reported.push(value);
    },
  };
  const initial = await syncMobile(server.profile, [], hooks);
  assert.deepEqual(received, [first.object_id]);
  assert.equal(initial.profile.cursor, 3);
  assert.equal(initial.receiveBlocked, true);
  assert.equal(initial.conflicts[0].code, 'unsupported_sync_payload');
  assert.deepEqual(reported, initial.conflicts);
  // Controlled fixture supplies a supported payload at the same unacknowledged
  // operation, analogous to correcting the input or upgrading the receiver.
  const corrected = await server.op(note('现在可读取'), 8, {
    object_id: blocked.object_id,
    op_id: blocked.op_id,
  });
  server.setOps([first, corrected, last]);
  const resumed = await syncMobile(initial.profile, [], hooks);
  assert.deepEqual(server.pullRequests, [0, 3]);
  assert.deepEqual(resumed.remoteChanges, [blocked.object_id, last.object_id]);
  assert.equal(resumed.profile.cursor, 10);
  assert.equal(resumed.receiveBlocked, false);
});

test('malformed note envelopes and document structures never manufacture title, body, or time', async (t) => {
  const server = await fixture(t);
  const cases = [
    null,
    [],
    { ...note(), title: null },
    { ...note(), updated_at: undefined },
    { ...note(), updated_at: 'yesterday' },
    { ...note(), updated_at: '2026-02-30T12:00:00Z' },
    { ...note(), document_schema: '3' },
    { ...note(), document_schema: 4 },
    { ...note(), document: undefined },
    { ...note(), document: { type: 'doc', content: [] } },
    { ...note(), document: { type: 'doc', content: {} } },
    {
      ...note(),
      document: { type: 'doc', content: [{ type: 'text', text: 'invalid top-level node' }] },
    },
    {
      ...note(),
      document: { type: 'doc', content: [{ type: 'paragraph', text: 'not valid paragraph text' }] },
    },
    {
      ...note(),
      document: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text' }] }] },
    },
    {
      ...note(),
      document: { type: 'doc', content: [{ type: 'paragraph', content: 'not children' }] },
    },
    { ...note(), document: { type: 'doc', content: [{ type: 'paragraph', attrs: null }] } },
    { ...note(), document: { type: 'doc', content: [{ type: 'unknownBlock', content: [] }] } },
    { ...note(), sync_kind: null },
    {
      ...note(),
      document: {
        type: 'doc',
        content: [
          {
            type: 'paragraph',
            content: [{ type: 'text', text: 'unknown format', marks: [{ type: 'futureMark' }] }],
          },
        ],
      },
    },
    {
      ...note('older body', 2),
      document: {
        type: 'doc',
        content: [
          {
            type: 'paragraph',
            content: [
              { type: 'text', text: 'newer mark in old schema', marks: [{ type: 'underline' }] },
            ],
          },
        ],
      },
    },
  ];
  for (const payload of cases) {
    server.setOps([await server.op(payload, 1)]);
    const received = [];
    const result = await syncMobile(server.profile, [], {
      onRemote: (value) => {
        received.push(value);
      },
    });
    assert.deepEqual(received, [], JSON.stringify(payload));
    assert.equal(result.profile.cursor, 0);
    assert.equal(result.receiveBlocked, true);
    assert.equal(result.conflicts.length, 1);
    assert.match(result.conflicts[0].code, /^(invalid|unsupported)_/);
  }
});

test('authenticated ciphertext with bad UTF-8 or JSON stays retryable without exposing plaintext in the error', async (t) => {
  const server = await fixture(t);
  const invalidUtf8 = Buffer.concat([
    Buffer.from('{"title":"'),
    Buffer.from([0xff]),
    Buffer.from(
      '","document_schema":3,"document":{"type":"doc","content":[]},"updated_at":"2026-11-05T00:00:00Z"}',
    ),
  ]);
  for (const bytes of [invalidUtf8, encoder.encode('private controlled text that is not JSON')]) {
    server.setOps([await server.op(bytes, 1)]);
    const result = await syncMobile(server.profile, [], {
      onRemote() {
        assert.fail('must not apply');
      },
    });
    assert.equal(result.conflicts[0].code, 'invalid_sync_payload');
    assert.ok(!result.conflicts[0].message.includes('private controlled text'));
    assert.equal(result.profile.cursor, 0);
  }
});

test('decryption failure and local persistence failure do not skip later operations', async (t) => {
  const server = await fixture(t);
  const config = await server.op({ sync_kind: 'tag' }, 2);
  const current = await server.op(note('加密正文'), 5);
  const later = await server.op(note('稍后正文'), 7);
  server.setOps([config, { ...current, ciphertext: 'not*base64' }, later]);
  const invalidEncoding = await syncMobile(server.profile, [], {
    onRemote() {
      assert.fail('must not apply invalid encoding');
    },
  });
  assert.equal(invalidEncoding.profile.cursor, 2);
  assert.equal(invalidEncoding.conflicts[0].code, 'invalid_base64');
  const badCiphertext = Buffer.from(current.ciphertext, 'base64');
  badCiphertext[badCiphertext.length - 1] ^= 1;
  server.setOps([config, { ...current, ciphertext: badCiphertext.toString('base64') }, later]);
  const stored = new Map();
  const decrypted = await syncMobile(server.profile, [], {
    onRemote: (value) => stored.set(value.objectId, value),
  });
  assert.equal(decrypted.profile.cursor, 2);
  assert.equal(decrypted.conflicts[0].code, 'decrypt_failed');
  assert.equal(stored.size, 0);
  server.setOps([config, current, later]);
  const failed = await syncMobile(decrypted.profile, [], {
    onRemote() {
      throw new Error('private storage detail');
    },
  });
  assert.equal(failed.profile.cursor, 2);
  assert.equal(failed.conflicts[0].code, 'remote_apply_failed');
  assert.ok(!failed.conflicts[0].message.includes('private storage detail'));
  const retried = await syncMobile(failed.profile, [], {
    onRemote: (value) => stored.set(value.objectId, value),
  });
  assert.deepEqual([...stored.keys()], [current.object_id, later.object_id]);
  assert.equal(retried.profile.cursor, 7);
  assert.equal(retried.receiveBlocked, false);
});

test('the existing explicit keep-local callback result is not mistaken for a failed write', async (t) => {
  const server = await fixture(t);
  const local = await server.op(note('另一个版本'), 4);
  const next = await server.op(note('下一条'), 9);
  server.setOps([local, next]);
  const decisions = [];
  const result = await syncMobile(server.profile, [], {
    onRemote(value) {
      decisions.push(value.objectId);
      return value.objectId !== local.object_id;
    },
  });
  assert.deepEqual(decisions, [local.object_id, next.object_id]);
  assert.deepEqual(result.remoteChanges, [next.object_id]);
  assert.deepEqual(result.conflicts, []);
  assert.equal(result.profile.cursor, 9);
  assert.equal(result.receiveBlocked, false);
});

test('configuration deletion does not create captures, and failed note deletion retains its cursor', async (t) => {
  const server = await fixture(t);
  const configuration = await server.op({ sync_kind: 'canvas' }, 1);
  const deleteConfig = {
    ...configuration,
    server_seq: 2,
    kind: 'delete',
    ciphertext: null,
    tombstone: true,
    resulting_version: 2,
  };
  const live = await server.op(note('保留至删除成功'), 3);
  const deleteNote = {
    ...live,
    server_seq: 4,
    kind: 'purge',
    ciphertext: null,
    tombstone: true,
    purged: true,
    resulting_version: 2,
  };
  server.setOps([configuration, deleteConfig, live, deleteNote]);
  const stored = new Map();
  let fail = true;
  const hooks = {
    onRemote: (value) => stored.set(value.objectId, value),
    onDelete(value) {
      if (!stored.has(value.objectId)) return;
      if (fail) throw new Error('write failed');
      stored.delete(value.objectId);
    },
  };
  const first = await syncMobile(server.profile, [], hooks);
  assert.equal(stored.size, 1);
  assert.ok(stored.has(live.object_id) && !stored.has(configuration.object_id));
  assert.equal(first.profile.cursor, 3);
  assert.equal(first.conflicts[0].code, 'remote_apply_failed');
  fail = false;
  const next = await syncMobile(first.profile, [], hooks);
  assert.equal(next.profile.cursor, 4);
  assert.equal(stored.size, 0);
});

test('malformed batch cursors and cross-workspace operations are rejected before application', async (t) => {
  let mode = 'cursor';
  const server = await fixture(t, {
    batchOverride(batch) {
      if (mode === 'cursor') return { ...batch, next_cursor: batch.next_cursor + 100 };
      if (mode === 'workspace')
        return {
          ...batch,
          ops: batch.ops.map((op) => ({ ...op, workspace_id: 'another-workspace' })),
        };
      if (mode === 'sequence')
        return { ...batch, ops: batch.ops.map((op) => ({ ...op, server_seq: 0 })) };
      return { ...batch, ops: [], has_more: true, next_cursor: 0 };
    },
  });
  server.setOps([await server.op(note(), 5)]);
  for (mode of ['cursor', 'workspace', 'sequence', 'empty']) {
    await assert.rejects(
      syncMobile(server.profile, [], {
        onRemote() {
          assert.fail('must not apply invalid batch');
        },
      }),
      { code: 'invalid_pull_batch' },
    );
    assert.equal(server.profile.cursor, 0);
  }
});

test('a missing note persistence hook cannot falsely acknowledge a received note', async (t) => {
  const server = await fixture(t);
  server.setOps([await server.op(note(), 1)]);
  const result = await syncMobile(server.profile, []);
  assert.equal(result.profile.cursor, 0);
  assert.equal(result.receiveBlocked, true);
  assert.equal(result.conflicts[0].code, 'remote_handler_missing');
});

test('the receive budget reports more work and resumes after the persisted prefix without inventing a conflict', async (t) => {
  const server = await fixture(t);
  const ops = await Promise.all(
    Array.from({ length: 4001 }, (_, index) => server.op(note(`条目 ${index + 1}`), index + 1)),
  );
  server.setOps(ops);
  const stored = new Map();
  const hooks = { onRemote: (value) => stored.set(value.objectId, value) };
  const first = await syncMobile(server.profile, [], hooks);
  assert.equal(first.profile.cursor, 4000);
  assert.equal(first.remoteChanges.length, 4000);
  assert.equal(first.hasMore, true);
  assert.equal(first.receiveBlocked, false);
  assert.deepEqual(first.conflicts, []);
  assert.equal(server.pullRequests.length, 20);
  assert.equal(stored.has(ops[4000].object_id), false);

  const next = await syncMobile(first.profile, [], hooks);
  assert.equal(server.pullRequests[20], 4000);
  assert.equal(next.profile.cursor, 4001);
  assert.equal(next.hasMore, false);
  assert.deepEqual(next.remoteChanges, [ops[4000].object_id]);
  assert.equal(stored.size, 4001);
});

test('a deletion without a handler only requires persistence for a capture in the synchronized workspace', async (t) => {
  const server = await fixture(t);
  const record = await server.op(note(), 1);
  const deletion = {
    ...record,
    kind: 'delete',
    ciphertext: null,
    tombstone: true,
    resulting_version: 2,
  };
  server.setOps([deletion]);
  const foreign = { objectId: record.object_id, workspaceId: crypto.randomUUID(), baseVersion: 1 };
  const result = await syncMobile(server.profile, [foreign]);
  assert.equal(result.profile.cursor, 1);
  assert.equal(result.receiveBlocked, false);
  assert.deepEqual(result.conflicts, []);
  const local = { ...foreign, workspaceId: server.profile.workspaceId };
  const blocked = await syncMobile(server.profile, [local]);
  assert.equal(blocked.profile.cursor, 0);
  assert.equal(blocked.conflicts[0].code, 'remote_handler_missing');
});
