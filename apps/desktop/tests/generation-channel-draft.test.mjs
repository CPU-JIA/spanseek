import test from 'node:test';
import assert from 'node:assert/strict';
import { GenerationChannelDraft } from '../src/inference/generation-channel-draft.ts';
import { ModelDraft } from '../src/inference/model-draft.ts';
import {
  channelProtocolChange,
  channelRequestPreview,
  cloudflareEndpoint,
  createChannelSpec,
  newChannelModel,
  validateChannelSpec,
} from '../src/inference/generation-channel-presets.ts';

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const channel = (id = 'A', row_version = 1) => ({
  id,
  row_version,
  deleted: false,
  created_at: '2026-10-09T00:00:00Z',
  updated_at: '2026-10-09T00:00:00Z',
  spec: {
    ...createChannelSpec('openai'),
    name: `Channel ${id}`,
    endpoint: `https://${id.toLowerCase()}.example/v1`,
    models: [newChannelModel(`${id}-model`)],
  },
});
const profileFor = (saved, model = saved.spec.models[0].model) => ({
  ...saved.spec.models.find((item) => item.model === model),
  provider: saved.spec.provider,
  endpoint: saved.spec.endpoint,
  model_digest: '',
  config_revision: 2,
  channel_id: saved.id,
  capabilities: structuredClone(saved.spec.capabilities),
});
const server = (channels = [channel()], profile = null, row_version = 1) => ({
  channels,
  settings: { profile, row_version },
});
const mutation = (saved, settings = { profile: null, row_version: 2 }) => ({
  channel: saved,
  settings,
  receipt: {},
});
const catalog = (
  saved,
  models = [{ model: `${saved.id}-discovered`, display_name: null, context_tokens: null }],
) => ({
  channel_id: saved.id,
  row_version: saved.row_version,
  provider: saved.spec.provider,
  endpoint: saved.spec.endpoint,
  models,
});
const ready = async (channels = [channel()], profile = null, version = 1) => {
  const draft = new GenerationChannelDraft();
  await draft.load(async () => server(channels, profile, version));
  return draft;
};

test('named channels are independent and an existing legacy profile is not migrated or rewritten', async () => {
  const legacy = { ...profileFor(channel()), channel_id: undefined, capabilities: undefined };
  const draft = await ready([], legacy, 8);
  draft.create(createChannelSpec('openai'), 'openai', 'first');
  draft.change({ name: 'Personal', endpoint: 'https://one.example/v1' });
  draft.create(createChannelSpec('openai'), 'openai', 'second');
  draft.change({ name: 'Work', endpoint: 'https://two.example/v1' });
  draft.viewChannel('first');
  assert.equal(draft.selected.spec.name, 'Personal');
  assert.equal(draft.selected.spec.endpoint, 'https://one.example/v1');
  assert.equal(draft.getSnapshot().drafts.second.spec.name, 'Work');
  assert.deepEqual(draft.getSnapshot().server.settings, { profile: legacy, row_version: 8 });
  assert.equal(draft.getSnapshot().server.channels.length, 0);
});

test('refresh keeps channel drafts and their original version, while detecting an external edit', async () => {
  const a = channel('A'),
    b = channel('B');
  const draft = await ready([a, b]);
  draft.change({ name: 'Local A' });
  draft.viewChannel('B');
  draft.change({ endpoint: 'https://local-b.example/v1' });
  const updated = channel('A', 2);
  updated.spec.name = 'External A';
  await draft.load(async () => server([updated, b], null, 3));
  assert.equal(draft.selected.id, 'B');
  assert.equal(draft.selected.spec.endpoint, 'https://local-b.example/v1');
  draft.viewChannel('A');
  assert.equal(draft.selected.spec.name, 'Local A');
  assert.equal(draft.selected.baseRowVersion, 1);
  assert.equal(draft.selected.conflict, true);
  assert.equal(draft.canSave, false);
  draft.discardSelected();
  assert.equal(draft.selected.spec.name, 'External A');
  assert.equal(draft.selected.baseRowVersion, 2);
  assert.equal(draft.getSnapshot().drafts.B.dirty, true);
});

test('a late refresh cannot replace a newer response or edits made while reading', async () => {
  const draft = await ready();
  const old = deferred(),
    fresh = deferred();
  const oldRead = draft.load(() => old.promise);
  const freshRead = draft.load(() => fresh.promise);
  draft.change({ name: 'Typed during refresh' });
  fresh.resolve(server([channel('A', 3)], null, 6));
  await freshRead;
  old.resolve(server([channel('A', 2)], null, 2));
  await oldRead;
  assert.equal(draft.selected.spec.name, 'Typed during refresh');
  assert.equal(draft.getSnapshot().server.channels[0].row_version, 3);
  assert.equal(draft.getSnapshot().server.settings.row_version, 6);
  assert.equal(draft.getSnapshot().loading, false);
});

test('a channel can be saved without models or credentials, using both base versions', async () => {
  const draft = await ready([], null, 11);
  draft.create(createChannelSpec('gemini'), 'gemini', 'new-gemini');
  assert.equal(draft.canSave, true);
  let calls = 0;
  await draft.save(async (request) => {
    calls++;
    assert.equal(request.channel_id, 'new-gemini');
    assert.equal(request.base_row_version, 0);
    assert.equal(request.base_generation_row_version, 11);
    assert.deepEqual(request.spec.models, []);
    assert.equal('api_key' in request.spec, false);
    return mutation(
      { ...channel('new-gemini'), spec: request.spec },
      { profile: null, row_version: 12 },
    );
  });
  assert.equal(calls, 1);
  assert.equal(draft.selected.baseRowVersion, 1);
  assert.equal(draft.selected.dirty, false);
  assert.equal(draft.getSnapshot().credentialConfigured, null);
  assert.deepEqual(draft.getSnapshot().verifiedModels, []);
});

test('save results update their own channel and preserve edits typed after the request', async () => {
  const draft = await ready([channel('A'), channel('B')], null, 4);
  draft.change({ name: 'Submitted A' });
  const completion = deferred();
  let request;
  const saving = draft.save((value) => {
    request = value;
    return completion.promise;
  });
  draft.change({ name: 'Newer A input' });
  draft.viewChannel('B');
  draft.change({ name: 'Local B' });
  completion.resolve(
    mutation({ ...channel('A', 2), spec: request.spec }, { profile: null, row_version: 5 }),
  );
  await saving;
  assert.equal(draft.selected.id, 'B');
  assert.equal(draft.selected.spec.name, 'Local B');
  draft.viewChannel('A');
  assert.equal(draft.selected.spec.name, 'Newer A input');
  assert.equal(draft.selected.baseRowVersion, 2);
  assert.equal(draft.selected.dirty, true);
  assert.equal(
    draft.getSnapshot().server.channels.find((item) => item.id === 'A').spec.name,
    'Submitted A',
  );
});

test('a save conflict retains input and permits an explicit copy without rebasing the old channel', async () => {
  const draft = await ready();
  draft.change({ name: 'My retained channel' });
  await draft.save(async () => {
    throw { details: { code: 'metadata_conflict' } };
  });
  assert.equal(draft.selected.spec.name, 'My retained channel');
  assert.equal(draft.selected.baseRowVersion, 1);
  assert.equal(draft.getSnapshot().error, 'channel_conflict');
  await draft.load(async () => server([channel('A', 2)], null, 3));
  const copied = draft.copySelected('Retained copy');
  assert.ok(copied && copied !== 'A');
  assert.equal(draft.selected.baseRowVersion, 0);
  assert.equal(draft.selected.spec.name, 'Retained copy');
  assert.equal(draft.getSnapshot().drafts.A.spec.name, 'My retained channel');
  assert.equal(draft.getSnapshot().drafts.A.conflict, true);
});

test('reconfirming a saved mutation retains newer local input and disposed pages ignore it', async () => {
  const draft = await ready();
  draft.change({ name: 'Sent first' });
  const submitted = structuredClone(draft.selected.spec);
  draft.change({ name: 'Kept after lost reply' });
  draft.confirmMutation(mutation({ ...channel('A', 2), spec: submitted }), submitted);
  assert.equal(draft.selected.spec.name, 'Kept after lost reply');
  assert.equal(draft.selected.baseRowVersion, 2);
  draft.invalidate();
  const before = draft.getSnapshot();
  draft.confirmMutation(mutation(channel('A', 99)));
  assert.equal(draft.getSnapshot(), before);
});

test('model catalog responses are bound to channel, revision, protocol and endpoint', async () => {
  const a = channel('A'),
    b = channel('B');
  const draft = await ready([a, b]);
  await draft.credentialOperation(async () => true);
  const old = deferred(),
    fresh = deferred();
  const first = draft.loadModels(() => old.promise);
  draft.viewChannel('B');
  await draft.credentialOperation(async () => true);
  const second = draft.loadModels(() => fresh.promise);
  old.resolve(catalog(a));
  await first;
  assert.equal(draft.getSnapshot().catalog, null);
  assert.equal(draft.getSnapshot().listing, true);
  fresh.resolve(catalog(b));
  await second;
  assert.equal(draft.getSnapshot().catalog.channel_id, 'B');
  await draft.loadModels(async () => ({ ...catalog(b), row_version: 99 }));
  assert.equal(draft.getSnapshot().catalog, null);
  assert.equal(draft.getSnapshot().catalogError, 'channel_catalog_mismatch');
  assert.deepEqual(draft.selected.spec.models, b.spec.models);
});

test('catalog failure preserves saved manual models, and adding a model keeps the same-connection catalog', async () => {
  const saved = channel(),
    draft = await ready([saved]);
  await draft.credentialOperation(async () => true);
  await draft.loadModels(async () => {
    throw Error('synthetic remote error');
  });
  assert.deepEqual(draft.selected.spec.models, saved.spec.models);
  assert.equal(draft.getSnapshot().catalogError, 'channel_catalog_failed');
  await draft.loadModels(async () => catalog(saved));
  draft.change({ models: [...draft.selected.spec.models, newChannelModel('manual-second')] });
  assert.equal(draft.getSnapshot().catalog.channel_id, saved.id);
  assert.equal(draft.selected.spec.models.length, 2);
  assert.equal(draft.savedChannel, null);
  await draft.loadModels(() => assert.fail('do not discover from an unsaved configuration'));
});

test('credential results cannot escape their channel and raw exceptions never enter snapshots', async () => {
  const draft = await ready([channel('A'), channel('B')]);
  const old = deferred(),
    fresh = deferred();
  const first = draft.credentialOperation(() => old.promise);
  draft.viewChannel('B');
  const second = draft.credentialOperation(() => fresh.promise);
  old.reject(Error('synthetic-api-secret-never-store'));
  await first;
  assert.equal(draft.getSnapshot().credentialBusy, true);
  assert.equal(draft.getSnapshot().credentialError, null);
  fresh.reject({
    message: 'synthetic-api-secret-never-store',
    details: { details: { reason: 'synthetic-api-secret-never-store' } },
  });
  await second;
  assert.equal(draft.getSnapshot().credentialError, 'channel_credential_failed');
  assert.equal(
    JSON.stringify(draft.getSnapshot()).includes('synthetic-api-secret-never-store'),
    false,
  );
  assert.equal('api_key' in draft.selected.spec, false);
});

test('verification uses the selected saved channel and a late success cannot verify changed settings', async () => {
  const saved = channel(),
    draft = await ready([saved], null, 9);
  await draft.credentialOperation(async () => true);
  const check = deferred();
  const pending = draft.verify('A-model', (profile) => {
    assert.equal(profile.channel_id, saved.id);
    assert.equal(profile.endpoint, saved.spec.endpoint);
    assert.deepEqual(profile.capabilities, saved.spec.capabilities);
    assert.equal(profile.config_revision, 9);
    return check.promise;
  });
  draft.change({ endpoint: 'https://changed.example/v1' });
  check.resolve();
  await pending;
  assert.deepEqual(draft.getSnapshot().verifiedModels, []);
  assert.equal(draft.selected.spec.endpoint, 'https://changed.example/v1');
  await draft.verify('A-model', () => assert.fail('unsaved endpoints must not be tested'));
});

test('select and delete use both versions and active deletion requires an explicit clear flag', async () => {
  const saved = channel(),
    draft = await ready([saved], profileFor(saved), 7);
  await draft.remove(false, () => assert.fail('do not delete the active channel implicitly'));
  assert.equal(draft.getSnapshot().error, 'channel_active_delete');
  await draft.selectModel('A-model', async (request) => {
    assert.deepEqual(request, {
      channel_id: 'A',
      model: 'A-model',
      base_row_version: 1,
      base_generation_row_version: 7,
    });
    return { settings: { profile: profileFor(saved), row_version: 8 }, receipt: {} };
  });
  await draft.remove(true, async (request) => {
    assert.equal(request.clear_active, true);
    assert.equal(request.base_generation_row_version, 8);
    return mutation({ ...saved, row_version: 2, deleted: true }, { profile: null, row_version: 9 });
  });
  assert.equal(draft.getSnapshot().server.settings.profile, null);
  assert.equal(draft.selected, null);
});

test('workspace disposal prevents old catalog and save outcomes from becoming current state', async () => {
  const draft = await ready();
  await draft.credentialOperation(async () => true);
  const old = deferred();
  const listing = draft.loadModels(() => old.promise);
  draft.invalidate();
  old.resolve(catalog(channel()));
  await listing;
  assert.equal(draft.getSnapshot().catalog, null);
  draft.change({ name: 'stale handler' });
  assert.equal(draft.selected.spec.name, 'Channel A');
  const fresh = await ready([channel('B')]);
  assert.equal(fresh.selected.id, 'B');
  assert.equal(fresh.getSnapshot().credentialConfigured, null);
});

test('Cloudflare account input survives switching, while explicit API mode controls transport', async () => {
  const draft = await ready([channel('other')]);
  draft.create(createChannelSpec('cloudflare'), 'cloudflare', 'cf');
  draft.changeCloudflareAccount('partial-id');
  draft.viewChannel('other');
  draft.viewChannel('cf');
  assert.equal(draft.selected.accountInput, 'partial-id');
  assert.equal(draft.selected.spec.endpoint, '');
  const account = 'a'.repeat(32);
  draft.changeCloudflareAccount(account);
  assert.equal(draft.selected.spec.endpoint, cloudflareEndpoint(account));
  const response = channelProtocolChange(draft.selected.spec, 'openai_responses');
  assert.equal(response.endpoint, cloudflareEndpoint(account));
  assert.deepEqual(response.capabilities, { text_transport: 'json', structured_transport: 'json' });
  assert.equal(validateChannelSpec(response), null);
  assert.equal(
    validateChannelSpec({
      ...response,
      capabilities: { text_transport: 'stream', structured_transport: 'json' },
    }),
    'channel_cloudflare_invalid',
  );
  assert.equal(cloudflareEndpoint('https://unexpected.example'), null);
});

test('protocol changes remove incompatible token fields and custom capabilities stay explicit', () => {
  const spec = createChannelSpec('deepseek');
  spec.models = [newChannelModel('synthetic-model', 'deepseek')];
  assert.equal(spec.models[0].chat_token_limit, 'max_tokens');
  spec.capabilities = { text_transport: 'json', structured_transport: null };
  const gemini = channelProtocolChange(spec, 'gemini');
  assert.equal('chat_token_limit' in gemini.models[0], false);
  assert.deepEqual(gemini.capabilities, spec.capabilities);
  assert.equal(gemini.endpoint, spec.endpoint);
  const preset = createChannelSpec('gemini');
  assert.equal(
    channelRequestPreview(preset),
    'https://generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent?alt=sse',
  );
  preset.models = [{ model: 'synthetic-long-context', context_tokens: 1048576, temperature: 0 }];
  assert.equal(validateChannelSpec(preset), null);
  preset.models[0].context_tokens = 2097153;
  assert.equal(validateChannelSpec(preset), 'channel_model_parameters_invalid');
});

test('channel settings handoff updates a clean legacy form, preserves edited input, and rejects stale settings', async () => {
  const form = new ModelDraft(),
    saved = channel();
  const legacy = { ...profileFor(saved), channel_id: undefined, capabilities: undefined };
  await form.load(async () => ({ profile: legacy, row_version: 1 }));
  form.change({ model: 'unsaved legacy model' });
  form.acceptChannelSettings({ profile: profileFor(saved), row_version: 2 });
  assert.equal(form.getSnapshot().draft.model, 'unsaved legacy model');
  assert.equal(form.getSnapshot().settings.profile.channel_id, saved.id);
  assert.equal(form.acceptChannelSettings({ profile: null, row_version: 1 }), false);
  const clean = new ModelDraft();
  await clean.load(async () => ({ profile: profileFor(saved), row_version: 2 }));
  clean.beginLegacyConfiguration();
  assert.equal(clean.getSnapshot().draft.channel_id, undefined);
  assert.equal(clean.getSnapshot().draft.capabilities, undefined);
  assert.equal(clean.getSnapshot().settings.profile.channel_id, saved.id);
  assert.equal(clean.getSnapshot().draft.model, '');
});

test('confirmed model discovery reasons stay explicit without retaining remote error bodies', async () => {
  const saved = channel(),
    draft = await ready([saved]);
  await draft.credentialOperation(async () => true);
  for (const reason of [
    'model_discovery_unsupported',
    'model_response_invalid',
    'model_response_limit',
  ]) {
    await draft.loadModels(async () => {
      throw { message: 'synthetic-private-remote-body', details: { details: { reason } } };
    });
    assert.equal(draft.getSnapshot().catalogError, reason);
    assert.equal(draft.getSnapshot().listing, false);
    assert.deepEqual(draft.selected.spec.models, saved.spec.models);
    assert.equal(
      JSON.stringify(draft.getSnapshot()).includes('synthetic-private-remote-body'),
      false,
    );
  }
});

test('unsupported structured output remains a capability error without marking the model verified', async () => {
  const draft = await ready();
  await draft.credentialOperation(async () => true);
  await draft.verify('A-model', async () => {
    throw { details: { details: { reason: 'model_unsupported_output' } } };
  });
  assert.equal(draft.getSnapshot().verificationError, 'model_unsupported_output');
  assert.equal(draft.getSnapshot().checkingModel, null);
  assert.deepEqual(draft.getSnapshot().verifiedModels, []);
});
