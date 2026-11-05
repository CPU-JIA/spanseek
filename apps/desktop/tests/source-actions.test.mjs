import test from 'node:test';
import assert from 'node:assert/strict';
import { SourceActions, validateIntent, intentId } from '../src/sources/source-actions.ts';
const identity = { local_instance_id: 'instance', workspace_id: 'workspace' };
test('source task retry replays its exact command after an unknown reply', async () => {
  const disk = storage();
  const sent = [];
  const first = new SourceActions('tasks', identity, disk, async (intent) => {
    sent.push(intent);
    throw new Error('lost reply');
  });
  await first.load();
  await first.execute(
    { kind: 'source_job_retry', payload: { command_id: 'retry-A', job_id: 'job-A' } },
    'job_accepted',
  );
  assert.equal(first.getSnapshot().pending.command.payload.command_id, 'retry-A');
  const reopened = new SourceActions('tasks', identity, disk, async (intent) => {
    sent.push(intent);
  });
  await reopened.load();
  await reopened.retry();
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(disk.get(), undefined);
  assert.throws(
    () =>
      validateIntent(
        { key: 'tasks', identity, command: sent[0].command, expected: 'source_mutation' },
        'tasks',
        identity,
      ),
    /does not match/,
  );
});
const command = (id) => ({
  kind: 'sources_update',
  payload: {
    command_id: id,
    request: { source_id: 'source', base_row_version: 3, display_title: '中文' },
  },
});
function storage() {
  let record;
  return {
    read: async () => structuredClone(record),
    put: async (value) => {
      assert.equal(record, undefined);
      record = structuredClone(value);
    },
    remove: async (intent) => {
      assert.equal(intentId(record), intentId(intent));
      record = undefined;
    },
    get: () => record,
  };
}
test('unknown commit survives remount and replays immutable ID and base version', async () => {
  const disk = storage(),
    sent = [];
  const first = new SourceActions('scope', identity, disk, async (intent) => {
    sent.push(intent);
    throw new Error('lost reply');
  });
  await first.load();
  const input = command('request-A');
  await first.execute(input, 'source_mutation');
  input.payload.request.base_row_version = 99;
  assert.equal(first.getSnapshot().locked, true);
  assert.equal(disk.get().command.payload.request.base_row_version, 3);
  const reopened = new SourceActions('scope', identity, disk, async (intent) => {
    sent.push(intent);
  });
  await reopened.load();
  await reopened.retry();
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(disk.get(), undefined);
  assert.equal(reopened.getSnapshot().locked, false);
});
test('shared scope prevents a new request while the old response is pending', async () => {
  const disk = storage();
  let complete;
  const controller = new SourceActions(
    'scope',
    identity,
    disk,
    () => new Promise((resolve) => (complete = resolve)),
  );
  await controller.load();
  const a = controller.execute(command('A'), 'source_mutation');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(await controller.execute(command('B'), 'source_mutation'), false);
  assert.equal(intentId(disk.get()), 'A');
  complete();
  await a;
  assert.equal(disk.get(), undefined);
});
test('failed durable storage does not dispatch and wrong workspace cannot replay', async () => {
  let sent = 0;
  const broken = {
    read: async () => undefined,
    put: async () => {
      throw new Error('disk');
    },
    remove: async () => {},
  };
  const controller = new SourceActions('scope', identity, broken, async () => sent++);
  await controller.load();
  await controller.execute(command('A'), 'source_mutation');
  assert.equal(sent, 0);
  assert.throws(
    () =>
      validateIntent(
        {
          key: 'scope',
          identity: { ...identity, workspace_id: 'other' },
          command: command('A'),
          expected: 'source_mutation',
        },
        'scope',
        identity,
      ),
    /invalid/,
  );
});

test('accepted import task identity survives local acknowledgement cleanup failure', async () => {
  const disk = storage();
  const remove = disk.remove;
  let firstRemoval = true;
  disk.remove = async (intent) => {
    if (firstRemoval) {
      firstRemoval = false;
      throw new Error('receipt cleanup unavailable');
    }
    return remove(intent);
  };
  const sent = [];
  const controller = new SourceActions('imports', identity, disk, async (intent) => {
    sent.push(intent.command);
    return { job_id: 'accepted-job' };
  });
  await controller.load();
  const request = {
    kind: 'source_job_retry',
    payload: { command_id: 'same-request', job_id: 'accepted-job' },
  };
  assert.equal(await controller.execute(request, 'job_accepted'), false);
  assert.deepEqual(controller.getSnapshot().lastJob, {
    commandId: 'same-request',
    jobId: 'accepted-job',
  });
  assert.equal(controller.getSnapshot().pending.command.payload.command_id, 'same-request');
  await controller.retry();
  assert.deepEqual(sent[0], sent[1]);
  assert.equal(controller.getSnapshot().pending, null);
  assert.equal(controller.getSnapshot().lastJob.jobId, 'accepted-job');
});

test('an in-flight write only becomes unconfirmed once an attempt loses its reply', async () => {
  const disk = storage();
  let complete;
  let fail = false;
  const controller = new SourceActions('scope', identity, disk, () =>
    fail ? Promise.reject(new Error('lost reply')) : new Promise((resolve) => (complete = resolve)),
  );
  await controller.load();
  assert.equal(controller.getSnapshot().unconfirmed, false);
  const inflight = controller.execute(command('A'), 'source_mutation');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(intentId(controller.getSnapshot().pending), 'A');
  assert.equal(controller.getSnapshot().unconfirmed, false);
  complete();
  assert.equal(await inflight, true);
  assert.equal(controller.getSnapshot().unconfirmed, false);
  assert.equal(disk.get(), undefined);

  fail = true;
  assert.equal(await controller.execute(command('B'), 'source_mutation'), false);
  assert.equal(controller.getSnapshot().unconfirmed, true);
  assert.equal(intentId(controller.getSnapshot().pending), 'B');

  const reopened = new SourceActions('scope', identity, disk, async () => {});
  await reopened.load();
  assert.equal(reopened.getSnapshot().unconfirmed, true);
  await reopened.retry();
  assert.equal(reopened.getSnapshot().unconfirmed, false);
  assert.equal(reopened.getSnapshot().pending, null);
});

test('a definite rejection clears the request without asking for confirmation', async () => {
  const disk = storage();
  const controller = new SourceActions('scope', identity, disk, async () => {
    throw Object.assign(new Error('gone'), { details: { code: 'not_found' } });
  });
  await controller.load();
  assert.equal(await controller.execute(command('A'), 'source_mutation'), false);
  assert.equal(controller.getSnapshot().pending, null);
  assert.equal(controller.getSnapshot().unconfirmed, false);
  assert.ok(controller.getSnapshot().error);
});
