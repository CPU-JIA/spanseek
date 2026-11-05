import test from 'node:test';
import assert from 'node:assert/strict';
import { ModelDraft } from '../src/inference/model-draft.ts';

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
test('new remote configuration leaves room for chat output while saved capacity stays explicit', async () => {
  const state = new ModelDraft();
  assert.equal(state.getSnapshot().draft.context_tokens, 65536);
  const saved = settings();
  saved.profile.provider = 'openai_chat';
  saved.profile.model_digest = '';
  await state.load(async () => saved);
  assert.equal(state.getSnapshot().draft.context_tokens, 8192);
});
const settings = (model = 'saved-model', row_version = 1) => ({
  row_version,
  profile: {
    provider: 'ollama',
    endpoint: 'http://127.0.0.1:11435',
    model,
    model_digest: 'a'.repeat(64),
    config_revision: 1,
    temperature: 0,
    context_tokens: 8192,
  },
});
const installed = (name = 'installed-fixture', digest = 'b'.repeat(64)) => ({
  name,
  digest,
  size: 1024,
  supported: true,
  reason: null,
  profile: {
    ...settings(name).profile,
    endpoint: 'http://127.0.0.1:11435/',
    model_digest: digest,
    context_tokens: 4096,
  },
  context_min: 2048,
  context_max: 8192,
});
const catalog = (models = [installed()], endpoint = 'http://127.0.0.1:11435/') => ({
  endpoint,
  runtime_version: 'fixture-runtime',
  models,
});

test('model selection uses catalog identity and server defaults without resetting the draft revision or endpoint', async () => {
  const state = new ModelDraft();
  const saved = settings();
  saved.profile.config_revision = 9;
  await state.load(async () => saved);
  const supported = installed();
  const unsupported = {
    ...installed('unsupported-fixture'),
    supported: false,
    profile: null,
    reason: 'model_profile_unsupported',
  };
  await state.loadModels(async (endpoint) => {
    assert.equal(endpoint, saved.profile.endpoint);
    return catalog([supported, unsupported]);
  });
  assert.equal(state.selectModel(unsupported.name, unsupported.digest), false);
  assert.equal(state.selectModel(supported.name, 'not-the-installed-digest'), false);
  assert.equal(state.selectModel(supported.name, supported.digest), true);
  assert.equal(state.getSnapshot().draft.model_digest, supported.digest);
  assert.equal(state.getSnapshot().draft.context_tokens, 4096);
  assert.equal(state.getSnapshot().draft.config_revision, 9);
  assert.equal(state.getSnapshot().draft.endpoint, saved.profile.endpoint);
  assert.equal(state.selectedModel.name, supported.name);
  assert.equal(state.canSave, false);
});

test('late model lists belong to their endpoint and cannot replace a newer endpoint request', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings());
  const old = deferred(),
    fresh = deferred();
  const first = state.loadModels(() => old.promise);
  state.change({ endpoint: 'http://127.0.0.1:11436' });
  const second = state.loadModels((endpoint) => {
    assert.equal(endpoint, 'http://127.0.0.1:11436');
    return fresh.promise;
  });
  old.resolve(catalog([installed('stale-model')]));
  await first;
  assert.equal(state.getSnapshot().catalog, null);
  assert.equal(state.getSnapshot().listing, true);
  fresh.resolve(catalog([installed('fresh-model')], 'http://127.0.0.1:11436/'));
  await second;
  assert.equal(state.getSnapshot().catalog.models[0].name, 'fresh-model');
  assert.equal(state.getSnapshot().listing, false);
  assert.equal(state.selectModel('stale-model', 'b'.repeat(64)), false);
});

test('catalog refresh invalidates verification and failure cannot leave stale selectable models', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings());
  await state.loadModels(async () => catalog());
  state.selectModel(installed().name, installed().digest);
  await state.verify(async () => {});
  assert.equal(state.canSave, true);
  const stale = deferred();
  const loading = state.loadModels(() => stale.promise);
  assert.equal(state.canSave, false);
  await state.verify(() => assert.fail('do not verify while the model catalog is changing'));
  stale.reject(Error('service offline'));
  await loading;
  assert.equal(state.getSnapshot().catalog, null);
  assert.equal(state.getSnapshot().catalogError.message, 'service offline');
  assert.equal(state.getSnapshot().draft.model, installed().name);
  assert.equal(state.selectModel(installed().name, installed().digest), false);
  await state.loadModels(async () => catalog([]));
  assert.deepEqual(state.getSnapshot().catalog.models, []);
  assert.equal(state.getSnapshot().catalogError, null);
});

test('configuration replacement cancels old catalog results and disabling does not reuse another endpoint catalog', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings());
  const old = deferred();
  const loading = state.loadModels(() => old.promise);
  await state.load(async () => settings('reloaded', 2));
  await state.loadModels(async () => catalog());
  old.reject(Error('obsolete request'));
  await loading;
  assert.equal(state.getSnapshot().catalogError, null);
  assert.equal(state.getSnapshot().catalog.models[0].name, installed().name);
  await state.save(null, async () => ({ row_version: 3, profile: null }));
  assert.equal(state.getSnapshot().draft.endpoint, '');
  assert.equal(state.getSnapshot().draft.provider, 'openai_chat');
  assert.equal(state.getSnapshot().catalog, null);
});

test('a disposed page cannot start a follow-up catalog request, while an explicit remount load can resume', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings());
  state.invalidate();
  await state.loadModels(() => assert.fail('unmounted page must not start another read'));
  state.change({ model: 'stale-event' });
  assert.equal(state.getSnapshot().draft.model, 'saved-model');
  await state.load(async () => settings('remounted'));
  await state.loadModels(async () => catalog());
  assert.equal(state.getSnapshot().catalog.models.length, 1);
  assert.equal(state.locked, false);
});

test('confirming an index operation preserves the unsaved model draft; only settings confirmation reloads it', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('saved'));
  state.change({ model: 'unsaved-selection' });
  for (const kind of [
    'embedding_profile_register',
    'index_rebuild_start',
    'index_rebuild_cancel',
    undefined,
  ]) {
    await state.reloadAfterConfirmation(kind, () =>
      assert.fail('an unrelated receipt must not reload model settings'),
    );
    assert.equal(state.getSnapshot().draft.model, 'unsaved-selection');
  }
  await state.reloadAfterConfirmation('generation_settings_set', async () =>
    settings('confirmed', 8),
  );
  assert.equal(state.getSnapshot().draft.model, 'confirmed');
  assert.equal(state.getSnapshot().settings.row_version, 8);
});

test('loading configuration locks editing, validation and persistence until the actual settings arrive', async () => {
  const state = new ModelDraft();
  const read = deferred();
  const loading = state.load(() => read.promise);
  assert.equal(state.locked, true);
  state.change({ model: 'typed-before-read' });
  await state.verify(() => assert.fail('must not validate an unloaded draft'));
  await state.save(null, () => assert.fail('must not save before reading the base revision'));
  assert.equal(state.getSnapshot().draft.model, '');
  read.resolve(settings());
  await loading;
  assert.equal(state.locked, false);
  assert.equal(state.getSnapshot().draft.endpoint, 'http://127.0.0.1:11435');
  state.change({ model: 'user-selection' });
  assert.equal(state.getSnapshot().draft.model, 'user-selection');
});

test('read failure leaves the form locked and retry loads only the newest request', async () => {
  const state = new ModelDraft();
  await state.load(async () => {
    throw Error('read failed');
  });
  assert.equal(state.locked, true);
  assert.equal(state.getSnapshot().loading, false);
  assert.equal(state.getSnapshot().error.message, 'read failed');
  const old = deferred();
  const loading = state.load(() => old.promise);
  await state.load(async () => settings('current', 4));
  state.change({ model: 'edited-current' });
  old.resolve(settings('outdated', 1));
  await loading;
  assert.equal(state.getSnapshot().draft.model, 'edited-current');
  assert.equal(state.getSnapshot().settings.row_version, 4);
  assert.equal(state.getSnapshot().error, null);
});

test('a late verification cannot verify an edited draft or clear the newer check', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('A'));
  const old = deferred(),
    current = deferred();
  const checkingOld = state.verify(() => old.promise);
  state.change({ model: 'B' });
  const checkingCurrent = state.verify(() => current.promise);
  old.resolve();
  await checkingOld;
  assert.equal(state.getSnapshot().checking, true);
  assert.equal(state.canSave, false);
  current.resolve();
  await checkingCurrent;
  assert.equal(state.canSave, true);
  assert.equal(JSON.parse(state.getSnapshot().verified).model, 'B');
});

test('changing away and back does not revive an obsolete verification', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('A'));
  const old = deferred();
  const checking = state.verify(() => old.promise);
  state.change({ model: 'B' });
  state.change({ model: 'A' });
  old.resolve();
  await checking;
  assert.equal(state.canSave, false);
});

test('reload and scope disposal suppress both delayed success and delayed verification errors', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('old'));
  const old = deferred();
  const checking = state.verify(() => old.promise);
  await state.load(async () => settings('reloaded', 2));
  old.reject(Error('old unsupported profile'));
  await checking;
  assert.equal(state.getSnapshot().error, null);
  assert.equal(state.getSnapshot().draft.model, 'reloaded');
  const pending = deferred();
  const reading = state.load(() => pending.promise);
  state.invalidate();
  const before = state.getSnapshot();
  pending.resolve(settings('other-scope'));
  await reading;
  assert.equal(state.getSnapshot(), before);
});

test('saving captures the verified draft and base revision and prevents intervening edits or reloads', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('A', 7));
  await state.verify(async () => {});
  const pending = deferred();
  let sent;
  const saving = state.save(state.getSnapshot().draft, (request) => {
    sent = request;
    return pending.promise;
  });
  assert.equal(state.locked, true);
  state.change({ model: 'must-not-overwrite' });
  await state.load(() => assert.fail('reload must not race a settings mutation'));
  await state.verify(() => assert.fail('verification must not race a settings mutation'));
  assert.equal(sent.base_row_version, 7);
  assert.equal(sent.profile.model, 'A');
  pending.resolve(settings('A', 8));
  await saving;
  assert.equal(state.locked, false);
  assert.equal(state.getSnapshot().settings.row_version, 8);
  assert.equal(state.getSnapshot().draft.model, 'A');
  assert.equal(state.canSave, false);
});

test('unverified or changed configurations cannot be saved through a stale handler', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings('A'));
  await state.save(state.getSnapshot().draft, () => assert.fail('unverified write'));
  await state.verify(async () => {});
  await state.save({ ...state.getSnapshot().draft, model: 'B' }, () =>
    assert.fail('mismatched write'),
  );
  const disabled = settings();
  disabled.profile = null;
  await state.save(null, async (request) => {
    assert.equal(request.profile, null);
    return disabled;
  });
  assert.equal(state.getSnapshot().settings.profile, null);
});

const remoteSettings = (provider = 'openai_chat') => ({
  row_version: 3,
  profile: {
    ...settings().profile,
    provider,
    endpoint: 'https://api.example.test/custom/v1',
    model: 'remote-model',
    model_digest: '',
  },
});

test('all remote protocols verify without a local digest or catalog and retain the explicit API prefix', async () => {
  for (const provider of ['openai_chat', 'openai_responses', 'anthropic']) {
    const state = new ModelDraft();
    await state.load(async () => remoteSettings(provider));
    await state.loadModels(() => assert.fail('remote settings must never enumerate Ollama'));
    await state.verify(() => assert.fail('remote verification requires a configured credential'));
    await state.credentialOperation(async () => true);
    await state.verify(async (profile) => {
      assert.equal(profile.provider, provider);
      assert.equal(profile.endpoint, 'https://api.example.test/custom/v1');
      assert.equal(profile.model_digest, '');
    });
    assert.equal(state.canSave, true);
    await state.save(state.getSnapshot().draft, async (request) => {
      assert.equal(request.base_row_version, 3);
      assert.equal('api_key' in request.profile, false);
      return { row_version: 4, profile: request.profile };
    });
    assert.equal(state.getSnapshot().settings.row_version, 4);
  }
});

test('protocol switches reject late catalogs and credential status from the old configuration', async () => {
  const state = new ModelDraft();
  await state.load(async () => settings());
  const listing = deferred();
  const pendingList = state.loadModels(() => listing.promise);
  state.change(remoteSettings().profile);
  const old = deferred(),
    fresh = deferred();
  const pendingOld = state.credentialOperation(() => old.promise);
  state.change({ provider: 'anthropic' });
  const pendingFresh = state.credentialOperation(() => fresh.promise);
  listing.resolve(catalog());
  old.resolve(true);
  await Promise.all([pendingList, pendingOld]);
  assert.equal(state.getSnapshot().catalog, null);
  assert.equal(state.getSnapshot().credentialConfigured, null);
  assert.equal(state.getSnapshot().credentialBusy, true);
  fresh.resolve(false);
  await pendingFresh;
  assert.equal(state.getSnapshot().credentialConfigured, false);
  assert.equal(state.getSnapshot().credentialBusy, false);
});

test('changing a credential invalidates verification, including a still-running test', async () => {
  const state = new ModelDraft();
  await state.load(async () => remoteSettings());
  await state.credentialOperation(async () => true);
  const pending = deferred();
  const verifying = state.verify(() => pending.promise);
  await state.credentialOperation(async () => false);
  pending.resolve();
  await verifying;
  assert.equal(state.canSave, false);
  assert.equal(state.getSnapshot().verified, null);
  await state.credentialOperation(async () => true);
  await state.verify(async () => {});
  assert.equal(state.canSave, true);
  await state.credentialOperation(async () => true);
  assert.equal(state.canSave, false);
});

test('credential errors cannot expose request secrets and stale errors cannot contaminate another model', async () => {
  const state = new ModelDraft();
  await state.load(async () => remoteSettings());
  await state.credentialOperation(async () => {
    throw Error('secret-fixture');
  });
  assert.equal(state.getSnapshot().credentialError, 'credential_operation_failed');
  assert.equal(JSON.stringify(state.getSnapshot()).includes('secret-fixture'), false);
  const old = deferred();
  const pending = state.credentialOperation(() => old.promise);
  state.change({ model: 'new-model' });
  await state.credentialOperation(async () => true);
  old.reject(Error('obsolete secret-fixture'));
  await pending;
  assert.equal(state.getSnapshot().credentialConfigured, true);
  assert.equal(state.getSnapshot().credentialError, null);
});

test('credential results cannot survive leaving and returning to a profile or disposing the page', async () => {
  const state = new ModelDraft();
  await state.load(async () => remoteSettings());
  const old = deferred();
  const pending = state.credentialOperation(() => old.promise);
  state.change({ endpoint: 'https://other.example.test' });
  state.change({ endpoint: remoteSettings().profile.endpoint });
  old.resolve(true);
  await pending;
  assert.equal(state.getSnapshot().credentialConfigured, null);
  const disposed = deferred();
  const pendingDisposed = state.credentialOperation(() => disposed.promise);
  state.invalidate();
  const before = state.getSnapshot();
  disposed.resolve(true);
  await pendingDisposed;
  assert.equal(state.getSnapshot(), before);
});

test('credential failures retain only supported reason codes, never transport messages', async () => {
  const state = new ModelDraft();
  await state.load(async () => remoteSettings());
  const error = Object.assign(Error('secret-fixture'), {
    details: { details: { kind: 'inference_failure', reason: 'model_credential_unavailable' } },
  });
  await state.credentialOperation(async () => {
    throw error;
  });
  assert.equal(state.getSnapshot().credentialError, 'model_credential_unavailable');
  assert.equal(JSON.stringify(state.getSnapshot()).includes('secret-fixture'), false);
});

test('Chat output budget edits invalidate verification while preserving credential status and API identity', async () => {
  const state = new ModelDraft();
  await state.load(async () => remoteSettings());
  assert.equal('chat_token_limit' in state.getSnapshot().draft, false);
  await state.credentialOperation(async () => true);
  const pending = deferred();
  const checking = state.verify(() => pending.promise);
  state.change({ chat_token_limit: 'max_tokens' });
  pending.resolve();
  await checking;
  assert.equal(state.canSave, false);
  assert.equal(state.getSnapshot().credentialConfigured, true);
  assert.equal(state.getSnapshot().draft.endpoint, remoteSettings().profile.endpoint);
  await state.verify(async (profile) => assert.equal(profile.chat_token_limit, 'max_tokens'));
  assert.equal(state.canSave, true);
  state.change({ chat_token_limit: undefined });
  assert.equal(state.canSave, false);
  assert.equal('chat_token_limit' in state.getSnapshot().draft, false);
  assert.equal(JSON.stringify(state.getSnapshot().draft).includes('chat_token_limit'), false);
});

test('leaving Chat clears its output budget field and returning does not revive it', async () => {
  for (const provider of ['openai_responses', 'anthropic', 'ollama']) {
    const state = new ModelDraft();
    await state.load(async () => remoteSettings());
    state.change({ chat_token_limit: 'max_tokens' });
    state.change({ provider });
    assert.equal('chat_token_limit' in state.getSnapshot().draft, false);
    state.change({ provider: 'openai_chat' });
    assert.equal('chat_token_limit' in state.getSnapshot().draft, false);
    assert.equal(state.canSave, false);
  }
});
