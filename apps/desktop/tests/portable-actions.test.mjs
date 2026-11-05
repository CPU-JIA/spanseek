import test from 'node:test';
import assert from 'node:assert/strict';
import { PendingActions, actionErrorText } from '../src/portability/pending-actions.ts';

const command = () => {
  const id = crypto.randomUUID();
  return {
    id,
    identity: { workspace_id: 'workspace-a', local_instance_id: 'local-a' },
    command: { kind: 'backup_create', payload: { command_id: id, token: 'original-grant' } },
    displayName: 'archive.zip',
    createdAt: new Date().toISOString(),
  };
};
function fixture() {
  const f = {
    rows: new Map(),
    calls: [],
    receipts: new Map(),
    writesFail: false,
    cleanupFail: false,
    lost: false,
    error: null,
  };
  f.storage = {
    async read() {
      return structuredClone([...f.rows.values()]);
    },
    async put(action) {
      if (f.writesFail) throw Error('storage full');
      f.rows.set(action.id, structuredClone(action));
    },
    async remove(id) {
      if (f.cleanupFail) throw Error('cleanup failed');
      f.rows.delete(id);
    },
  };
  f.send = async (action) => {
    assert.deepEqual(action, f.rows.get(action.id), 'persisted payload must equal sent payload');
    f.calls.push(structuredClone(action));
    if (f.error) throw f.error;
    if (!f.receipts.has(action.id))
      f.receipts.set(action.id, { kind: 'job', id: crypto.randomUUID() });
    if (f.lost) throw Error('receipt lost after commit');
    return f.receipts.get(action.id);
  };
  f.open = () => new PendingActions(f.storage, f.send);
  return f;
}
test('failed local persistence sends no request', async () => {
  const f = fixture();
  f.writesFail = true;
  const controller = f.open();
  await assert.rejects(controller.submit(command()), /storage full/);
  assert.equal(f.calls.length, 0);
  assert.equal(controller.getSnapshot().pending.length, 0);
  assert.deepEqual(controller.getSnapshot().busy, []);
});

test('diagnostics preview conflict clears only the rejected export and allows a fresh preview', async () => {
  const f = fixture();
  const action = command();
  action.command = {
    kind: 'diagnostics_export',
    payload: {
      command_id: action.id,
      token: 'original-grant',
      categories: ['versions'],
      fingerprint: 'frozen-hash',
    },
  };
  f.error = { details: { code: 'app_state_conflict' } };
  const controller = f.open();
  await assert.rejects(controller.submit(action));
  assert.equal(controller.getSnapshot().pending.length, 0);
  assert.equal(f.rows.size, 0);
  const backup = command();
  await assert.rejects(controller.submit(backup));
  assert.equal(
    controller.getSnapshot().pending.length,
    1,
    'do not reinterpret other operations as definitive diagnostics rejection',
  );
});

test('lost diagnostics receipt preserves original fingerprint, categories and output token', async () => {
  const f = fixture();
  const action = command();
  action.command = {
    kind: 'diagnostics_export',
    payload: {
      command_id: action.id,
      token: 'original-grant',
      categories: ['versions', 'errors'],
      fingerprint: 'original-hash',
    },
  };
  f.lost = true;
  await assert.rejects(f.open().submit(action));
  action.command.payload.fingerprint = 'new-preview';
  action.command.payload.categories = ['statistics'];
  f.lost = false;
  await f.open().retry(action.id);
  assert.deepEqual(f.calls[0], f.calls[1]);
  assert.equal(f.receipts.size, 1);
});
test('lost acceptance survives reopening and replays the same scoped command', async () => {
  const f = fixture();
  f.lost = true;
  const action = command();
  await assert.rejects(f.open().submit(action), /receipt lost/);
  action.command.payload.token = 'later-input';
  action.identity.local_instance_id = 'other-instance';
  const reopened = f.open();
  await reopened.load();
  f.lost = false;
  await reopened.retry(action.id);
  assert.deepEqual(f.calls[0], f.calls[1]);
  assert.equal(f.receipts.size, 1);
  assert.equal(reopened.getSnapshot().pending.length, 0);
});
test('cleanup failure retains accepted intent and recovers with the original receipt', async () => {
  const f = fixture();
  f.cleanupFail = true;
  const action = command();
  await assert.rejects(f.open().submit(action), /cleanup failed/);
  assert.equal(f.rows.size, 1);
  f.cleanupFail = false;
  await f.open().retry(action.id);
  assert.equal(f.receipts.size, 1);
  assert.equal(f.rows.size, 0);
});
test('definitive rejection is removed while transport, identity and receipt uncertainty are retained', async () => {
  for (const code of [
    'invalid_input',
    'input_unavailable',
    'not_found',
    'io_failed',
    'workspace_mismatch',
    'idempotency_mismatch',
  ]) {
    const f = fixture();
    f.error = Object.assign(Error(code), { details: { code } });
    const action = command();
    await assert.rejects(f.open().submit(action));
    assert.equal(
      f.rows.has(action.id),
      !['invalid_input', 'input_unavailable', 'not_found'].includes(code),
      code,
    );
  }
});
test('caller edits during pending persistence cannot change the submitted intent', async () => {
  const f = fixture();
  const action = command();
  const expected = structuredClone(action);
  const result = f.open().submit(action);
  action.command.payload.token = 'mutated';
  action.displayName = 'changed';
  await result;
  assert.deepEqual(f.calls[0], expected);
});
test('concurrent submissions or retries for one ID are rejected without duplicate sends', async () => {
  const f = fixture();
  const action = command();
  const controller = f.open();
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const send = f.send;
  f.send = async (entry) => {
    await gate;
    return send(entry);
  };
  const blocked = new PendingActions(f.storage, f.send);
  const first = blocked.submit(action);
  await new Promise((resolve) => setImmediate(resolve));
  await assert.rejects(blocked.submit(action));
  await assert.rejects(blocked.retry(action.id));
  release();
  await first;
  assert.equal(f.calls.length, 1);
  await controller.load();
  assert.equal(controller.getSnapshot().pending.length, 0);
});
test('request and command IDs must match before anything is written', async () => {
  const f = fixture();
  const action = command();
  action.id = 'different';
  await assert.rejects(f.open().submit(action), /mismatch/);
  assert.equal(f.rows.size, 0);
  assert.equal(f.calls.length, 0);
});
test('definitive rejection names the dropped request instead of a bare banner line', async () => {
  const f = fixture();
  f.error = Object.assign(Error('输入内容不符合要求，请检查后重试。'), {
    details: { code: 'invalid_input' },
  });
  const action = command();
  const controller = f.open();
  await assert.rejects(controller.submit(action), /输入内容不符合要求/);
  const snapshot = controller.getSnapshot();
  assert.equal(snapshot.pending.length, 0, 'the refused request leaves the queue');
  assert.equal(f.rows.size, 0, 'and its local record is dropped');
  assert.equal(snapshot.error, null, 'no unattributed line is left in the banner');
  assert.deepEqual(snapshot.notice, {
    id: action.id,
    kind: 'backup_create',
    displayName: 'archive.zip',
    createdAt: action.createdAt,
    message: '输入内容不符合要求，请检查后重试。',
  });
  controller.dismissNotice();
  assert.equal(controller.getSnapshot().notice, null);
});

test('an uncertain failure keeps the request and reports an error, never a notice', async () => {
  const f = fixture();
  f.error = Object.assign(Error('io_failed'), { details: { code: 'io_failed' } });
  const controller = f.open();
  await assert.rejects(controller.submit(command()), /io_failed/);
  const snapshot = controller.getSnapshot();
  assert.equal(
    snapshot.notice,
    null,
    'uncertain acceptance must stay a request, not a dropped one',
  );
  assert.equal(snapshot.error, 'io_failed');
  assert.equal(snapshot.pending.length, 1, 'the request stays available for the exact retry');
});

test('a retry refused for good is named by the notice and leaves no banner line', async () => {
  const f = fixture();
  f.error = Object.assign(Error('io_failed'), { details: { code: 'io_failed' } });
  const action = command();
  const controller = f.open();
  await assert.rejects(controller.submit(action), /io_failed/);
  assert.equal(
    controller.getSnapshot().error,
    'io_failed',
    'an uncertain failure stays a banner line while the request is queued',
  );
  f.error = Object.assign(Error('所选文件或位置已改变，或授权已过期，请重新选择。'), {
    details: { code: 'input_unavailable' },
  });
  await assert.rejects(controller.retry(action.id), /授权已过期/);
  const snapshot = controller.getSnapshot();
  assert.deepEqual(snapshot.notice, {
    id: action.id,
    kind: 'backup_create',
    displayName: 'archive.zip',
    createdAt: action.createdAt,
    message: '所选文件或位置已改变，或授权已过期，请重新选择。',
  });
  assert.equal(snapshot.error, null, 'the queue clears its own line when it names the request');
  assert.equal(snapshot.pending.length, 0, 'the refused request leaves the queue');
  assert.equal(f.rows.size, 0, 'and its local record is dropped');
});

test('the action banner text stays quiet only for a request the queue already named', () => {
  const action = command();
  const refusal = Object.assign(Error('输入内容不符合要求，请检查后重试。'), {
    details: { code: 'invalid_input' },
  });
  assert.equal(
    actionErrorText(refusal, action),
    null,
    'notice and banner must not repeat one failure',
  );
  assert.equal(actionErrorText(refusal, undefined), '输入内容不符合要求，请检查后重试。');
  const uncertain = Object.assign(Error('io_failed'), { details: { code: 'io_failed' } });
  assert.equal(actionErrorText(uncertain, action), 'io_failed');
  const diagnostics = {
    ...action,
    command: {
      kind: 'diagnostics_export',
      payload: { command_id: action.id, token: 'grant', categories: [], fingerprint: 'hash' },
    },
  };
  const conflict = Object.assign(Error('statistics changed'), {
    details: { code: 'app_state_conflict' },
  });
  assert.equal(actionErrorText(conflict, diagnostics), null);
  assert.equal(actionErrorText(conflict, action), 'statistics changed');
});
