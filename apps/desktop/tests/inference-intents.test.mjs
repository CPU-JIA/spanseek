import test from 'node:test';
import assert from 'node:assert/strict';
import {
  validateInferenceIntent,
  storeInferenceIntent,
  confirmInferenceIntent,
} from '../src/inference/inference-intents.ts';
const identity = { local_instance_id: 'local-A', workspace_id: 'workspace' };
const intent = () => ({
  identity,
  command: {
    kind: 'answers_start',
    payload: {
      command_id: 'original-command',
      search: { query: 'original question', scope: { object_ids: ['source-A'] } },
    },
  },
  expected: 'answer_mutation',
});
function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}
test('knowledge task retry retains the selected material and template version across reload', () => {
  const disk = storage();
  const request = {
    identity,
    command: {
      kind: 'knowledge_tasks_start',
      payload: {
        command_id: 'task-original',
        request: {
          title: 'Compare two options',
          instructions: 'Compare evidence and unanswered questions.',
          template: { id: 'custom-template', row_version: 2 },
          materials: [{ id: 'evidence-1', text: 'Original material' }],
        },
      },
    },
    expected: 'knowledge_task_mutation',
  };
  storeInferenceIntent(disk, 'task', request);
  request.command.payload.request.template.row_version = 3;
  request.command.payload.request.materials[0].text = 'Changed later';
  const recovered = JSON.parse(disk.getItem('task'));
  validateInferenceIntent(recovered, identity);
  assert.equal(recovered.command.payload.request.template.row_version, 2);
  assert.equal(recovered.command.payload.request.materials[0].text, 'Original material');
  assert.throws(() => confirmInferenceIntent(disk, 'task', request), /record changed/);
  confirmInferenceIntent(disk, 'task', recovered);
  assert.equal(disk.getItem('task'), null);
});

test('unknown answer start preserves the exact serialized command and scope across reload', () => {
  const disk = storage(),
    request = intent();
  storeInferenceIntent(disk, 'key', request);
  request.command.payload.search.query = 'changed';
  const recovered = JSON.parse(disk.getItem('key'));
  validateInferenceIntent(recovered, identity);
  assert.equal(recovered.command.payload.command_id, 'original-command');
  assert.equal(recovered.command.payload.search.query, 'original question');
  assert.deepEqual(recovered.command.payload.search.scope.object_ids, ['source-A']);
  assert.throws(() => storeInferenceIntent(disk, 'key', request), /Pending request/);
  confirmInferenceIntent(disk, 'key', recovered);
  assert.equal(disk.getItem('key'), null);
});
test('wrong instance and mismatched reply kind cannot replay a pending mutation', () => {
  assert.throws(
    () => validateInferenceIntent(intent(), { ...identity, local_instance_id: 'restored-copy' }),
    /Invalid pending/,
  );
  assert.throws(
    () => validateInferenceIntent({ ...intent(), expected: 'note_commit' }, identity),
    /Invalid pending/,
  );
});

test('agent acceptance retries preserve the original scope and budget after reload', () => {
  const disk = storage();
  const request = intent();
  request.command.kind = 'answers_agent_start';
  request.command.payload.agent = {
    max_model_calls: 3,
    max_tool_calls: 4,
    max_evidence: 12,
    total_timeout_seconds: 120,
  };
  storeInferenceIntent(disk, 'agent', request);
  request.command.payload.agent.max_model_calls = 4;
  request.command.payload.search.scope.object_ids = ['different'];
  const restored = JSON.parse(disk.getItem('agent'));
  validateInferenceIntent(restored, identity);
  assert.equal(restored.command.payload.agent.max_model_calls, 3);
  assert.deepEqual(restored.command.payload.search.scope.object_ids, ['source-A']);
  confirmInferenceIntent(disk, 'agent', restored);
  assert.equal(disk.getItem('agent'), null);
});
test('an older response cannot clear a different pending request', () => {
  const disk = storage(),
    original = intent(),
    newer = intent();
  newer.command.payload.command_id = 'new-command';
  storeInferenceIntent(disk, 'key', newer);
  assert.throws(() => confirmInferenceIntent(disk, 'key', original), /record changed/);
  assert.equal(JSON.parse(disk.getItem('key')).command.payload.command_id, 'new-command');
});

test('embedding configuration retries keep endpoint but credentials cannot be persisted', () => {
  const disk = storage();
  const request = {
    identity,
    command: {
      kind: 'embedding_remote_configure',
      payload: {
        command_id: 'configure-original',
        endpoint: 'https://example.com/ai/v1',
      },
    },
    expected: 'embedding_profile_mutation',
  };
  validateInferenceIntent(request, identity);
  storeInferenceIntent(disk, 'embedding', request);
  const recovered = JSON.parse(disk.getItem('embedding'));
  validateInferenceIntent(recovered, identity);
  assert.equal(recovered.command.payload.endpoint, request.command.payload.endpoint);
  assert.throws(
    () =>
      validateInferenceIntent(
        {
          ...request,
          command: {
            ...request.command,
            payload: { ...request.command.payload, api_key: 'synthetic-do-not-store' },
          },
        },
        identity,
      ),
    /Invalid pending/,
  );
  assert.throws(
    () =>
      validateInferenceIntent(
        {
          ...request,
          command: {
            kind: 'embedding_credential_set',
            payload: { endpoint: 'https://example.com', api_key: 'synthetic' },
          },
        },
        identity,
      ),
    /Invalid pending/,
  );
});

test('storage itself rejects credential commands before any write', () => {
  const disk = storage();
  const request = {
    identity,
    command: {
      kind: 'embedding_credential_set',
      payload: {
        endpoint: 'https://example.com/ai/v1',
        api_key: 'synthetic',
        command_id: 'not-a-durable-command',
      },
    },
    expected: 'done',
  };
  assert.throws(() => storeInferenceIntent(disk, 'sensitive', request), /Invalid pending/);
  assert.equal(disk.getItem('sensitive'), null);
});

test('answer output budgets survive durable retry without substituting a new choice', () => {
  const data = new Map();
  const storage = {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => data.set(key, value),
    removeItem: (key) => data.delete(key),
  };
  for (const kind of ['answers_start', 'answers_agent_start']) {
    const request = intent();
    request.command.kind = kind;
    request.command.payload.max_output_tokens = 12288;
    if (kind === 'answers_agent_start')
      request.command.payload.agent = {
        max_model_calls: 4,
        max_tool_calls: 8,
        max_evidence: 32,
        total_timeout_seconds: 240,
      };
    storeInferenceIntent(storage, kind, request);
    const recovered = JSON.parse(storage.getItem(kind));
    assert.equal(recovered.command.payload.max_output_tokens, 12288);
    const changed = structuredClone(recovered);
    changed.command.payload.max_output_tokens = 8192;
    assert.throws(() => confirmInferenceIntent(storage, kind, changed));
    confirmInferenceIntent(storage, kind, recovered);
    assert.equal(storage.getItem(kind), null);
  }
});

test('new channel intents retain both versions and the selected protocol across retry', () => {
  for (const [kind, expected] of [
    ['generation_channel_save', 'generation_channel_mutation'],
    ['generation_channel_delete', 'generation_channel_mutation'],
    ['generation_channel_select', 'generation_settings_mutation'],
  ]) {
    const disk = storage();
    const change = {
      channel_id: 'stable-channel',
      base_row_version: 3,
      base_generation_row_version: 7,
      ...(kind.endsWith('_save')
        ? {
            spec: {
              provider: 'openai_responses',
              endpoint: 'https://example.invalid/v1',
              models: [],
            },
          }
        : {}),
      ...(kind.endsWith('_delete') ? { clear_active: true } : {}),
      ...(kind.endsWith('_select') ? { model: 'frozen-model-choice' } : {}),
    };
    const request = {
      identity,
      command: { kind, payload: { command_id: 'frozen-channel-command', request: change } },
      expected,
    };
    storeInferenceIntent(disk, kind, request);
    const expectedChange = structuredClone(change);
    change.base_generation_row_version = 99;
    const restored = JSON.parse(disk.getItem(kind));
    validateInferenceIntent(restored, identity);
    assert.deepEqual(restored.command.payload.request, expectedChange);
    confirmInferenceIntent(disk, kind, restored);
    assert.equal(disk.getItem(kind), null);
  }
});

test('new channel intents reject credential commands and nested secret fields before writing', () => {
  const disk = storage();
  const secret = 'synthetic-input-do-not-store';
  for (const kind of [
    'generation_channel_credential_set',
    'generation_channel_credential_status',
    'generation_channel_credential_delete',
  ]) {
    const request = {
      identity,
      command: {
        kind,
        payload: {
          command_id: 'not-durable',
          channel_id: 'channel',
          row_version: 1,
          api_key: secret,
        },
      },
      expected: 'done',
    };
    assert.throws(() => storeInferenceIntent(disk, kind, request), /Invalid pending/);
    assert.equal(disk.getItem(kind), null);
  }
  for (const credential of [{ api_key: secret }, { models: [{ api_key: secret }] }]) {
    const request = {
      identity,
      command: {
        kind: 'generation_channel_save',
        payload: {
          command_id: 'safe-id',
          request: {
            channel_id: 'channel',
            base_row_version: 0,
            base_generation_row_version: 0,
            spec: credential,
          },
        },
      },
      expected: 'generation_channel_mutation',
    };
    assert.throws(() => storeInferenceIntent(disk, 'nested', request), /Invalid pending/);
    assert.equal(disk.getItem('nested'), null);
  }
});
