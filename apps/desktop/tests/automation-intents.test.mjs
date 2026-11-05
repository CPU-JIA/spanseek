import test from 'node:test';
import assert from 'node:assert/strict';
import { AutomationActions, automationIntentKey } from '../src/automation/automation-intents.ts';
import { readBoundedCursorPage } from '../src/automation/bounded-cursor-page.ts';

const id = (n) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;
const identity = { local_instance_id: id(1), workspace_id: id(2) };
const command = (kind = 'research_create') => {
  const requests = {
    automation_create_change_set: {
      kind: 'tag_suggestions',
      limit: 20,
      collection_id: null,
      stale_after_days: null,
    },
    automation_apply_item: { change_id: id(10), item_id: id(11), confirm: true },
    automation_skip_item: { change_id: id(10), item_id: id(11), confirm: false },
    automation_reject_change_set: { change_id: id(10) },
    research_create: {
      goal: 'Original goal',
      queries: ['private query'],
      collection_id: null,
      budget: { max_steps: 5, max_sources: 10 },
      stop: { deadline_seconds: null, missing_after_step: null },
      external_allowed: false,
      external_search_endpoint: null,
    },
    research_advance: { task_id: id(20), base_row_version: 3 },
    research_cancel: { task_id: id(20), base_row_version: 3, reason: 'Original reason' },
    audit_undo: { audit_id: id(30), confirm: true },
    review_insight_settings_set: {
      base_row_version: 2,
      enabled: true,
      window_days: 30,
      include_completed: true,
      include_stale: false,
    },
  };
  return {
    kind,
    payload:
      kind === 'research_run_external'
        ? { command_id: id(99), task_id: id(20), base_row_version: 3 }
        : { command_id: id(99), request: requests[kind] },
  };
};
const reply = (c) => ({
  receipt: { command_id: c.payload.command_id },
  ...(c.kind.startsWith('research_')
    ? { task: { id: id(20) } }
    : c.kind.startsWith('automation_')
      ? { change: { id: id(10) }, item: { item_id: id(11) } }
      : c.kind === 'audit_undo'
        ? { reversed: { id: id(31), kind: 'audit.undo' } }
        : { settings: { row_version: 3 } }),
});
function memory() {
  const records = new Map();
  return {
    records,
    async read(key) {
      return structuredClone(records.get(key));
    },
    async claim(intent) {
      const previous = records.get(intent.key);
      if (previous) assert.deepEqual(previous, intent);
      records.set(intent.key, structuredClone(intent));
    },
    async confirm(intent) {
      const previous = records.get(intent.key);
      if (previous) assert.deepEqual(previous, intent);
      records.delete(intent.key);
    },
  };
}
for (const kind of [
  'automation_create_change_set',
  'automation_apply_item',
  'automation_skip_item',
  'automation_reject_change_set',
  'research_create',
  'research_advance',
  'research_run_external',
  'research_cancel',
  'audit_undo',
  'review_insight_settings_set',
])
  test(`${kind}: lost receipt and reopened page replay the exact committed request`, async () => {
    const storage = memory();
    const submitted = [];
    const commits = new Map();
    let loss = true;
    const send = async (request) => {
      submitted.push(request);
      if (!commits.has(request.payload.command_id))
        commits.set(request.payload.command_id, reply(request));
      if (loss) {
        loss = false;
        throw new Error('reply lost');
      }
      return commits.get(request.payload.command_id);
    };
    const actions = await AutomationActions.open(identity, storage, send);
    const original = command(kind);
    assert.equal(await actions.run(original), null);
    assert.equal(commits.size, 1);
    assert.deepEqual(actions.getSnapshot().pending.command, original);
    assert.equal(await actions.run(command(kind)), null);
    const reopened = await AutomationActions.open(identity, storage, send);
    assert.ok(await reopened.run());
    assert.deepEqual(submitted, [original, original]);
    assert.equal(commits.size, 1);
    assert.equal(storage.records.size, 0);
  });
test('freezes the entire input before the first asynchronous storage boundary', async () => {
  const storage = memory();
  const sent = [];
  const actions = await AutomationActions.open(identity, storage, async (request) => {
    sent.push(request);
    return reply(request);
  });
  const original = command();
  const expected = structuredClone(original);
  const operation = actions.run(original);
  original.payload.request.queries.push('later draft');
  original.payload.command_id = id(100);
  await operation;
  assert.deepEqual(sent, [expected]);
});
test('failed durable claim never sends; its exact unsent request can be retried', async () => {
  const storage = memory();
  let broken = true;
  let sends = 0;
  const claim = storage.claim;
  storage.claim = async (intent) => {
    if (broken) throw new Error('disk unavailable');
    await claim(intent);
  };
  const actions = await AutomationActions.open(identity, storage, async (request) => {
    sends++;
    return reply(request);
  });
  await actions.run(command());
  assert.equal(sends, 0);
  assert.ok(actions.getSnapshot().pending);
  broken = false;
  await actions.run();
  assert.equal(sends, 1);
});
test('cleanup failure retains the committed command for receipt replay', async () => {
  const storage = memory();
  const confirm = storage.confirm;
  let broken = true;
  let sends = 0;
  storage.confirm = async (intent) => {
    if (broken) throw new Error('cleanup failed');
    await confirm(intent);
  };
  const actions = await AutomationActions.open(identity, storage, async (request) => {
    sends++;
    return reply(request);
  });
  assert.equal(await actions.run(command()), null);
  assert.equal(storage.records.size, 1);
  broken = false;
  assert.ok(await actions.run());
  assert.equal(sends, 2);
  assert.equal(storage.records.size, 0);
});
test('leaving the active UI keeps its committed outcome available until consumed', async () => {
  const storage = memory();
  const actions = await AutomationActions.open(identity, storage, async (request) =>
    reply(request),
  );
  assert.equal(await actions.run(command(), () => false), null);
  assert.equal(storage.records.size, 1);
  let consumed = false;
  await actions.run(undefined, (outcome) => {
    assert.equal(outcome.result.task.id, id(20));
    consumed = true;
    assert.equal(storage.records.size, 1);
    return true;
  });
  assert.ok(consumed);
  assert.equal(storage.records.size, 0);
});
test('definitive CAS rejection clears the request; unknown identity and IO failures retain it', async () => {
  for (const code of [
    'metadata_conflict',
    'revision_conflict',
    'invalid_input',
    'object_trashed',
    'not_found',
    'workspace_changed',
    'io',
  ]) {
    const storage = memory();
    const actions = await AutomationActions.open(identity, storage, async () => {
      throw Object.assign(new Error(code), { details: { code } });
    });
    await actions.run(command());
    assert.equal(!!actions.getSnapshot().pending, ['workspace_changed', 'io'].includes(code), code);
  }
});
test('mismatched receipt, object and item cannot clear the original request', async () => {
  for (const transform of [
    (result) => ({ ...result, receipt: { command_id: id(200) } }),
    (result) => ({ ...result, change: { id: id(200) } }),
    (result) => ({ ...result, item: { item_id: id(200) } }),
  ]) {
    const storage = memory();
    const actions = await AutomationActions.open(identity, storage, async (request) =>
      transform(reply(request)),
    );
    await actions.run(command('automation_apply_item'));
    assert.equal(storage.records.size, 1);
  }
});
test('corrupt or wrong-workspace storage is retained and blocks initialization', async () => {
  const storage = memory();
  storage.records.set(automationIntentKey(identity), {
    schema: 1,
    identity: { ...identity, workspace_id: id(5) },
  });
  await assert.rejects(
    AutomationActions.open(identity, storage, async (request) => reply(request)),
  );
  assert.equal(storage.records.size, 1);
});
test('damaged nested or required payload fields never reach the rejection cleanup path', async () => {
  const cases = [
    [
      'research_create',
      (request) => {
        request.budget.unexpected = 1;
      },
    ],
    [
      'research_create',
      (request) => {
        request.unexpected = true;
      },
    ],
    [
      'research_create',
      (request) => {
        request.budget = {};
      },
    ],
    [
      'research_create',
      (request) => {
        delete request.budget.max_sources;
      },
    ],
    [
      'research_create',
      (request) => {
        request.stop = {};
      },
    ],
    [
      'research_create',
      (request) => {
        request.stop.deadline_seconds = '30';
      },
    ],
    [
      'research_create',
      (request) => {
        request.budget.max_steps = -1;
      },
    ],
    [
      'research_create',
      (request) => {
        request.budget.max_sources = 2 ** 32;
      },
    ],
    [
      'research_create',
      (request) => {
        delete request.collection_id;
      },
    ],
    [
      'research_create',
      (request) => {
        request.external_search_endpoint = 42;
      },
    ],
    [
      'automation_create_change_set',
      (request) => {
        delete request.stale_after_days;
      },
    ],
    [
      'automation_create_change_set',
      (request) => {
        request.collection_id = 'not-an-id';
      },
    ],
    [
      'review_insight_settings_set',
      (request) => {
        request.window_days = '30';
      },
    ],
  ];
  for (const [kind, damage] of cases) {
    const storage = memory();
    const original = command(kind);
    damage(original.payload.request);
    const record = {
      schema: 1,
      key: automationIntentKey(identity),
      identity,
      createdAt: new Date().toISOString(),
      command: original,
    };
    storage.records.set(record.key, structuredClone(record));
    let sends = 0;
    await assert.rejects(
      AutomationActions.open(identity, storage, async () => {
        sends++;
        throw Object.assign(new Error('invalid input'), { details: { code: 'invalid_input' } });
      }),
    );
    assert.equal(sends, 0);
    assert.deepEqual(storage.records.get(record.key), record);
  }
});
test('atomic slot conflict adopts the persisted original instead of replacing it', async () => {
  const storage = memory();
  const older = await AutomationActions.open(identity, storage, async () => {
    throw new Error('loss');
  });
  const other = await AutomationActions.open(identity, storage, async (request) => reply(request));
  await older.run(command());
  const different = command();
  different.payload.command_id = id(100);
  await other.run(different);
  assert.deepEqual(other.getSnapshot().pending.command, command());
  assert.ok(await other.run());
});
test('100 rows use two real 50-row cursors and continuation reaches all 121 rows', async () => {
  const data = Array.from({ length: 121 }, (_, i) => i);
  const calls = [];
  const read = async (cursor, size) => {
    calls.push([cursor, size]);
    const start = cursor === null ? 0 : Number(cursor);
    const items = data.slice(start, start + size);
    return {
      items,
      next_cursor: start + items.length < data.length ? String(start + items.length) : null,
    };
  };
  const first = await readBoundedCursorPage(null, 100, 50, read);
  const second = await readBoundedCursorPage(first.next_cursor, 100, 50, read);
  assert.deepEqual(calls, [
    [null, 50],
    ['50', 50],
    ['100', 50],
  ]);
  assert.deepEqual([...first.items, ...second.items], data);
  assert.equal(second.next_cursor, null);
});
test('later page failure rejects the entire UI page and repeated cursors cannot loop', async () => {
  await assert.rejects(
    readBoundedCursorPage(null, 100, 50, async (cursor) => {
      if (cursor) throw new Error('second read failed');
      return { items: Array(50).fill(0), next_cursor: '50' };
    }),
    /second read failed/,
  );
  await assert.rejects(
    readBoundedCursorPage('50', 100, 50, async () => ({ items: [], next_cursor: '50' })),
    /continuation/,
  );
});
