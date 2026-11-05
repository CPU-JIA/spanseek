import { Select } from '../ui/Select';
import { SectionSwitch } from '../ui/SectionSwitch';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { RefreshCw } from 'lucide-react';
import type {
  ChunkSettings,
  EmbeddingProfile,
  EmbeddingProvider,
  GenerationProfile,
  GenerationSettings,
  IndexStatus,
  RebuildJob,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { emptyScope } from './SearchPage';
import { useInferenceAction } from './useInferenceAction';
import { inferenceErrorMessage } from './inference-errors';
import { latestRebuild } from './inference-state';
import { ModelDraft } from './model-draft';
import {
  DIMENSION_CHOICES,
  EMBEDDING_PROVIDERS,
  modelOption,
  plainHttpAllowed,
  providerLabel,
  providerOption,
} from './embedding-providers';
import { GenerationChannelsPanel } from './GenerationChannelsPanel';
import { RetrievalConsentCard } from './retrieval-consent';
import './inference.css';

export function ModelsPage({
  identity,
  embedded = false,
}: {
  identity: WorkspaceIdentity;
  embedded?: boolean;
}) {
  const { t, language } = useI18n();
  const [section, setSection] = useState<'channels' | 'local' | 'embedding' | 'privacy'>(
    'channels',
  );
  const configuration = useMemo(
    () => new ModelDraft(),
    [identity.local_instance_id, identity.workspace_id],
  );
  const model = useSyncExternalStore(configuration.subscribe, configuration.getSnapshot);
  const { settings, draft, checking, verified } = model;
  const [index, setIndex] = useState<IndexStatus | null>(null);
  const [jobs, setJobs] = useState<RebuildJob[]>([]);
  const [more, setMore] = useState(false);
  const [remoteState, setRemoteState] = useState<{ semantic: boolean; consent: boolean } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [channelChange, setChannelChange] = useState(0);
  const [apiKey, setApiKey] = useState('');
  const [embeddingEndpoint, setEmbeddingEndpoint] = useState('');
  const [embeddingKey, setEmbeddingKey] = useState('');
  const [embeddingBusy, setEmbeddingBusy] = useState(false);
  const embeddingLock = useRef(false);
  const [embeddingCredential, setEmbeddingCredential] = useState<boolean | null>(null);
  const [embeddingStatusError, setEmbeddingStatusError] = useState<string | null>(null);
  const [embeddingProvider, setEmbeddingProvider] = useState<EmbeddingProvider>('cloudflare');
  const [embeddingModel, setEmbeddingModel] = useState('@cf/baai/bge-m3');
  const [embeddingOutputDimensions, setEmbeddingOutputDimensions] = useState<number | null>(null);
  const embeddingPreset = providerOption(embeddingProvider);
  const embeddingModelPreset = modelOption(embeddingProvider, embeddingModel);
  const [chunkSettings, setChunkSettings] = useState<ChunkSettings | null>(null);
  const [chunkTokens, setChunkTokens] = useState(512);
  const [overlapPercent, setOverlapPercent] = useState(15);
  const [chunkBusy, setChunkBusy] = useState(false);
  const [chunkMessage, setChunkMessage] = useState<string | null>(null);
  const [previousProfile, setPreviousProfile] = useState<EmbeddingProfile | null>(null);
  useEffect(() => {
    setEmbeddingEndpoint('');
    setEmbeddingKey('');
    setEmbeddingCredential(null);
    setEmbeddingProvider('cloudflare');
    setEmbeddingModel('@cf/baai/bge-m3');
    setEmbeddingOutputDimensions(null);
  }, [identity.local_instance_id, identity.workspace_id]);
  // The registered profile is the truth about what this workspace calls, so the
  // form starts from it instead of from a hard-coded provider.
  const activeProfile = index?.profile ?? null;
  const appliedProfile = useRef<string | null>(null);
  useEffect(() => {
    if (!activeProfile || appliedProfile.current === activeProfile.id) return;
    appliedProfile.current = activeProfile.id;
    setEmbeddingProvider(activeProfile.provider ?? 'cloudflare');
    setEmbeddingModel(activeProfile.model);
    setEmbeddingOutputDimensions(activeProfile.output_dimensions ?? null);
    if (activeProfile.endpoint) setEmbeddingEndpoint(activeProfile.endpoint);
  }, [activeProfile]);
  useEffect(() => {
    let current = true;
    setEmbeddingCredential(null);
    setEmbeddingStatusError(null);
    if (!embeddingEndpoint || !embeddingModel) return;
    const timer = setTimeout(() => {
      void call(
        {
          kind: 'embedding_credential_status',
          payload: {
            provider: embeddingProvider,
            endpoint: embeddingEndpoint,
            model: embeddingModel,
          },
        },
        'generation_credential_status',
        identity,
      )
        .then((value) => {
          if (current) setEmbeddingCredential(value.configured);
        })
        .catch((error) => {
          if (current) setEmbeddingStatusError(message(error));
        });
    }, 250);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [
    embeddingEndpoint,
    embeddingProvider,
    embeddingModel,
    identity.local_instance_id,
    identity.workspace_id,
    reload,
  ]);

  const remote = draft.provider !== 'ollama';
  useEffect(() => {
    setApiKey('');
    if (model.loaded && remote && !draft.channel_id && draft.endpoint && draft.model) {
      void configuration.credentialOperation(async (profile) => {
        const status = await call(
          { kind: 'generation_credential_status', payload: { profile } },
          'generation_credential_status',
          identity,
        );
        return status.configured;
      });
    }
  }, [
    configuration,
    model.loaded,
    draft.provider,
    draft.endpoint,
    draft.model,
    draft.channel_id,
    channelChange,
  ]);
  function channelChanged(settings: GenerationSettings) {
    if (configuration.acceptChannelSettings(settings)) {
      setChannelChange((value) => value + 1);
      if (configuration.getSnapshot().draft.provider === 'ollama')
        void configuration.loadModels(readModels);
    }
  }
  async function changeCredential(remove: boolean) {
    if (configuration.locked || actions.locked || model.credentialBusy) return;
    const secret = apiKey;
    setApiKey('');
    await configuration.credentialOperation(async (profile) => {
      if (remove) {
        await call(
          { kind: 'generation_credential_delete', payload: { profile } },
          'done',
          identity,
        );
      } else {
        await call(
          { kind: 'generation_credential_set', payload: { profile, api_key: secret } },
          'done',
          identity,
        );
      }
      return !remove;
    });
  }
  const actions = useInferenceAction(identity, 'models');
  const live = useRef(true);
  const readModels = (endpoint: string) =>
    call({ kind: 'generation_models_list', payload: { endpoint } }, 'generation_models', identity);
  async function readConfiguration() {
    const loaded = await configuration.load(() =>
      call({ kind: 'generation_settings_get' }, 'generation_settings', identity),
    );
    if (loaded) void configuration.loadModels(readModels);
  }
  useEffect(() => {
    void readConfiguration();
    return () => configuration.invalidate();
  }, [configuration]);
  function reloadAll() {
    if (actions.locked || model.saving || model.loading) return;
    void readConfiguration();
    setReload((value) => value + 1);
  }
  useEffect(() => {
    live.current = true;
    let current = true;
    Promise.all([
      call({ kind: 'index_status' }, 'index_status', identity),
      call(
        { kind: 'index_rebuild_list', payload: { after_id: null, limit: 30 } },
        'index_rebuilds',
        identity,
      ),
    ])
      .then(([status, rebuilds]) => {
        if (!current) return;
        setIndex(status);
        setEmbeddingEndpoint((value) => value || status.profile?.endpoint || '');
        setJobs(rebuilds);
        setMore(rebuilds.length === 30);
        setError(null);
      })
      .catch((e) => {
        if (current) setError(message(e));
      });
    return () => {
      current = false;
      live.current = false;
    };
  }, [reload]);
  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const active = jobs.filter((job) => job.status === 'queued' || job.status === 'running');
        if (active.length || index?.profile?.endpoint) {
          const [status, ...updates] = await Promise.all([
            call({ kind: 'index_status' }, 'index_status', identity),
            ...active.map((job) =>
              call(
                { kind: 'index_rebuild_get', payload: { job_id: job.id } },
                'index_rebuild',
                identity,
              ),
            ),
          ]);
          if (!current) return;
          setIndex(status as IndexStatus);
          setJobs((old) =>
            old.map((job) =>
              latestRebuild(
                job,
                (updates as RebuildJob[]).find((update) => update.id === job.id) ?? job,
              ),
            ),
          );
        }
      } catch (e) {
        if (current) setError(message(e));
      }
      if (current) timer = setTimeout(() => void poll(), 2000);
    }
    timer = setTimeout(() => void poll(), 2000);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [jobs, index?.profile?.id]);
  async function verify() {
    if (actions.locked) return;
    await configuration.verify((profile) =>
      call({ kind: 'generation_model_verify', payload: { profile } }, 'done', identity),
    );
  }
  async function save(profile: GenerationProfile | null) {
    if (actions.locked) return;
    await configuration.save(profile, async (request) => {
      const result = await actions.execute(
        {
          kind: 'generation_settings_set',
          payload: {
            command_id: crypto.randomUUID(),
            request,
          },
        },
        'generation_settings_mutation',
      );
      return result?.settings ?? null;
    });
  }
  useEffect(() => {
    let live = true;
    void call({ kind: 'chunk_settings_get' }, 'chunk_settings', identity)
      .then((value) => {
        if (!live) return;
        setChunkSettings(value);
        setChunkTokens(value.params.chunk_tokens);
        setOverlapPercent(value.params.overlap_percent);
      })
      .catch(() => {
        if (live) setChunkSettings(null);
      });
    void call({ kind: 'embedding_profile_previous' }, 'embedding_profile', identity)
      .then((value) => {
        if (live) setPreviousProfile(value);
      })
      .catch(() => {
        if (live) setPreviousProfile(null);
      });
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, reload]);
  async function saveChunking() {
    if (!chunkSettings || actions.locked || chunkBusy) return;
    setChunkBusy(true);
    setChunkMessage(null);
    try {
      const saved = await call(
        {
          kind: 'chunk_settings_set',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              base_row_version: chunkSettings.row_version,
              params: { chunk_tokens: chunkTokens, overlap_percent: overlapPercent },
            },
          },
        },
        'chunk_settings_mutation',
        identity,
      );
      setChunkSettings(saved.settings);
      setChunkTokens(saved.settings.params.chunk_tokens);
      setOverlapPercent(saved.settings.params.overlap_percent);
      // A new cap only takes effect once the profile is re-registered, which is
      // what rebuilds the index; the previous generation stays for rollback.
      const profile = index?.profile;
      if (profile?.endpoint) {
        await actions.execute(
          {
            kind: 'embedding_remote_configure',
            payload: {
              command_id: crypto.randomUUID(),
              provider: profile.provider ?? 'cloudflare',
              endpoint: profile.endpoint,
              model: profile.model,
              output_dimensions: profile.output_dimensions ?? null,
            },
          },
          'embedding_profile_mutation',
        );
        setChunkMessage(
          t(
            '已保存并按新参数重建索引；旧索引保留，可回退。',
            'Saved and rebuilding the index with the new values; the previous index is kept for rollback.',
          ),
        );
      } else {
        setChunkMessage(
          t(
            '已保存；配置向量服务后按此参数建立索引。',
            'Saved; the index will use these values once embeddings are configured.',
          ),
        );
      }
      setReload((value) => value + 1);
    } catch (failure) {
      setChunkMessage(inferenceErrorMessage(failure, t));
    } finally {
      setChunkBusy(false);
    }
  }
  async function revertIndex() {
    if (actions.locked || chunkBusy) return;
    setChunkBusy(true);
    setChunkMessage(null);
    try {
      await call(
        { kind: 'embedding_profile_revert', payload: { command_id: crypto.randomUUID() } },
        'embedding_profile_mutation',
        identity,
      );
      setChunkMessage(t('已回退到上一套索引。', 'Rolled back to the previous index.'));
      setReload((value) => value + 1);
    } catch (failure) {
      setChunkMessage(inferenceErrorMessage(failure, t));
    } finally {
      setChunkBusy(false);
    }
  }
  async function register() {
    if (actions.locked || embeddingLock.current) return;
    embeddingLock.current = true;
    setEmbeddingBusy(true);
    setEmbeddingStatusError(null);
    const secret = embeddingKey;
    const endpoint = embeddingEndpoint;
    const provider = embeddingProvider;
    const model = embeddingModel;
    const outputDimensions = embeddingOutputDimensions;
    setEmbeddingKey('');
    try {
      if (secret)
        await call(
          {
            kind: 'embedding_credential_set',
            payload: { provider, endpoint, model, api_key: secret },
          },
          'done',
          identity,
        );
      // Only the credential-free configuration can enter durable intent storage.
      const result = await actions.execute(
        {
          kind: 'embedding_remote_configure',
          payload: {
            command_id: crypto.randomUUID(),
            provider,
            endpoint,
            model,
            output_dimensions: outputDimensions,
          },
        },
        'embedding_profile_mutation',
      );
      if (result && live.current) setReload((value) => value + 1);
    } catch (error) {
      if (live.current) setEmbeddingStatusError(inferenceErrorMessage(error, t));
    } finally {
      embeddingLock.current = false;
      if (live.current) setEmbeddingBusy(false);
    }
  }
  async function useLocalEmbedding() {
    if (actions.locked || embeddingLock.current) return;
    embeddingLock.current = true;
    setEmbeddingBusy(true);
    setEmbeddingStatusError(null);
    try {
      const result = await actions.execute(
        {
          kind: 'embedding_profile_register',
          payload: { command_id: crypto.randomUUID() },
        },
        'embedding_profile_mutation',
      );
      if (result && live.current) setReload((value) => value + 1);
    } catch (error) {
      if (live.current) setEmbeddingStatusError(inferenceErrorMessage(error, t));
    } finally {
      embeddingLock.current = false;
      if (live.current) setEmbeddingBusy(false);
    }
  }

  async function removeEmbeddingCredential() {
    if (actions.locked || embeddingLock.current) return;
    embeddingLock.current = true;
    setEmbeddingBusy(true);
    setEmbeddingKey('');
    try {
      await call(
        {
          kind: 'embedding_credential_delete',
          payload: {
            provider: embeddingProvider,
            endpoint: embeddingEndpoint,
            model: embeddingModel,
          },
        },
        'done',
        identity,
      );
      if (live.current) setReload((value) => value + 1);
    } catch (error) {
      if (live.current) setEmbeddingStatusError(inferenceErrorMessage(error, t));
    } finally {
      embeddingLock.current = false;
      if (live.current) setEmbeddingBusy(false);
    }
  }
  async function rebuild() {
    if (!index?.profile) return;
    const result = await actions.execute(
      {
        kind: 'index_rebuild_start',
        payload: {
          command_id: crypto.randomUUID(),
          request: { profile_id: index.profile.id, scope: emptyScope() },
        },
      },
      'index_rebuild_mutation',
    );
    if (result && live.current)
      setJobs((old) => [result.job, ...old.filter((job) => job.id !== result.job.id)]);
  }
  async function cancel(job: RebuildJob) {
    const result = await actions.execute(
      {
        kind: 'index_rebuild_cancel',
        payload: { command_id: crypto.randomUUID(), job_id: job.id },
      },
      'index_rebuild_mutation',
    );
    if (result && live.current)
      setJobs((old) => old.map((value) => (value.id === job.id ? result.job : value)));
  }
  async function older() {
    try {
      const page = await call(
        { kind: 'index_rebuild_list', payload: { after_id: jobs.at(-1)?.id ?? null, limit: 30 } },
        'index_rebuilds',
        identity,
      );
      if (live.current) {
        setJobs((old) => [
          ...old,
          ...page.filter((item) => !old.some((job) => job.id === item.id)),
        ]);
        setMore(page.length === 30);
      }
    } catch (e) {
      if (live.current) setError(message(e));
    }
  }
  const change = (patch: Partial<GenerationProfile>) => {
    if (!actions.locked) {
      if (patch.provider !== undefined || patch.endpoint !== undefined || patch.model !== undefined)
        setApiKey('');
      configuration.change(patch);
    }
  };
  const formLocked = configuration.locked || actions.locked;
  const modelError = model.error ? inferenceErrorMessage(model.error, t) : null;
  const selectedModel = configuration.selectedModel;
  const selection = selectedModel ? JSON.stringify([selectedModel.name, selectedModel.digest]) : '';
  const supportReason = (reason: string | null) =>
    (
      ({
        model_profile_unsupported: t('当前版本不支持此配置', 'Unsupported configuration'),
        model_not_local: t('并非本机模型', 'Not a local model'),
        model_unavailable: t('模型不可用', 'Model unavailable'),
        model_response_invalid: t('模型信息无效', 'Invalid model information'),
      }) as Record<string, string>
    )[reason ?? ''] ?? t('未通过支持检查', 'Support check did not pass');
  const refreshControl = (
    <button
      type="button"
      className="secondary-button"
      aria-label={t('刷新配置', 'Refresh configuration')}
      title={t('刷新配置', 'Refresh configuration')}
      disabled={model.loading || model.saving || actions.locked}
      onClick={reloadAll}
    >
      <RefreshCw size={15} aria-hidden="true" />
      {!embedded && t('刷新配置', 'Refresh')}
    </button>
  );
  return (
    <PageFrame
      className="inference-page models-page"
      title={t('模型与连接', 'Models & connections')}
      titleHidden={embedded}
      description={t(
        '语义检索与答案生成分别配置。未配置生成模型时，笔记与关键词检索仍可使用。',
        'Semantic search and answer generation are configured separately. Notes and keyword search remain available without a generation model.',
      )}
      actions={!embedded ? refreshControl : undefined}
      toolbar={
        <div className="models-section-toolbar">
          <SectionSwitch
            label={t('模型配置分类', 'Model settings sections')}
            value={section}
            onChange={setSection}
            options={[
              { value: 'channels', label: t('AI 渠道', 'AI channels') },
              { value: 'local', label: t('本机与独立模型', 'Local & standalone') },
              { value: 'embedding', label: t('语义检索', 'Semantic search') },
              { value: 'privacy', label: t('数据与费用', 'Data & costs') },
            ]}
          />
          {embedded && refreshControl}
        </div>
      }
    >
      {(modelError || error || actions.error) && (
        <div className="inference-error" role="alert">
          {modelError || error || actions.error}
        </div>
      )}
      {(!actions.ready || actions.pending) && (
        <div className="inference-notice">
          <p>
            {t(
              '有模型操作等待读取或确认，将重放原请求。',
              'A model operation awaits reading or confirmation; its original request will be replayed.',
            )}
          </p>
          <button
            disabled={actions.busy}
            onClick={async () => {
              const kind = actions.pending?.command.kind;
              const result = await actions.retry();
              if (result && live.current) {
                void configuration
                  .reloadAfterConfirmation(kind, () =>
                    call({ kind: 'generation_settings_get' }, 'generation_settings', identity),
                  )
                  .then((loaded) => {
                    if (loaded) void configuration.loadModels(readModels);
                  });
                setReload((value) => value + 1);
              }
            }}
          >
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      )}
      <div hidden={section !== 'privacy'}>
        <RetrievalConsentCard
          identity={identity}
          onStateChange={(params) =>
            setRemoteState({ semantic: params.semantic_enabled, consent: params.remote_consent })
          }
        />
      </div>
      <div hidden={section !== 'channels'}>
        <GenerationChannelsPanel
          identity={identity}
          settings={settings}
          disabled={model.saving || actions.locked || embeddingBusy}
          onChanged={channelChanged}
        />
      </div>
      <div hidden={section !== 'local'}>
        {draft.channel_id ? (
          <div className="inference-notice">
            <p>
              {t(
                '当前模型来自 AI 渠道。可在「AI 渠道」中管理，或在这里配置独立服务。',
                'The current model belongs to an AI channel. Manage it in AI channels, or configure a standalone service here.',
              )}
            </p>
            <button disabled={formLocked} onClick={() => configuration.beginLegacyConfiguration()}>
              {t('编辑单模型或本机配置', 'Edit a single-model or local configuration')}
            </button>
          </div>
        ) : (
          <section className="inference-card">
            <h2>{t('生成模型', 'Generation model')}</h2>
            <p>
              {model.loading
                ? t('正在读取模型配置…', 'Loading model configuration…')
                : !model.loaded
                  ? t(
                      '配置尚未读取。若读取失败，请重新读取。',
                      'Configuration has not been loaded. Reload if reading failed.',
                    )
                  : settings?.profile
                    ? t(
                        '已有保存配置；测试连接确认服务可用，不代表答案质量达标。',
                        'Configuration saved. Testing confirms service availability, not answer quality.',
                      )
                    : t(
                        '选择接口协议，填写服务地址与模型，再测试连接。',
                        'Choose a protocol, enter the service address and model, then test the connection.',
                      )}
            </p>
            <div className="choice-field inference-field">
              {t('接口协议', 'API protocol')}
              <Select
                aria-label={t('接口协议', 'API protocol')}
                value={draft.provider}
                disabled={formLocked}
                onValueChange={(nextValue) => {
                  const provider = nextValue;
                  change({
                    provider,
                    endpoint:
                      provider === 'ollama'
                        ? 'http://127.0.0.1:11434'
                        : remote
                          ? draft.endpoint
                          : '',
                    model: provider !== 'ollama' && remote ? draft.model : '',
                    model_digest: '',
                  });
                }}
              >
                <option value="openai_chat">OpenAI · Chat Completions</option>
                <option value="openai_responses">OpenAI · Responses</option>
                <option value="anthropic">Anthropic · Messages</option>
                <option value="ollama">Ollama · {t('本机', 'Local')}</option>
              </Select>
            </div>
            <div className="inference-controls">
              <label className="inference-field">
                {remote ? t('API 根地址', 'API base URL') : t('本机服务地址', 'Local endpoint')}
                <input
                  disabled={formLocked}
                  type="url"
                  placeholder={remote ? 'https://api.example.com/v1' : 'http://127.0.0.1:11434'}
                  autoComplete="off"
                  spellCheck={false}
                  value={draft.endpoint}
                  onChange={(e) => change({ endpoint: e.target.value })}
                />
              </label>
              {!remote && (
                <button
                  className="secondary-button"
                  disabled={formLocked || model.listing}
                  onClick={() => void configuration.loadModels(readModels)}
                >
                  {model.listing
                    ? t('正在读取模型…', 'Reading models…')
                    : t('重新读取本机模型', 'Refresh local models')}
                </button>
              )}
            </div>
            {remote ? (
              <>
                <p className="muted">
                  {t(
                    '填写服务商提供的 API 根地址，保留 /v1 等路径前缀；不会自动补上 /v1。',
                    'Use the provider’s API base URL, including path prefixes such as /v1. /v1 is not added automatically.',
                  )}
                </p>
                <label className="inference-field">
                  {t('模型名称', 'Model name')}
                  <input
                    value={draft.model}
                    disabled={formLocked}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="gpt-oss-120b"
                    onChange={(event) => change({ model: event.target.value, model_digest: '' })}
                  />
                </label>
                <label className="inference-field">
                  {t('API 密钥', 'API key')}
                  <input
                    type="password"
                    value={apiKey}
                    disabled={formLocked || model.credentialBusy}
                    autoComplete="new-password"
                    spellCheck={false}
                    placeholder={
                      model.credentialConfigured
                        ? t('已保存；输入新密钥以替换', 'Saved; enter a new key to replace it')
                        : t('输入 API 密钥', 'Enter API key')
                    }
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                </label>
                <p role="status" className="muted">
                  {model.credentialBusy
                    ? t('正在更新密钥状态…', 'Updating key status…')
                    : model.credentialConfigured === true
                      ? t('此配置已保存密钥。', 'A key is saved for this configuration.')
                      : model.credentialConfigured === false
                        ? t('此配置尚未保存密钥。', 'No key saved for this configuration.')
                        : t(
                            '填写地址与模型后读取密钥状态。',
                            'Enter an address and model to check key status.',
                          )}
                </p>
                {model.credentialError ? (
                  <p role="alert" className="inference-error">
                    {inferenceErrorMessage(model.credentialError, t)}
                  </p>
                ) : null}
                <div className="inference-controls">
                  <button
                    className="secondary-button"
                    disabled={
                      formLocked ||
                      model.credentialBusy ||
                      !draft.endpoint ||
                      !draft.model ||
                      !apiKey.trim()
                    }
                    onClick={() => void changeCredential(false)}
                  >
                    {t('保存密钥', 'Save key')}
                  </button>
                  <button
                    className="text-button"
                    disabled={formLocked || model.credentialBusy || !draft.endpoint || !draft.model}
                    onClick={() => void changeCredential(true)}
                  >
                    {t('移除密钥', 'Remove key')}
                  </button>
                </div>
              </>
            ) : (
              <>
                <div className="choice-field inference-field">
                  {t('已安装模型', 'Installed model')}
                  <Select
                    aria-label={t('已安装模型', 'Installed model')}
                    value={selection}
                    disabled={formLocked || model.listing || !model.catalog?.models.length}
                    onValueChange={(nextValue) => {
                      const item = model.catalog?.models.find(
                        (entry) => JSON.stringify([entry.name, entry.digest]) === nextValue,
                      );
                      if (item) configuration.selectModel(item.name, item.digest);
                    }}
                  >
                    <option value="" disabled>
                      {model.listing
                        ? t('正在读取…', 'Loading…')
                        : draft.model
                          ? t(
                              `${draft.model} · 请从当前服务重新选择`,
                              `${draft.model} · Select from the current service`,
                            )
                          : t('请选择本机已安装模型', 'Select an installed local model')}
                    </option>
                    {model.catalog?.models.map((item, index) => (
                      <option
                        key={`${item.name}/${item.digest}/${index}`}
                        value={JSON.stringify([item.name, item.digest])}
                        disabled={!item.supported || !item.profile}
                      >
                        {item.name}
                        {item.supported && item.profile
                          ? ` · ${(item.size / 1024 ** 3).toLocaleString(language, { maximumFractionDigits: 1 })} GB`
                          : ` · ${supportReason(item.reason)}`}
                      </option>
                    ))}
                  </Select>
                </div>
                {model.catalogError ? (
                  <p className="inference-error" role="alert">
                    {inferenceErrorMessage(model.catalogError, t)}
                  </p>
                ) : null}
                {!model.listing && model.catalog?.models.length === 0 ? (
                  <p role="status">
                    {t(
                      '此服务尚未安装本机模型。请先在 Ollama 中准备模型，再重新读取。',
                      'This service has no local models installed. Prepare a model in Ollama, then refresh.',
                    )}
                  </p>
                ) : null}
                {!model.listing &&
                model.catalog &&
                model.catalog.models.length > 0 &&
                !model.catalog.models.some((item) => item.supported && item.profile) ? (
                  <p role="status">
                    {t(
                      '已读到模型，但当前版本没有通过支持检查的可选配置。',
                      'Models were found, but none passed the support checks for this version.',
                    )}
                  </p>
                ) : null}
                {!model.listing && !model.catalog && !model.catalogError && model.loaded ? (
                  <p className="muted">
                    {t(
                      '请读取当前服务的本机模型后选择。',
                      'Read the current service’s local models to select one.',
                    )}
                  </p>
                ) : null}
                <details>
                  <summary>{t('模型标识与运行环境', 'Model identity and runtime')}</summary>
                  <p>
                    {draft.model || t('尚未选择模型', 'No model selected')}
                    {model.catalog ? ` · Ollama ${model.catalog.runtime_version}` : ''}
                  </p>
                  <label className="inference-field">
                    {t(
                      '本机模型 SHA-256 摘要（自动读取）',
                      'Local model SHA-256 digest (read automatically)',
                    )}
                    <input readOnly value={draft.model_digest} />
                  </label>
                </details>
              </>
            )}
            {draft.provider === 'openai_chat' && (
              <details>
                <summary>{t('高级接口设置', 'Advanced API settings')}</summary>
                <div className="choice-field inference-field">
                  {t('输出预算字段', 'Output budget field')}
                  <Select
                    aria-label={t('输出预算字段', 'Output budget field')}
                    value={draft.chat_token_limit ?? 'max_completion_tokens'}
                    disabled={formLocked}
                    onValueChange={(nextValue) =>
                      change({
                        chat_token_limit: nextValue === 'max_tokens' ? 'max_tokens' : undefined,
                      })
                    }
                  >
                    <option value="max_completion_tokens">
                      max_completion_tokens · {t('默认', 'Default')}
                    </option>
                    <option value="max_tokens">
                      max_tokens · {t('兼容接口', 'Compatible APIs')}
                    </option>
                  </Select>
                </div>
                <p className="muted">
                  {t(
                    '按服务商要求选择。OpenAI 默认使用 max_completion_tokens；DeepSeek Chat 使用 max_tokens。不会根据地址自动切换，修改后需重新测试连接。',
                    'Follow your provider’s requirements. OpenAI defaults to max_completion_tokens; DeepSeek Chat uses max_tokens. The address does not select this automatically. Test the connection again after changing it.',
                  )}
                </p>
              </details>
            )}
            <div className="inference-controls">
              <label>
                {t('上下文容量', 'Context capacity')}
                <input
                  type="number"
                  min={selectedModel?.context_min ?? 256}
                  max={remote ? 131072 : (selectedModel?.context_max ?? 32768)}
                  value={draft.context_tokens}
                  disabled={formLocked}
                  onChange={(e) => change({ context_tokens: Number(e.target.value) })}
                />
              </label>
              <label>
                {t('温度', 'Temperature')}
                <input
                  type="number"
                  min={0}
                  max={2}
                  step={0.1}
                  value={draft.temperature}
                  disabled={formLocked}
                  onChange={(e) => change({ temperature: Number(e.target.value) })}
                />
              </label>
            </div>
            {remote && draft.context_tokens <= 8192 && (
              <p role="status" className="muted">
                {t(
                  '当前上下文容量不足以使用自由对话默认的 8,192 token 输出预算。请按模型实际容量调整上下文，或在对话中明确降低输出预算；输入与历史也需要占用上下文。',
                  'This context capacity cannot fit the default 8,192-token chat output budget. Set the context to the model’s actual capacity, or explicitly lower the output budget in the conversation. Input and history also need context space.',
                )}
              </p>
            )}
            <p className="muted">
              {remote
                ? t(
                    '问题与所选证据会发送到此 API 服务。测试连接会发送一条简短请求，可能产生少量费用。',
                    'Questions and selected evidence are sent to this API service. Testing sends a short request and may incur a small charge.',
                  )
                : t(
                    '问题与所选证据只发送到此本机服务。验证会拒绝缺模型、摘要不匹配和不受支持的配置；不会自动下载。',
                    'Questions and selected evidence go only to this local service. Missing models, mismatched digests, and unsupported configurations are rejected; nothing is downloaded automatically.',
                  )}
            </p>
            {verified === JSON.stringify(draft) && (
              <p role="status">
                {t(
                  '连接测试通过，可以保存此配置。',
                  'Connection test passed. This configuration can be saved.',
                )}
              </p>
            )}
            <div className="inference-controls">
              <button
                className="secondary-button"
                disabled={
                  formLocked ||
                  model.listing ||
                  checking ||
                  (remote
                    ? model.credentialBusy || model.credentialConfigured !== true || !!apiKey
                    : !selectedModel?.supported || !selectedModel.profile)
                }
                onClick={() => void verify()}
              >
                {checking ? t('正在验证…', 'Verifying…') : t('测试连接', 'Test connection')}
              </button>
              <button
                className="primary-button"
                disabled={formLocked || !configuration.canSave || (remote && !!apiKey)}
                onClick={() => void save(draft)}
              >
                {t('保存已验证配置', 'Save verified configuration')}
              </button>
              <button
                className="text-button"
                disabled={formLocked || !settings?.profile}
                onClick={() => void save(null)}
              >
                {t('停用生成模型', 'Disable generation')}
              </button>
            </div>
          </section>
        )}
      </div>
      <section hidden={section !== 'embedding'} className="inference-card">
        <h2>{t('知识库向量检索', 'Knowledge base vector search')}</h2>
        <p>
          {providerLabel(embeddingProvider, language === 'en')} · {embeddingModel}
          {embeddingModelPreset && ` · ${embeddingModelPreset.dimensions} ${t('维', 'dimensions')}`}
          {embeddingModelPreset?.price &&
            ` · ${t(
              `官方参考价 ${embeddingModelPreset.price} / 百万输入 token`,
              `Official reference ${embeddingModelPreset.price} per 1M input tokens`,
            )}`}
        </p>
        <div className="muted">
          {t(
            '资料与问题使用同一个向量模型。启用后，参与检索的文本会发送至你配置的服务；向量和原文保存在本机。切换模型后需要重建索引。',
            'Documents and queries use the same embedding model. Enabled text is sent to your configured service; vectors and originals stay on this device. Rebuild after switching models.',
          )}
          {remoteState?.semantic === false && (
            <p className="inference-notice">
              {t(
                '本知识库已关闭语义检索。可保留配置，在「数据与费用」中重新启用。',
                'Semantic retrieval is off. Keep your configuration and enable it in Data & costs when needed.',
              )}
            </p>
          )}
          {remoteState?.consent === false && (
            <p className="inference-notice">
              {t(
                '尚未允许向远程服务发送文本。请在「数据与费用」中查看并确认。',
                'Remote processing has not been enabled. Review and confirm it in Data & costs.',
              )}
            </p>
          )}
          <button type="button" className="text-button" onClick={() => setSection('privacy')}>
            {t('数据与费用设置', 'Data & cost settings')}
          </button>
        </div>
        <div className="choice-field inference-field">
          {t('向量服务商', 'Embedding provider')}
          <Select
            aria-label={t('向量服务商', 'Embedding provider')}
            value={embeddingProvider}
            disabled={embeddingBusy || actions.locked}
            onValueChange={(next) => {
              const provider = next as EmbeddingProvider;
              const option = providerOption(provider);
              setEmbeddingProvider(provider);
              setEmbeddingModel(option.freeModel ? '' : (option.models[0]?.model ?? ''));
              setEmbeddingOutputDimensions(null);
              setEmbeddingEndpoint(option.endpoint);
              setEmbeddingKey('');
            }}
          >
            {EMBEDDING_PROVIDERS.map((option) => (
              <option key={option.id} value={option.id}>
                {providerLabel(option.id, language === 'en')}
              </option>
            ))}
          </Select>
        </div>
        <div className="choice-field inference-field">
          {t('向量模型', 'Embedding model')}
          {embeddingPreset.freeModel ? (
            <input
              aria-label={t('向量模型', 'Embedding model')}
              value={embeddingModel}
              disabled={embeddingBusy || actions.locked}
              placeholder="nomic-embed-text"
              onChange={(event) => {
                setEmbeddingModel(event.target.value);
                setEmbeddingOutputDimensions(null);
              }}
            />
          ) : (
            <Select
              aria-label={t('向量模型', 'Embedding model')}
              value={embeddingModel}
              disabled={embeddingBusy || actions.locked}
              onValueChange={(next) => {
                setEmbeddingModel(next);
                setEmbeddingOutputDimensions(null);
                setEmbeddingKey('');
              }}
            >
              {embeddingPreset.models.map((option) => (
                <option key={option.model} value={option.model} disabled={option.retired}>
                  {option.model}
                  {option.inputTokens ? ` · ${option.inputTokens} token` : ''}
                  {option.price ? ` · ${option.price}/M` : ''}
                  {option.retired ? ` · ${t('已停用', 'retired')}` : ''}
                </option>
              ))}
            </Select>
          )}
        </div>
        {embeddingModelPreset?.retired && (
          <p className="inference-error">
            {t(
              '该模型已被服务商停用（text-embedding-004 于 2026-01-14 停用），保存会失败。请改用 gemini-embedding-001 或 gemini-embedding-2。',
              'The provider shut this model down (text-embedding-004 was retired on 2026-01-14), so saving fails. Use gemini-embedding-001 or gemini-embedding-2 instead.',
            )}
          </p>
        )}
        {embeddingPreset.truncation && (
          <div className="choice-field inference-field">
            {t('输出维度', 'Output dimensions')}
            <Select
              aria-label={t('输出维度', 'Output dimensions')}
              value={
                embeddingOutputDimensions === null ? 'default' : String(embeddingOutputDimensions)
              }
              disabled={embeddingBusy || actions.locked}
              onValueChange={(next) =>
                setEmbeddingOutputDimensions(next === 'default' ? null : Number(next))
              }
            >
              <option value="default">{t('默认（模型原生维度）', 'Default (native size)')}</option>
              {DIMENSION_CHOICES.map((size) => (
                <option key={size} value={String(size)}>
                  {size}
                </option>
              ))}
            </Select>
          </div>
        )}
        <label className="inference-field">
          {t('向量 API 地址', 'Embedding API URL')}
          <input
            type="url"
            value={embeddingEndpoint}
            disabled={embeddingBusy || actions.locked}
            placeholder={embeddingPreset.endpointHint}
            onChange={(event) => {
              setEmbeddingEndpoint(event.target.value);
              setEmbeddingKey('');
            }}
          />
        </label>
        <label className="inference-field">
          {t('向量 API 令牌', 'Embedding API token')}
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={embeddingKey}
            disabled={embeddingBusy || actions.locked}
            placeholder={
              embeddingCredential
                ? t('已安全保存；留空保留', 'Stored securely; leave blank to keep')
                : t('输入令牌', 'Enter token')
            }
            onChange={(event) => setEmbeddingKey(event.target.value)}
          />
        </label>
        <p className="muted">
          {embeddingCredential === true
            ? t('令牌已保存到系统凭据库', 'Token stored in the system vault')
            : t('为当前知识库配置向量服务', 'Configure embeddings for this workspace')}
        </p>
        {embeddingStatusError && <p className="inference-error">{embeddingStatusError}</p>}
        {index?.profile && !index.profile.endpoint && (
          <p className="inference-notice">
            {t(
              '当前使用本机 E5 小型模型（离线）：文本不离开设备，首次加载与重建较慢；切换到远程服务后旧索引保留，可回退。',
              'The local E5 small model is active (offline): text never leaves this device, and the first load/rebuild is slower. Switching to a remote service keeps the old index for rollback.',
            )}
          </p>
        )}
        {index && (
          <p>
            {t(
              `${index.ready_objects} / ${index.eligible_objects} 个对象已有活动索引，${index.running_generations} 个代次处理中。`,
              `${index.ready_objects} / ${index.eligible_objects} objects have active indexes; ${index.running_generations} generations are processing.`,
            )}
          </p>
        )}
        {index?.profile && (
          <details>
            <summary>{t('当前向量配置', 'Current embedding profile')}</summary>
            <p>{index.profile.model}</p>
            <p>
              {index.profile.provider
                ? t(
                    `${providerLabel(index.profile.provider, false)} · ${index.profile.dimensions} 维`,
                    `${providerLabel(index.profile.provider, true)} · ${index.profile.dimensions} dimensions`,
                  )
                : t(
                    `本机 E5（离线） · ${index.profile.dimensions} 维`,
                    `Local E5 (offline) · ${index.profile.dimensions} dimensions`,
                  )}
            </p>
            <p>{index.profile.revision}</p>
            <p>{index.profile.tokenizer}</p>
            <p>
              {t(
                `${index.profile.max_tokens} token 上限 · 重叠 ${index.profile.overlap_tokens} token`,
                `${index.profile.max_tokens}-token cap · ${index.profile.overlap_tokens}-token overlap`,
              )}
            </p>
          </details>
        )}
        <details>
          <summary>
            {t('分块设置（保存后重建索引）', 'Chunking (saving rebuilds the index)')}
          </summary>
          <p className="muted">
            {t(
              '结构优先：段落与句子边界优先，默认 512 token 上限、约 15% 重叠。修改后需要重建索引；旧索引保留，可随时回退。',
              'Structure first: paragraph and sentence boundaries, a 512-token cap and about 15% overlap by default. Changing these rebuilds the index; the previous index is kept so you can roll back.',
            )}
          </p>
          <label className="inference-field">
            {t('分块上限（token）', 'Chunk cap (tokens)')}
            <input
              type="number"
              min={64}
              max={1024}
              step={64}
              value={chunkTokens}
              disabled={chunkBusy || actions.locked}
              onChange={(event) => setChunkTokens(Number(event.target.value))}
            />
          </label>
          <label className="inference-field">
            {t('重叠比例（%）', 'Overlap (%)')}
            <input
              type="number"
              min={0}
              max={30}
              step={5}
              value={overlapPercent}
              disabled={chunkBusy || actions.locked}
              onChange={(event) => setOverlapPercent(Number(event.target.value))}
            />
          </label>
          <div className="inference-controls">
            <button
              className="secondary-button"
              disabled={chunkBusy || actions.locked || !chunkSettings}
              onClick={() => void saveChunking()}
            >
              {chunkBusy ? t('正在保存…', 'Saving…') : t('保存并重建索引', 'Save and rebuild')}
            </button>
            {previousProfile && (
              <button
                className="secondary-button"
                disabled={chunkBusy || actions.locked}
                onClick={() => void revertIndex()}
              >
                {t('回退到上一套索引', 'Roll back to the previous index')}
              </button>
            )}
          </div>
          {previousProfile && (
            <p className="muted">
              {t(
                `可回退：${previousProfile.model} · ${previousProfile.max_tokens} token 上限`,
                `Available to roll back: ${previousProfile.model} · ${previousProfile.max_tokens}-token cap`,
              )}
            </p>
          )}
          {chunkMessage && (
            <p className="muted" role="status">
              {chunkMessage}
            </p>
          )}
        </details>
        <div className="inference-controls">
          <button
            className="secondary-button"
            disabled={embeddingBusy || actions.locked || remoteState?.semantic === false}
            onClick={() => void useLocalEmbedding()}
          >
            {t('使用本机 E5（离线）', 'Use local E5 (offline)')}
          </button>
          <button
            className="secondary-button"
            disabled={
              embeddingBusy ||
              actions.locked ||
              !index ||
              remoteState?.consent === false ||
              remoteState?.semantic === false ||
              !embeddingEndpoint ||
              !embeddingModel ||
              embeddingModelPreset?.retired === true ||
              !plainHttpAllowed(embeddingEndpoint) ||
              (!embeddingKey && embeddingCredential !== true)
            }
            onClick={() => void register()}
          >
            {t('保存并启用向量服务', 'Save and enable embeddings')}
          </button>
          <button
            className="text-button"
            disabled={embeddingBusy || actions.locked || !embeddingCredential}
            onClick={() => void removeEmbeddingCredential()}
          >
            {t('移除向量令牌', 'Remove embedding token')}
          </button>
          <button
            className="primary-button"
            disabled={
              embeddingBusy ||
              actions.locked ||
              !index?.profile?.endpoint ||
              embeddingEndpoint.replace(/\/+$/, '') !== index.profile.endpoint ||
              embeddingProvider !== (index.profile.provider ?? 'cloudflare') ||
              embeddingModel !== index.profile.model ||
              embeddingOutputDimensions !== (index.profile.output_dimensions ?? null) ||
              embeddingCredential !== true
            }
            onClick={() => void rebuild()}
          >
            {t('重建全库语义索引', 'Rebuild semantic index')}
          </button>
        </div>
        {jobs.map((job) => (
          <article className="inference-result" key={job.id}>
            <strong>
              {t('索引重建', 'Index rebuild')} ·{' '}
              {
                {
                  queued: t('等待处理', 'Queued'),
                  running: t('处理中', 'Running'),
                  completed: t('已完成', 'Completed'),
                  canceled: t('已取消', 'Canceled'),
                  failed: t('未完成', 'Failed'),
                }[job.status]
              }
            </strong>
            <p>
              {t(
                `已处理 ${job.processed} 项，跳过 ${job.skipped} 项`,
                `${job.processed} processed, ${job.skipped} skipped`,
              )}{' '}
              · {displayDate(job.updated_at, language)}
            </p>
            {job.error_code && (
              <p className="inference-error">{inferenceErrorMessage(job.error_code, t)}</p>
            )}
            {(job.status === 'queued' || job.status === 'running') && (
              <button
                className="secondary-button"
                disabled={embeddingBusy || actions.locked}
                onClick={() => void cancel(job)}
              >
                {t('取消此任务', 'Cancel task')}
              </button>
            )}
          </article>
        ))}
        {more && (
          <button className="text-button" onClick={() => void older()}>
            {t('加载更早重建任务', 'Older rebuild tasks')}
          </button>
        )}
      </section>
    </PageFrame>
  );
}
