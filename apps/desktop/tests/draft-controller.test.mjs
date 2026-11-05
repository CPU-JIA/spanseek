import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { setImmediate } from 'node:timers/promises';
import test from 'node:test';
import { DraftController } from '../src/editor/draft-controller.ts';
import { draftKey } from '../src/editor/drafts.ts';
import { DraftRegistry } from '../src/editor/draft-registry.ts';

// Only the persistence and transport boundaries are replaced. Every transition
// below executes the production DraftController, without a browser or Tauri.
class MemoryDraftStorage {
  records = new Map();
  writes = [];
  failWrites = 0;

  async read(key) {
    return structuredClone(this.records.get(key));
  }
  async write(record) {
    if (this.failWrites > 0) {
      this.failWrites -= 1;
      throw new Error('本地草稿未能落盘');
    }
    const copy = structuredClone(record);
    this.writes.push(copy);
    this.records.set(copy.key, copy);
  }
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function transport() {
  const attempts = [];
  let active = 0;
  let maximumActive = 0;
  const save = (commandId, request) => {
    const response = deferred();
    active += 1;
    maximumActive = Math.max(maximumActive, active);
    attempts.push({ commandId, request: structuredClone(request), ...response });
    return response.promise.finally(() => {
      active -= 1;
    });
  };
  return {
    attempts,
    save,
    get maximumActive() {
      return maximumActive;
    },
  };
}

function document(text, blockId = randomUUID()) {
  return {
    type: 'doc',
    content: [{ type: 'paragraph', attrs: { id: blockId }, content: [{ type: 'text', text }] }],
  };
}

function note(text = '原始正文', id = randomUUID()) {
  return {
    id,
    title: '原始标题',
    revision: 1,
    revision_id: randomUUID(),
    parent_revision_id: null,
    content_hash: 'a'.repeat(64),
    author_kind: 'human',
    schema_version: 2,
    document: document(text),
    trashed: false,
    updated_at: '2026-10-02T00:00:00Z',
  };
}

function committed(attempt, revision = 2) {
  const result = {
    ...note('', attempt.request.note_id),
    title: attempt.request.title,
    document: structuredClone(attempt.request.document),
    schema_version: attempt.request.document_schema,
    revision,
    parent_revision_id: attempt.request.base_revision_id,
  };
  return {
    note: result,
    receipt: {
      command_id: attempt.commandId,
      object_id: result.id,
      revision_id: result.revision_id,
      change_seq: String(revision),
      committed_at: result.updated_at,
      replayed: false,
    },
  };
}

function identity() {
  return { local_instance_id: randomUUID(), workspace_id: randomUUID() };
}

async function until(predicate, message = 'expected transition did not occur') {
  for (let turn = 0; turn < 40; turn += 1) {
    if (predicate()) return;
    await setImmediate();
  }
  assert.fail(message);
}

async function fixture(
  t,
  { base = note(), scope = identity(), storage = new MemoryDraftStorage() } = {},
) {
  // No wall-clock sleeps and no autosave callbacks escaping a failed test.
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const remote = transport();
  const controller = await DraftController.open(scope, base, storage, remote.save);
  return { controller, remote, storage, base, scope };
}

test('new edits survive an old receipt and all callers share one serial save queue', async (t) => {
  const { controller, remote, storage, base, scope } = await fixture(t);
  const snapshots = [];
  controller.subscribe(() => snapshots.push(controller.getSnapshot()));
  controller.update('第一版', document('第一版正文'));
  const firstFlush = controller.flush();
  await until(() => remote.attempts.length === 1);
  const first = remote.attempts[0];
  controller.update('过渡修改', document('不会单独发送的过渡修改'));
  const latest = document('保存期间继续输入的最终正文');
  controller.update('最终标题', latest);
  const anotherFlush = controller.flush();
  assert.equal(remote.attempts.length, 1);
  assert.equal(controller.getSnapshot().status, 'saving');
  const firstCommit = committed(first);
  first.resolve(firstCommit);
  await until(() => remote.attempts.length === 2);

  const pending = controller.getSnapshot();
  assert.equal(pending.record.title, '最终标题');
  assert.deepEqual(pending.record.document, latest);
  assert.equal(pending.record.acknowledged, 1);
  assert.equal(pending.record.generation, 3);
  assert.notEqual(pending.status, 'saved');
  assert.equal(remote.attempts[1].request.base_revision_id, firstCommit.note.revision_id);
  assert.deepEqual(remote.attempts[1].request.document, latest);
  assert.notEqual(remote.attempts[1].commandId, first.commandId);
  remote.attempts[1].resolve(committed(remote.attempts[1], 3));
  await Promise.all([firstFlush, anotherFlush]);

  assert.equal(remote.maximumActive, 1);
  assert.equal(remote.attempts.length, 2);
  assert.equal(controller.getSnapshot().status, 'saved');
  assert.ok(
    snapshots.every(
      ({ status, record }) =>
        status !== 'saved' || (!record.pending && record.generation === record.acknowledged),
    ),
  );
  const durable = await storage.read(draftKey(scope, base.id));
  assert.equal(durable.generation, durable.acknowledged);
  assert.equal(durable.pending, null);
  assert.deepEqual(durable.document, latest);
  await controller.dispose();
});

test('a clean open editor adopts a remote revision while preserving its block bindings', async (t) => {
  const { controller, base } = await fixture(t);
  const remote = {
    ...base,
    title: '远端标题',
    revision: 2,
    revision_id: randomUUID(),
    parent_revision_id: base.revision_id,
    document: document('远端正文'),
    updated_at: '2026-10-03T00:00:00Z',
  };
  assert.equal(await controller.applyRemote(remote), 'adopted');
  assert.equal(controller.getSnapshot().status, 'saved');
  assert.equal(controller.getSnapshot().record.base.revision_id, remote.revision_id);
  assert.equal(controller.getSnapshot().record.title, '远端标题');
  assert.equal(controller.getSnapshot().record.document.content[0].content[0].text, '远端正文');
  await controller.dispose();
});

test('a remote revision never overwrites a local draft and becomes an explicit conflict', async (t) => {
  const { controller, base } = await fixture(t);
  controller.update('本地标题', document('本地正文'));
  await until(() => controller.getSnapshot().status === 'local');
  const remote = {
    ...base,
    title: '远端标题',
    revision: 2,
    revision_id: randomUUID(),
    parent_revision_id: base.revision_id,
    document: document('远端正文'),
  };
  assert.equal(await controller.applyRemote(remote), 'conflict');
  assert.equal(controller.getSnapshot().status, 'conflict');
  assert.equal(controller.getSnapshot().record.title, '本地标题');
  assert.equal(controller.getSnapshot().record.document.content[0].content[0].text, '本地正文');
  assert.match(controller.getSnapshot().error, /其他设备/);
});

test('a remote trash stays explicit instead of deleting an open local draft', async (t) => {
  const { controller, base } = await fixture(t);
  const remote = {
    ...base,
    revision: 2,
    revision_id: randomUUID(),
    parent_revision_id: base.revision_id,
    trashed: true,
  };
  assert.equal(await controller.applyRemote(remote), 'conflict');
  assert.equal(controller.getSnapshot().status, 'conflict');
  assert.equal(controller.getSnapshot().record.base.trashed, false);
  assert.match(controller.getSnapshot().error, /回收站/);
});

test('edits arriving during a clean-state write keep flush open until those edits commit', async (t) => {
  const { controller, remote, storage } = await fixture(t);
  const gate = deferred();
  const write = storage.write.bind(storage);
  let entered = false;
  storage.write = async (record) => {
    if (!entered) {
      entered = true;
      await gate.promise;
    }
    await write(record);
  };
  const snapshots = [];
  controller.subscribe(() => snapshots.push(controller.getSnapshot()));
  let settled = false;
  const flush = controller.flush().finally(() => {
    settled = true;
  });
  await until(() => entered);
  controller.update('确认期间的新修改', document('不能被旧的干净状态确认吞掉'));
  gate.resolve();
  await until(() => settled || remote.attempts.length === 1);
  assert.equal(
    remote.attempts.length,
    1,
    'flush returned before the newly arrived edit was submitted',
  );
  assert.equal(settled, false);
  assert.ok(
    snapshots.every(
      ({ status, record }) => status !== 'saved' || record.generation === record.acknowledged,
    ),
  );
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await flush;
  assert.equal(controller.getSnapshot().status, 'saved');
  await controller.dispose();
});

test('IME in either pane prevents autosave and explicit submission until both finish', async (t) => {
  const { controller, remote } = await fixture(t);
  controller.setComposing('left', true);
  controller.setComposing('right', true);
  controller.update('中文标题', document('正在输入拼音'));
  t.mock.timers.tick(2000);
  await assert.rejects(controller.flush(), /输入法/);
  assert.equal(remote.attempts.length, 0);
  controller.setComposing('left', false);
  t.mock.timers.tick(2000);
  assert.equal(remote.attempts.length, 0);
  controller.update('中文标题', document('最终的中文输入'));
  controller.setComposing('right', false);
  t.mock.timers.tick(649);
  assert.equal(remote.attempts.length, 0);
  t.mock.timers.tick(1);
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].request.document.content[0].content[0].text, '最终的中文输入');
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await until(() => controller.getSnapshot().status === 'saved');
  await controller.dispose();
});

test('IME begun during a save lets that receipt settle without submitting composing edits', async (t) => {
  const { controller, remote } = await fixture(t);
  controller.update('已经完成的输入', document('第一段'));
  const flush = controller.flush();
  await until(() => remote.attempts.length === 1);
  controller.setComposing('left', true);
  controller.update('正在输入', document('拼音中间态'));
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await flush;
  assert.equal(remote.attempts.length, 1);
  assert.notEqual(controller.getSnapshot().status, 'saved');
  assert.equal(controller.getSnapshot().record.acknowledged, 1);
  controller.update('输入完成', document('最终汉字'));
  controller.setComposing('left', false);
  const finalFlush = controller.flush();
  await until(() => remote.attempts.length === 2);
  assert.equal(remote.attempts[1].request.title, '输入完成');
  remote.attempts[1].resolve(committed(remote.attempts[1], 3));
  await finalFlush;
  await controller.dispose();
});

test('unknown save results retry the identical command and payload before newer edits', async (t) => {
  const { controller, remote } = await fixture(t);
  controller.update('可能已经提交', document('第一份正文'));
  const firstFlush = controller.flush();
  const rejected = assert.rejects(firstFlush, /回执丢失/);
  await until(() => remote.attempts.length === 1);
  const first = remote.attempts[0];
  const result = committed(first);
  first.reject(new Error('连接中断，回执丢失'));
  await rejected;
  assert.equal(controller.getSnapshot().status, 'error');
  controller.update('离线继续编辑', document('第二份正文'));
  const retry = controller.retry();
  await until(() => remote.attempts.length === 2);
  assert.equal(remote.attempts[1].commandId, first.commandId);
  assert.deepEqual(remote.attempts[1].request, first.request);
  remote.attempts[1].resolve({ ...result, receipt: { ...result.receipt, replayed: true } });
  await until(() => remote.attempts.length === 3);
  assert.notEqual(remote.attempts[2].commandId, first.commandId);
  assert.equal(remote.attempts[2].request.base_revision_id, result.note.revision_id);
  assert.equal(remote.attempts[2].request.title, '离线继续编辑');
  remote.attempts[2].resolve(committed(remote.attempts[2], 3));
  await retry;
  assert.equal(remote.maximumActive, 1);
  await controller.dispose();
});

test('restart resumes a persisted pending intent even when the server already committed it', async (t) => {
  const { controller, remote, storage, scope } = await fixture(t);
  controller.update('重启前提交', document('已捕获的正文'));
  const flush = controller.flush();
  const rejected = assert.rejects(flush, /未知结果/);
  await until(() => remote.attempts.length === 1);
  const original = remote.attempts[0];
  const result = committed(original);
  original.reject(new Error('未知结果'));
  await rejected;
  const nextRemote = transport();
  const restarted = await DraftController.open(scope, result.note, storage, nextRemote.save);
  assert.equal(restarted.getSnapshot().status, 'local');
  const resumed = restarted.flush();
  await until(() => nextRemote.attempts.length === 1);
  assert.equal(nextRemote.attempts[0].commandId, original.commandId);
  assert.deepEqual(nextRemote.attempts[0].request, original.request);
  nextRemote.attempts[0].resolve({ ...result, receipt: { ...result.receipt, replayed: true } });
  await resumed;
  assert.equal(restarted.getSnapshot().status, 'saved');
  assert.equal(restarted.getSnapshot().record.base.revision_id, result.note.revision_id);
  assert.equal(nextRemote.attempts.length, 1);
  await restarted.dispose();
});

test('a persisted schema 2 intent replays unchanged before new rich edits use schema 3', async (t) => {
  const { controller, remote, storage, scope, base } = await fixture(t);
  const legacy = structuredClone(controller.getSnapshot().record);
  legacy.title = '升级前待确认';
  legacy.document = document('升级前的正文');
  legacy.generation = 1;
  legacy.pending = {
    commandId: randomUUID(),
    generation: 1,
    request: {
      note_id: base.id,
      base_revision_id: base.revision_id,
      title: legacy.title,
      document: legacy.document,
      document_schema: 2,
    },
  };
  await storage.write(legacy);
  const restarted = await DraftController.open(scope, base, storage, remote.save);
  const retry = restarted.flush();
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].commandId, legacy.pending.commandId);
  assert.deepEqual(remote.attempts[0].request, legacy.pending.request);
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await retry;
  const rich = document('下划线和高亮');
  rich.content[0].content[0].marks = [
    { type: 'underline' },
    { type: 'highlight', attrs: { color: '#FFE08A' } },
  ];
  restarted.update('升级后的编辑', rich);
  const saved = restarted.flush();
  await until(() => remote.attempts.length === 2);
  assert.equal(remote.attempts[1].request.document_schema, 3);
  assert.deepEqual(remote.attempts[1].request.document, rich);
  remote.attempts[1].resolve(committed(remote.attempts[1], 3));
  await saved;
  assert.equal(restarted.getSnapshot().record.base.schema_version, 3);
  await restarted.dispose();
});

test('drafts are isolated by both identities, including copies of the same logical workspace', async (t) => {
  const { controller, remote, storage, base, scope } = await fixture(t);
  controller.update('A 的未提交草稿', document('不得泄漏给恢复副本'));
  await until(() => storage.records.has(draftKey(scope, base.id)));
  const copy = { ...scope, local_instance_id: randomUUID() };
  const otherWorkspace = { ...scope, workspace_id: randomUUID() };
  for (const isolated of [copy, otherWorkspace]) {
    const other = await DraftController.open(isolated, base, storage, remote.save);
    assert.equal(other.getSnapshot().status, 'saved');
    assert.equal(other.getSnapshot().record.title, base.title);
    assert.notEqual(other.getSnapshot().record.key, controller.getSnapshot().record.key);
    await other.dispose();
  }
  assert.equal(remote.attempts.length, 0);
  const corruptStorage = {
    read: async () => structuredClone(controller.getSnapshot().record),
    write: async () => assert.fail('invalid identity must not be written'),
  };
  await assert.rejects(DraftController.open(copy, base, corruptStorage, remote.save), /身份校验/);
});

test('stale recovered drafts require conflict handling and never submit against a newer head', async (t) => {
  const { controller, remote, storage, base, scope } = await fixture(t);
  controller.update('本地修改', document('需要保留的本地正文'));
  await until(() => storage.records.has(draftKey(scope, base.id)));
  const newer = { ...base, revision: 2, revision_id: randomUUID(), title: '外部修改' };
  const restarted = await DraftController.open(scope, newer, storage, remote.save);
  assert.equal(restarted.getSnapshot().status, 'conflict');
  assert.equal(restarted.getSnapshot().record.title, '本地修改');
  await assert.rejects(restarted.flush(), /更新|冲突/);
  await assert.rejects(restarted.retry(), /冲突/);
  assert.equal(remote.attempts.length, 0);
});

test('a backend revision conflict retains pending data and disables blind retries', async (t) => {
  const { controller, remote, storage, base, scope } = await fixture(t);
  controller.update('冲突中的正文', document('不能覆盖远端'));
  const flush = controller.flush();
  const rejected = assert.rejects(flush, /版本冲突/);
  await until(() => remote.attempts.length === 1);
  remote.attempts[0].reject(
    Object.assign(new Error('版本冲突'), { details: { code: 'revision_conflict' } }),
  );
  await rejected;
  assert.equal(controller.getSnapshot().status, 'conflict');
  assert.ok((await storage.read(draftKey(scope, base.id))).pending);
  controller.update('冲突后仍可保留编辑', document('继续保留而不自动提交'));
  assert.equal(controller.getSnapshot().status, 'conflict');
  await assert.rejects(controller.retry(), /冲突/);
  assert.equal(remote.attempts.length, 1);
});

test('failed local writes prevent submission and can be retried after storage recovers', async (t) => {
  const { controller, remote, storage } = await fixture(t);
  storage.failWrites = 2;
  controller.update('磁盘故障时的正文', document('必须保留在内存'));
  await until(() => controller.getSnapshot().status === 'error');
  await assert.rejects(controller.flush(), /未能落盘/);
  assert.equal(remote.attempts.length, 0);
  assert.equal(controller.getSnapshot().record.title, '磁盘故障时的正文');
  const pending = structuredClone(controller.getSnapshot().record.pending);
  const retry = controller.retry();
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].commandId, pending.commandId);
  assert.deepEqual(remote.attempts[0].request, pending.request);
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await retry;
  await controller.dispose();
});

test('a conflict copy of a maximum-length title still fits the core title limit', async (t) => {
  // The core rejects titles over 300 characters, so the copy title must be
  // truncated instead of failing the whole "keep the draft as a new note".
  const longTitle = '标'.repeat(300);
  const { controller, remote, base, scope } = await fixture(t, {
    base: { ...note(), title: longTitle },
  });
  controller.update(longTitle, document('冲突中的正文'));
  const flush = controller.flush();
  const rejected = assert.rejects(flush, /版本冲突/);
  await until(() => remote.attempts.length === 1);
  remote.attempts[0].reject(
    Object.assign(new Error('版本冲突'), { details: { code: 'revision_conflict' } }),
  );
  await rejected;
  assert.equal(controller.getSnapshot().status, 'conflict');
  const sent = [];
  await controller.runHistory(
    async (command) => {
      sent.push(command);
      const created = {
        ...note('', randomUUID()),
        title: command.payload.note.title,
        document: structuredClone(command.payload.note.document),
        schema_version: command.payload.note.document_schema,
        revision: 1,
        parent_revision_id: null,
      };
      return {
        note: created,
        receipt: {
          command_id: command.payload.command_id,
          object_id: created.id,
          revision_id: created.revision_id,
          change_seq: '1',
          committed_at: created.updated_at,
          replayed: false,
        },
      };
    },
    async () => ({ ...base, revision: 2, revision_id: randomUUID() }),
    { conflictSuffix: '（冲突草稿）' },
  );
  assert.equal(sent.length, 1);
  const title = sent[0].payload.note.title;
  assert.ok(Array.from(title).length <= 300, `title kept ${Array.from(title).length} characters`);
  assert.match(title, /冲突草稿/);
  assert.equal(controller.getSnapshot().status, 'saved');
  assert.equal(controller.getSnapshot().record.title, base.title);
  await controller.dispose();
});
test('a failed acknowledgement write is retried locally without submitting another revision', async (t) => {
  const { controller, remote, storage, base, scope } = await fixture(t);
  controller.update('已提交但草稿确认写失败', document('后台已经保存'));
  const flush = controller.flush();
  const rejected = assert.rejects(flush, /未能落盘/);
  await until(() => remote.attempts.length === 1);
  storage.failWrites = 1;
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await rejected;
  assert.equal(controller.getSnapshot().status, 'error');
  assert.equal(controller.getSnapshot().record.pending, null);
  await controller.retry();
  assert.equal(
    remote.attempts.length,
    1,
    'durable backend receipt must not create a second revision',
  );
  assert.equal(controller.getSnapshot().status, 'saved');
  const stored = await storage.read(draftKey(scope, base.id));
  assert.equal(stored.pending, null);
  assert.equal(stored.generation, stored.acknowledged);
  await controller.dispose();
});

test('adopting a revision rejects edits while its local write is pending', async (t) => {
  const { controller, storage, base, scope } = await fixture(t);
  const gate = deferred();
  const write = storage.write.bind(storage);
  let entered = false;
  storage.write = async (record) => {
    if (!entered) {
      entered = true;
      await gate.promise;
    }
    await write(record);
  };
  const newer = { ...base, revision: 2, revision_id: randomUUID(), title: '准备采用的版本' };
  // Capture either success or a deliberate stale-adoption rejection immediately.
  const adopting = controller.adopt(newer).then(
    () => null,
    (error) => error,
  );
  await until(() => entered);
  assert.equal(controller.getSnapshot().historyBusy, true);
  assert.throws(
    () => controller.update('采用期间继续编辑', document('写入期间不接受新编辑')),
    /版本操作/,
  );
  gate.resolve();
  assert.equal(await adopting, null);
  assert.equal(controller.getSnapshot().historyBusy, false);
  assert.equal(controller.getSnapshot().record.title, newer.title);
  assert.equal(controller.getSnapshot().status, 'saved');
  assert.equal((await storage.read(draftKey(scope, base.id))).title, newer.title);
});

test('a network result cannot adopt over edits made since the operation started', async (t) => {
  const { controller, base } = await fixture(t);
  const expected = controller.getSnapshot().record;
  controller.update('网络等待期间的新标题', document('最新内容'));
  await assert.rejects(
    controller.adopt({ ...base, revision_id: randomUUID() }, expected),
    /又有修改/,
  );
  assert.equal(controller.getSnapshot().record.title, '网络等待期间的新标题');
});

test('failed controller loads are retryable and cannot poison unrelated flushes', async (t) => {
  const { controller, base, scope } = await fixture(t);
  const registry = new DraftRegistry();
  await assert.rejects(
    registry.get(scope, base.id, async () => {
      throw new Error('读取失败');
    }),
    /读取失败/,
  );
  await registry.flush();
  assert.equal(await registry.get(scope, base.id, async () => controller), controller);
  const finish = await registry.freezeAndFlush();
  finish(true);
  assert.throws(() => controller.update('too late', document('too late')), /已关闭/);
});

test('workspace drain freezes every note until the slowest save has finished', async (t) => {
  const { controller, base, scope, remote, storage } = await fixture(t);
  const another = await DraftController.open(scope, note(), storage, remote.save);
  const registry = new DraftRegistry();
  await registry.get(scope, base.id, async () => controller);
  await registry.get(scope, another.getSnapshot().record.base.id, async () => another);
  another.update('慢保存', document('不能绕过最后一道屏障'));
  const draining = registry.freezeAndFlush();
  await until(() => remote.attempts.length === 1);
  assert.throws(() => controller.update('较早保存后继续输入', document('禁止')), /切换/);
  await assert.rejects(
    registry.get(scope, randomUUID(), async () => controller),
    /切换/,
  );
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  const finish = await draining;
  finish(false);
  controller.update('取消切库后继续输入', document('允许'));
  assert.equal(controller.getSnapshot().record.title, '取消切库后继续输入');
});

test('a failed drain restores editing and retains the failed note for retry', async (t) => {
  const { controller, base, scope, remote } = await fixture(t);
  const registry = new DraftRegistry();
  await registry.get(scope, base.id, async () => controller);
  controller.update('失败前', document('保存失败'));
  const draining = registry.freezeAndFlush();
  const rejection = assert.rejects(draining, /故障/);
  await until(() => remote.attempts.length === 1);
  remote.attempts[0].reject(new Error('故障'));
  await rejection;
  assert.equal(
    await registry.get(scope, base.id, async () => assert.fail('must retain')),
    controller,
  );
  controller.update('失败后继续编辑', document('保留'));
  assert.equal(controller.getSnapshot().record.title, '失败后继续编辑');
});

test('workspace drain refuses IME and unfinished image/history actions', async (t) => {
  const { controller } = await fixture(t);
  controller.setComposing('title', true);
  assert.throws(() => controller.freeze(), /输入法/);
  controller.setComposing('title', false);
  const finishAction = controller.beginAction();
  assert.throws(() => controller.freeze(), /图片或版本/);
  finishAction();
  finishAction();
  const thaw = controller.freeze();
  assert.throws(() => controller.beginAction(), /切换/);
  thaw();
  await controller.dispose();
});

test('unmount composition cleanup cannot restart autosave during a frozen lifecycle action', async (t) => {
  const { controller, base, scope, storage, remote } = await fixture(t);
  const registry = new DraftRegistry();
  await registry.get(scope, base.id, async () => controller);
  const finish = await registry.freezeAndFlush();
  const writes = storage.writes.length;
  // The trash pending gate unmounts both panes before its IPC response arrives.
  controller.setComposing('left-title', false);
  controller.setComposing('left-body', false);
  controller.setComposing('right-title', false);
  controller.setComposing('right-body', false);
  t.mock.timers.tick(1000);
  await setImmediate();
  assert.equal(storage.writes.length, writes, 'frozen cleanup must not schedule a new local write');
  assert.equal(remote.attempts.length, 0);
  finish(true);
  assert.throws(() => controller.update('too late', document('too late')), /已关闭/);
});

test('a failed release unlocks the registry and retains its controller for recovery', async (t) => {
  const { controller, base, scope, storage } = await fixture(t);
  const registry = new DraftRegistry();
  await registry.get(scope, base.id, async () => controller);
  const finish = await registry.freezeAndFlush();
  const gate = deferred();
  const write = storage.write.bind(storage);
  let entered = false;
  storage.write = async (record) => {
    entered = true;
    await gate.promise;
    await write(record);
  };
  // An unexpected explicit flush must fail release safely, without losing ownership.
  const lateFlush = controller.flush();
  await until(() => entered);
  assert.throws(() => finish(true), /未保存/);
  assert.equal(
    await registry.get(scope, base.id, async () => assert.fail('must retain the controller')),
    controller,
  );
  gate.resolve();
  await lateFlush;
  controller.update('恢复后继续编辑', document('原会话已解锁'));
  assert.equal(controller.getSnapshot().record.title, '恢复后继续编辑');
});

test('blank titles save as the visible untitled placeholder without invalidating the draft', async (t) => {
  const { controller, remote } = await fixture(t);
  controller.update('  ', document('空标题正文仍要保存'));
  const saving = controller.flush();
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].request.title, '未命名笔记');
  remote.attempts[0].resolve(committed(remote.attempts[0]));
  await saving;
  assert.equal(controller.getSnapshot().status, 'saved');
  await controller.dispose();
});

test('a definitive validation rejection allows a corrected draft to submit a new intent', async (t) => {
  const { controller, remote } = await fixture(t);
  controller.update('待修正', document('受控无效输入'));
  const saving = controller.flush();
  const rejected = assert.rejects(saving, /输入无效/);
  await until(() => remote.attempts.length === 1);
  remote.attempts[0].reject(
    Object.assign(new Error('输入无效'), { details: { code: 'invalid_document' } }),
  );
  await rejected;
  assert.equal(controller.getSnapshot().record.pending, null);
  controller.update('已修正', document('有效正文'));
  const retry = controller.retry();
  await until(() => remote.attempts.length === 2);
  assert.notEqual(remote.attempts[0].commandId, remote.attempts[1].commandId);
  assert.equal(remote.attempts[1].request.title, '已修正');
  remote.attempts[1].resolve(committed(remote.attempts[1]));
  await retry;
  await controller.dispose();
});

test('adoption refuses active composition and blocks new composition during its durable write', async (t) => {
  const { controller, base, storage } = await fixture(t);
  controller.setComposing('title', true);
  await assert.rejects(controller.adopt(base), /输入法/);
  controller.setComposing('title', false);
  const gate = deferred();
  const write = storage.write.bind(storage);
  let entered = false;
  storage.write = async (record) => {
    if (!entered) {
      entered = true;
      await gate.promise;
    }
    await write(record);
  };
  const adopting = controller.adopt({ ...base, title: '待采用' });
  await until(() => entered);
  assert.throws(() => controller.setComposing('title', true), /版本操作/);
  gate.resolve();
  await adopting;
  assert.equal(controller.getSnapshot().record.title, '待采用');
  controller.setComposing('title', false);
  await controller.dispose();
});

test('a burst of edits merges into one waiting local write instead of one per keystroke', async (t) => {
  const { controller, remote, storage, scope, base } = await fixture(t);
  controller.update('标题 1', document('第 1 段'));
  await setImmediate();
  assert.equal(storage.writes.length, 1, 'the first edit of the window writes at once');
  for (let index = 2; index <= 40; index += 1)
    controller.update(`标题 ${index}`, document(`第 ${index} 段`));
  await setImmediate();
  assert.equal(storage.writes.length, 1, 'edits inside the window merge into one waiting record');
  assert.equal(controller.getSnapshot().record.title, '标题 40');
  assert.equal(controller.getSnapshot().status, 'dirty');

  // The debounce window ends: one more local write carries the newest draft, and
  // it is durable before the command is sent.
  t.mock.timers.tick(650);
  await until(() => remote.attempts.length === 1);
  assert.equal(storage.writes.length, 2, '40 keystrokes produced one extra local write');
  assert.equal(storage.writes[1].generation, 40);
  assert.deepEqual(storage.writes[1].pending.request, remote.attempts[0].request);
  assert.equal(
    storage.writes[1].pending.request.document.content[0].content[0].text,
    '第 40 段',
    'the merged write carries the newest edit, not an intermediate one',
  );

  remote.attempts[0].resolve(committed(remote.attempts[0], 2));
  await until(() => controller.getSnapshot().status === 'saved');
  // The same burst wrote 42 whole-document records before this fix (40 during
  // typing plus the submit and acknowledgement writes); see
  // .local-archive/v18-block3-before/burst-comparison.mjs for the measurement.
  assert.ok(
    storage.writes.length <= 4,
    `40 keystrokes must not write once per keystroke (wrote ${storage.writes.length})`,
  );
  const durable = await storage.read(draftKey(scope, base.id));
  assert.equal(durable.generation, durable.acknowledged);
  assert.equal(durable.pending, null);
  assert.equal(durable.document.content[0].content[0].text, '第 40 段');
  await controller.dispose();
});

test('a flush during a merged burst still lands the exact payload before the IPC', async (t) => {
  const { controller, remote, storage } = await fixture(t);
  const gate = deferred();
  const write = storage.write.bind(storage);
  let entered = false;
  storage.write = async (record) => {
    if (!entered) {
      entered = true;
      await gate.promise;
    }
    await write(record);
  };
  controller.update('第一版', document('第一版正文'));
  controller.update('第二版', document('第二版正文'));
  controller.update('第三版', document('第三版正文'));
  const flush = controller.flush();
  await until(() => entered);
  assert.equal(storage.writes.length, 0, 'the burst head write is still in flight');
  assert.equal(remote.attempts.length, 0);
  gate.resolve();
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].request.title, '第三版');
  assert.equal(storage.writes.length, 2, 'the merged record is written once before the send');
  assert.equal(storage.writes[1].generation, 3);
  assert.deepEqual(
    storage.writes[1].pending.request,
    remote.attempts[0].request,
    'the payload sent over IPC is already durable',
  );
  assert.equal(storage.writes[1].document.content[0].content[0].text, '第三版正文');
  remote.attempts[0].resolve(committed(remote.attempts[0], 2));
  await flush;
  assert.equal(controller.getSnapshot().status, 'saved');
  await controller.dispose();
});

test('a failed merged local write stays visible and retryable', async (t) => {
  const { controller, remote, storage } = await fixture(t);
  controller.update('标题 1', document('第 1 段'));
  await setImmediate();
  assert.equal(storage.writes.length, 1);
  for (let index = 2; index <= 8; index += 1)
    controller.update(`标题 ${index}`, document(`第 ${index} 段`));
  storage.failWrites = 1;
  t.mock.timers.tick(650);
  await until(() => controller.getSnapshot().status === 'error');
  assert.equal(remote.attempts.length, 0, 'a failed local write must block submission');
  assert.equal(controller.getSnapshot().record.title, '标题 8');
  const pending = structuredClone(controller.getSnapshot().record.pending);
  assert.ok(pending, 'the exact retry payload is retained');
  const retry = controller.retry();
  await until(() => remote.attempts.length === 1);
  assert.equal(remote.attempts[0].commandId, pending.commandId);
  assert.deepEqual(remote.attempts[0].request, pending.request);
  assert.equal(remote.attempts[0].request.document.content[0].content[0].text, '第 8 段');
  // Only the burst head and the retry reached storage; the merged write failed and
  // was never counted as durable.
  assert.equal(storage.writes.length, 2, 'the failed merged write is not a durable draft');
  remote.attempts[0].resolve(committed(remote.attempts[0], 2));
  await retry;
  assert.equal(controller.getSnapshot().status, 'saved');
  await controller.dispose();
});

test('the merged local write reports local durability for the newest generation', async (t) => {
  const { controller, remote, storage, scope, base } = await fixture(t);
  controller.update('标题 1', document('第 1 段'));
  controller.update('标题 2', document('第 2 段'));
  await setImmediate();
  assert.equal(storage.writes.length, 1);
  assert.equal(controller.getSnapshot().status, 'dirty', 'the merged edit is not written yet');
  // An image import persists the draft without submitting a revision.
  const importing = controller.addImageImport('blob-hash', 'command-id');
  await until(() => controller.getSnapshot().status === 'local');
  await importing;
  assert.equal(controller.getSnapshot().record.generation, 2);
  assert.equal(storage.writes.length, 2, 'the merged edit is the only extra write');
  assert.equal(storage.writes[1].document.content[0].content[0].text, '第 2 段');
  assert.equal(remote.attempts.length, 0);
  assert.equal(
    (await storage.read(draftKey(scope, base.id))).imageImports['blob-hash'],
    'command-id',
  );
});
