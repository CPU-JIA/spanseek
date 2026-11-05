import { Select } from '../ui/Select';
import { Combobox } from '../ui/Combobox';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import {
  Check,
  ChevronRight,
  Cloud,
  KeyRound,
  Network,
  Plus,
  RefreshCw,
  Settings2,
  Trash2,
} from 'lucide-react';
import type {
  DiscoveredGenerationModel,
  GenerationChannelMutation,
  GenerationChannelSpec,
  GenerationSettings,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { GenerationChannelDraft } from './generation-channel-draft';
import { channelErrorMessage } from './channel-messages';
import {
  channelProtocolChange,
  channelRequestPreview,
  channelTemplates,
  cloudflareAccount,
  createChannelSpec,
  newChannelModel,
  protocolFamily,
  validateChannelSpec,
  type ChannelTemplate,
} from './generation-channel-presets';
import { useInferenceAction } from './useInferenceAction';
import './generation-channels.css';

type ChannelFocusContext = {
  controller: GenerationChannelDraft;
  epoch: number;
  anchor: Element | null;
};

export function GenerationChannelsPanel({
  identity,
  onChanged,
  disabled = false,
  settings,
}: {
  identity: WorkspaceIdentity;
  onChanged?: (settings: GenerationSettings) => void;
  disabled?: boolean;
  settings?: GenerationSettings | null;
}) {
  const { t } = useI18n();
  const controller = useMemo(
    () => new GenerationChannelDraft(),
    [identity.local_instance_id, identity.workspace_id],
  );
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const actions = useInferenceAction(identity, 'models/channels');
  const [adding, setAdding] = useState(false);
  const [confirming, setConfirming] = useState<'discard' | 'delete' | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);
  const [modelInputs, setModelInputs] = useState<Record<string, string>>({});
  const panel = useRef<HTMLElement>(null);
  const retryButton = useRef<HTMLButtonElement>(null);
  const confirmationTrigger = useRef<HTMLButtonElement | null>(null);
  const focusEpoch = useRef(0);
  const currentController = useRef(controller);
  currentController.current = controller;
  const [returnFocus, setReturnFocus] = useState<{
    context: ChannelFocusContext;
    preferred: HTMLElement | null;
    retry: boolean;
  } | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const focusNewChannel = useRef(false);
  const keyInput = useRef<HTMLInputElement>(null);
  const [hasKeyInput, setHasKeyInput] = useState(false);
  const changed = useRef(onChanged);
  changed.current = onChanged;
  const draft = controller.selected;
  const spec = draft?.spec;
  const saved = controller.savedChannel;
  const profile = state.server?.settings.profile;
  const blocked = disabled || controller.locked || actions.locked;
  const editingBlocked = disabled || !state.loaded || !!state.writing;
  const read = () => call({ kind: 'generation_channels' }, 'generation_channels', identity);

  useEffect(() => {
    setModelInputs({});
    focusNewChannel.current = false;
    void controller.load(read);
    return () => {
      ++focusEpoch.current;
      controller.invalidate();
    };
  }, [controller]);
  useEffect(() => {
    if (settings && state.loaded) controller.confirmSettings(settings);
  }, [controller, settings?.row_version, state.loaded]);
  useEffect(() => {
    setAdding(false);
    setConfirming(null);
    setLocalError(null);
  }, [controller, state.selectedId]);
  useEffect(() => {
    if (!adding && focusNewChannel.current && nameInput.current) {
      nameInput.current.focus();
      focusNewChannel.current = false;
    }
  }, [controller, state.selectedId, adding]);
  useLayoutEffect(() => {
    if (!returnFocus || state.writing || actions.busy) return;
    setReturnFocus(null);
    if (!ownsFocus(returnFocus.context)) return;
    const available = (element: HTMLElement | null | undefined) =>
      element?.isConnected &&
      panel.current?.contains(element) &&
      !element.matches(':disabled') &&
      element.getClientRects().length > 0;
    const target = [
      returnFocus.retry ? retryButton.current : null,
      returnFocus.preferred,
      panel.current?.querySelector<HTMLButtonElement>('.channel-nav-item[aria-pressed="true"]'),
      panel.current?.querySelector<HTMLButtonElement>('.channel-add'),
    ].find(available);
    target?.focus();
  }, [returnFocus, controller, state.writing, actions.busy]);
  useEffect(() => {
    if (keyInput.current) keyInput.current.value = '';
    setHasKeyInput(false);
    if (!adding && controller.savedChannel && !controller.locked) {
      void controller.credentialOperation(async (channel) => {
        const result = await call(
          {
            kind: 'generation_channel_credential_status',
            payload: { channel_id: channel.id, row_version: channel.row_version },
          },
          'generation_credential_status',
          identity,
        );
        return result.configured;
      });
    }
  }, [controller, state.connectionEpoch, state.loading, state.writing, adding]);

  function captureFocus(): ChannelFocusContext {
    return { controller, epoch: focusEpoch.current, anchor: document.activeElement };
  }
  function ownsFocus(context: ChannelFocusContext) {
    return (
      context.controller === currentController.current &&
      context.controller.isActive &&
      context.epoch === focusEpoch.current &&
      panel.current?.isConnected &&
      (document.activeElement === document.body || document.activeElement === context.anchor)
    );
  }
  function restoreFocus(
    context: ChannelFocusContext,
    preferred: HTMLElement | null = null,
    retry = false,
  ) {
    if (ownsFocus(context)) setReturnFocus({ context, preferred, retry });
  }
  function confirm(kind: 'discard' | 'delete', trigger: HTMLButtonElement) {
    if (blocked) return;
    ++focusEpoch.current;
    confirmationTrigger.current = trigger;
    setConfirming(kind);
  }
  function closeConfirmation(discard = false) {
    if (blocked) return;
    const context = captureFocus();
    if (discard) controller.discardSelected();
    setConfirming(null);
    restoreFocus(context, confirmationTrigger.current);
  }
  function change(patch: Partial<GenerationChannelSpec>) {
    setLocalError(null);
    setConfirming(null);
    controller.change(patch);
  }
  function create(template: ChannelTemplate) {
    const next = createChannelSpec(template);
    if (template === 'custom') next.name = t('自定义渠道', 'Custom channel');
    const existing = Object.values(state.drafts).filter((item) =>
      item.spec.name.startsWith(next.name),
    ).length;
    if (existing) next.name += ` ${existing + 1}`;
    focusNewChannel.current = true;
    controller.create(next, template);
    setAdding(false);
  }
  async function save() {
    if (!spec || blocked) return;
    const invalid = validateChannelSpec(spec);
    if (invalid) {
      setLocalError(invalid);
      return;
    }
    const context = captureFocus();
    const result = await controller.save((request) =>
      actions.execute(
        { kind: 'generation_channel_save', payload: { command_id: crypto.randomUUID(), request } },
        'generation_channel_mutation',
      ),
    );
    if (result) changed.current?.(result.settings);
    else restoreFocus(context, context.anchor instanceof HTMLElement ? context.anchor : null, true);
  }
  async function selectModel(model: string) {
    const result = await controller.selectModel(model, (request) =>
      actions.execute(
        {
          kind: 'generation_channel_select',
          payload: { command_id: crypto.randomUUID(), request },
        },
        'generation_settings_mutation',
      ),
    );
    if (result) changed.current?.(result.settings);
  }
  async function disable() {
    const result = await controller.disable((base_row_version) =>
      actions.execute(
        {
          kind: 'generation_settings_set',
          payload: {
            command_id: crypto.randomUUID(),
            request: { base_row_version, profile: null },
          },
        },
        'generation_settings_mutation',
      ),
    );
    if (result) changed.current?.(result.settings);
  }
  async function remove() {
    if (blocked) return;
    const context = captureFocus();
    const result = await controller.remove(profile?.channel_id === draft?.id, (request) =>
      actions.execute(
        {
          kind: 'generation_channel_delete',
          payload: { command_id: crypto.randomUUID(), request },
        },
        'generation_channel_mutation',
      ),
    );
    if (result) {
      if (context.epoch === focusEpoch.current && controller.isActive) setConfirming(null);
      restoreFocus(context);
      changed.current?.(result.settings);
    } else
      restoreFocus(context, context.anchor instanceof HTMLElement ? context.anchor : null, true);
  }
  async function credential(removeKey: boolean) {
    if (blocked || !saved) return;
    let secret = keyInput.current?.value ?? '';
    if (keyInput.current) keyInput.current.value = '';
    setHasKeyInput(false);
    try {
      await controller.credentialOperation(async (channel) => {
        if (removeKey)
          await call(
            {
              kind: 'generation_channel_credential_delete',
              payload: { channel_id: channel.id, row_version: channel.row_version },
            },
            'done',
            identity,
          );
        else
          await call(
            {
              kind: 'generation_channel_credential_set',
              payload: {
                channel_id: channel.id,
                row_version: channel.row_version,
                api_key: secret,
              },
            },
            'done',
            identity,
          );
        return !removeKey;
      });
    } finally {
      secret = '';
    }
  }
  async function recover() {
    if (actions.busy || !state.loaded) return;
    const context = captureFocus();
    const pending = actions.pending;
    if (!pending) {
      await actions.retry();
      restoreFocus(context, null, true);
      return;
    }
    if (
      pending.command.kind === 'generation_channel_save' ||
      pending.command.kind === 'generation_channel_delete'
    ) {
      const result = await actions.retry<'generation_channel_mutation'>();
      if (result && controller.isActive) {
        controller.confirmMutation(
          result as GenerationChannelMutation,
          pending.command.kind === 'generation_channel_save'
            ? pending.command.payload.request.spec
            : undefined,
        );
        if (pending.command.kind === 'generation_channel_delete') {
          if (context.epoch === focusEpoch.current) setConfirming(null);
          restoreFocus(context);
        } else restoreFocus(context, null, true);
        changed.current?.(result.settings);
      } else restoreFocus(context, null, true);
    } else {
      const result = await actions.retry<'generation_settings_mutation'>();
      if (result && controller.isActive) {
        controller.confirmSettings(result.settings);
        changed.current?.(result.settings);
      }
    }
  }
  function addModel(item?: DiscoveredGenerationModel) {
    if (!draft || !spec || editingBlocked) return;
    const id = item?.model ?? modelInputs[draft.id] ?? '';
    item ??= state.catalog?.models.find((candidate) => candidate.model === id.trim());
    const model = newChannelModel(
      id,
      spec.preset === 'cloudflare_workers_ai' ? 'cloudflare' : draft.template,
    );
    if (spec.provider !== 'openai_chat') delete model.chat_token_limit;
    if (
      item?.context_tokens != null &&
      item.context_tokens >= 256 &&
      item.context_tokens <= 2097152
    )
      model.context_tokens = item.context_tokens;
    const models = [...spec.models, model];
    const invalid = validateChannelSpec({ ...spec, models });
    if (invalid) {
      setLocalError(invalid);
      return;
    }
    change({ models });
    setModelInputs((old) => ({ ...old, [draft.id]: '' }));
  }
  const error = localError || state.error;

  return (
    <section ref={panel} className="generation-channels" aria-label={t('AI 渠道', 'AI channels')}>
      <header className="channel-panel-heading">
        <div>
          <h2>{t('AI 渠道', 'AI channels')}</h2>
          <p className="muted">
            {t(
              '为服务商或账户分别建一个渠道，在同一渠道中管理多个模型。',
              'Create a channel for each provider or account, and manage its models together.',
            )}
          </p>
        </div>
        <button
          className="channel-icon-button"
          type="button"
          title={t('刷新渠道，保留草稿', 'Refresh channels and keep drafts')}
          aria-label={t('刷新渠道，保留草稿', 'Refresh channels and keep drafts')}
          disabled={!!state.writing || state.loading}
          onClick={() => void controller.load(read)}
        >
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </header>
      <div className="channel-active-summary">
        <span className="channel-status-dot" data-active={!!profile} />
        <span>
          {profile
            ? t(`当前模型：${profile.model}`, `Current model: ${profile.model}`)
            : t('尚未选择生成模型', 'No generation model selected')}
          {profile && !profile.channel_id ? (
            <small> · {t('已有单模型配置', 'Existing single-model configuration')}</small>
          ) : null}
        </span>
        {profile?.channel_id ? (
          <button className="text-button" disabled={blocked} onClick={() => void disable()}>
            {t('停用', 'Disable')}
          </button>
        ) : null}
      </div>
      {error ? (
        <p className="inference-error" role="alert">
          {channelErrorMessage(error, t)}
        </p>
      ) : null}
      {actions.error ? (
        <p className="inference-error" role="alert">
          {actions.error}
        </p>
      ) : null}
      {!actions.ready || actions.pending ? (
        <div className="inference-notice">
          <p>
            {t(
              '有渠道操作等待读取或确认，草稿已保留。',
              'A channel operation awaits reading or confirmation. Drafts are retained.',
            )}
          </p>
          <button
            ref={retryButton}
            disabled={actions.busy || !state.loaded}
            onClick={() => void recover()}
          >
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      ) : null}
      {state.loading && !state.loaded ? (
        <p role="status">{t('正在读取渠道…', 'Loading channels…')}</p>
      ) : null}
      {!state.loaded && !state.loading ? (
        <button onClick={() => void controller.load(read)}>
          {t('重新读取渠道', 'Reload channels')}
        </button>
      ) : null}
      {state.loaded ? (
        <div className="channel-workspace">
          <nav className="channel-sidebar" aria-label={t('已配置渠道', 'Configured channels')}>
            <button
              className="channel-add"
              disabled={blocked}
              onClick={() => {
                ++focusEpoch.current;
                setAdding(true);
              }}
            >
              <Plus size={16} aria-hidden="true" />
              {t('添加渠道', 'Add channel')}
            </button>
            {Object.values(state.drafts).map((item) => (
              <button
                key={item.id}
                className="channel-nav-item"
                aria-pressed={!adding && state.selectedId === item.id}
                onClick={() => {
                  ++focusEpoch.current;
                  controller.viewChannel(item.id);
                  setAdding(false);
                }}
              >
                <span className="channel-provider-icon">
                  {item.spec.preset === 'cloudflare_workers_ai' ? (
                    <Cloud size={18} aria-hidden="true" />
                  ) : (
                    <Network size={18} aria-hidden="true" />
                  )}
                </span>
                <span className="channel-nav-label">
                  <strong>{item.spec.name || t('未命名渠道', 'Untitled channel')}</strong>
                  <small>
                    {item.conflict
                      ? t('需要核对', 'Review changes')
                      : item.dirty
                        ? t('未保存', 'Unsaved')
                        : `${item.spec.models.length} ${t('个模型', 'models')}`}
                  </small>
                </span>
                {profile?.channel_id === item.id ? (
                  <Check size={14} aria-label={t('当前渠道', 'Current channel')} />
                ) : null}
              </button>
            ))}
            {Object.keys(state.drafts).length === 0 ? (
              <p className="muted channel-sidebar-empty">
                {t(
                  '还没有渠道。选择服务商开始配置。',
                  'No channels yet. Choose a provider to get started.',
                )}
              </p>
            ) : null}
          </nav>
          <div className="channel-detail">
            {adding || !draft || !spec ? (
              <section aria-label={t('选择服务商', 'Choose provider')}>
                <h3>{t('添加 AI 渠道', 'Add an AI channel')}</h3>
                <p className="muted">
                  {t(
                    '选择预设，或使用自己的兼容服务。同一服务商可以添加多个账户。',
                    'Choose a preset or your own compatible service. You can add several accounts for one provider.',
                  )}
                </p>
                <div className="channel-presets">
                  {channelTemplates.map((template) => (
                    <button
                      key={template.id}
                      disabled={blocked}
                      onClick={() => create(template.id)}
                    >
                      <span className="channel-provider-icon">
                        {template.id === 'cloudflare' ? (
                          <Cloud size={20} aria-hidden="true" />
                        ) : template.id === 'custom' ? (
                          <Plus size={20} aria-hidden="true" />
                        ) : (
                          <Network size={20} aria-hidden="true" />
                        )}
                      </span>
                      <span>
                        <strong>{template.name || t('自定义服务商', 'Custom provider')}</strong>
                        <small>
                          {template.id === 'custom'
                            ? t('自定义名称、协议与地址', 'Your name, protocol, and endpoint')
                            : template.id === 'cloudflare'
                              ? t('Account ID 与 API Token', 'Account ID and API token')
                              : protocolFamily(template.provider) === 'openai'
                                ? t('OpenAI 兼容接口', 'OpenAI-compatible API')
                                : template.provider === 'gemini'
                                  ? t('Gemini 原生接口', 'Native Gemini API')
                                  : 'Anthropic Messages'}
                        </small>
                      </span>
                      <ChevronRight size={16} aria-hidden="true" />
                    </button>
                  ))}
                </div>
              </section>
            ) : (
              <>
                <header className="channel-detail-heading">
                  <div>
                    <h3>{spec.name || t('未命名渠道', 'Untitled channel')}</h3>
                    <span className="channel-state-label">
                      {draft.baseRowVersion === 0
                        ? t('新渠道 · 尚未保存', 'New channel · Not saved')
                        : draft.dirty
                          ? t('有未保存更改', 'Unsaved changes')
                          : t('渠道配置已保存', 'Channel configuration saved')}
                    </span>
                  </div>
                  <button
                    className="primary-button"
                    disabled={blocked || !draft.dirty || draft.conflict}
                    onClick={() => void save()}
                  >
                    {state.writing === 'save'
                      ? t('保存中…', 'Saving…')
                      : t('保存渠道', 'Save channel')}
                  </button>
                </header>
                {draft.conflict ? (
                  <div className="inference-notice">
                    <p>
                      {t(
                        '已存配置发生变化。当前输入仍在草稿中，请核对后处理。',
                        'The saved configuration changed. Your input remains in the draft for review.',
                      )}
                    </p>
                    <div className="channel-actions">
                      <button
                        disabled={blocked}
                        onClick={() => controller.copySelected(`${spec.name} ${t('副本', 'copy')}`)}
                      >
                        {t('草稿另存为新渠道', 'Copy draft to a new channel')}
                      </button>
                      <button
                        disabled={blocked}
                        onClick={(event) => confirm('discard', event.currentTarget)}
                      >
                        {t('采用已存配置…', 'Use saved configuration…')}
                      </button>
                    </div>
                  </div>
                ) : null}
                <div className="channel-fields">
                  <label className="inference-field">
                    {t('渠道名称', 'Channel name')}
                    <input
                      ref={nameInput}
                      value={spec.name}
                      maxLength={120}
                      disabled={editingBlocked}
                      onChange={(event) => change({ name: event.target.value })}
                    />
                  </label>
                  <div className="choice-field inference-field">
                    {t('协议家族', 'Protocol family')}
                    <Select
                      aria-label={t('协议家族', 'Protocol family')}
                      value={protocolFamily(spec.provider)}
                      disabled={editingBlocked || spec.preset === 'cloudflare_workers_ai'}
                      onValueChange={(nextValue) =>
                        change(
                          channelProtocolChange(
                            spec,
                            nextValue === 'openai' ? 'openai_chat' : nextValue,
                          ),
                        )
                      }
                    >
                      <option value="openai">OpenAI</option>
                      <option value="anthropic">Anthropic</option>
                      <option value="gemini">Gemini</option>
                    </Select>
                  </div>
                  {protocolFamily(spec.provider) === 'openai' ? (
                    <div className="choice-field inference-field">
                      {t('OpenAI 接口模式', 'OpenAI API mode')}
                      <Select
                        aria-label={t('OpenAI 接口模式', 'OpenAI API mode')}
                        value={spec.provider}
                        disabled={editingBlocked}
                        onValueChange={(nextValue) =>
                          change(channelProtocolChange(spec, nextValue))
                        }
                      >
                        <option value="openai_chat">Chat Completions</option>
                        <option value="openai_responses">Responses</option>
                      </Select>
                    </div>
                  ) : null}
                  {spec.preset === 'cloudflare_workers_ai' ? (
                    <label className="inference-field">
                      Cloudflare Account ID
                      <input
                        value={draft.accountInput ?? cloudflareAccount(spec.endpoint)}
                        spellCheck={false}
                        autoComplete="off"
                        disabled={editingBlocked}
                        placeholder={t('32 位 Account ID', '32-character Account ID')}
                        onChange={(event) => {
                          setLocalError(null);
                          controller.changeCloudflareAccount(event.target.value);
                        }}
                      />
                    </label>
                  ) : null}
                  <label className="inference-field channel-full-field">
                    {t('API 根地址', 'API base URL')}
                    <input
                      type="url"
                      value={spec.endpoint}
                      readOnly={spec.preset === 'cloudflare_workers_ai'}
                      disabled={editingBlocked}
                      spellCheck={false}
                      autoComplete="off"
                      placeholder="https://api.example.com/v1"
                      onChange={(event) => change({ endpoint: event.target.value })}
                    />
                  </label>
                </div>
                <p className="muted channel-endpoint-note">
                  {t(
                    '按服务商要求填写完整根路径，保留 /v1 或 /v1beta；不会自动补全版本。',
                    'Use the full base path required by the provider, including /v1 or /v1beta. API versions are not added automatically.',
                  )}
                </p>
                {channelRequestPreview(spec) ? (
                  <details className="channel-advanced">
                    <summary>
                      <ChevronRight
                        className="channel-disclosure-icon"
                        size={14}
                        aria-hidden="true"
                      />
                      {t('请求地址预览', 'Request URL preview')}
                    </summary>
                    <code className="channel-url-preview">{channelRequestPreview(spec)}</code>
                  </details>
                ) : null}
                <details className="channel-advanced">
                  <summary>
                    <ChevronRight
                      className="channel-disclosure-icon"
                      size={14}
                      aria-hidden="true"
                    />
                    <Settings2 size={14} aria-hidden="true" />
                    {t('高级传输设置', 'Advanced transport settings')}
                  </summary>
                  <div className="channel-fields">
                    <div className="choice-field inference-field">
                      {t('自由文本与对话', 'Text and chat')}
                      <Select
                        aria-label={t('自由文本与对话', 'Text and chat')}
                        value={spec.capabilities.text_transport}
                        disabled={
                          editingBlocked ||
                          (spec.preset === 'cloudflare_workers_ai' &&
                            spec.provider === 'openai_responses')
                        }
                        onValueChange={(nextValue) =>
                          change({
                            capabilities: {
                              ...spec.capabilities,
                              text_transport: nextValue === 'json' ? 'json' : 'stream',
                            },
                          })
                        }
                      >
                        <option value="stream">{t('流式 SSE', 'Streaming SSE')}</option>
                        <option value="json">{t('非流式 JSON', 'Non-streaming JSON')}</option>
                      </Select>
                    </div>
                    <div className="choice-field inference-field">
                      {t('结构化证据回答', 'Structured evidence answers')}
                      <Select
                        aria-label={t('结构化证据回答', 'Structured evidence answers')}
                        value={spec.capabilities.structured_transport ?? 'unsupported'}
                        disabled={editingBlocked}
                        onValueChange={(nextValue) =>
                          change({
                            capabilities: {
                              ...spec.capabilities,
                              structured_transport:
                                nextValue === 'unsupported'
                                  ? null
                                  : nextValue === 'json'
                                    ? 'json'
                                    : 'stream',
                            },
                          })
                        }
                      >
                        <option value="stream" disabled={spec.preset === 'cloudflare_workers_ai'}>
                          {t('流式 SSE', 'Streaming SSE')}
                        </option>
                        <option value="json">{t('非流式 JSON', 'Non-streaming JSON')}</option>
                        <option value="unsupported">
                          {t('此渠道不支持', 'Not supported by this channel')}
                        </option>
                      </Select>
                    </div>
                  </div>
                  <p className="muted">
                    {spec.preset === 'cloudflare_workers_ai'
                      ? t(
                          'Cloudflare 的结构化回答使用非流式 JSON；Responses 文本也使用非流式 JSON。模型须支持所选能力。',
                          'Cloudflare structured answers use non-streaming JSON. Responses text also uses non-streaming JSON. The model must support the selected capability.',
                        )
                      : t(
                          '按服务商与模型的真实能力选择。请求失败会显示原因，不会自动切换传输或移除结构化要求。',
                          'Choose transports supported by the provider and model. A failed request reports an error without switching transport or dropping structured output.',
                        )}
                  </p>
                </details>
                <section className="channel-credentials" aria-label={t('渠道密钥', 'Channel key')}>
                  <h4>
                    <KeyRound size={15} aria-hidden="true" />
                    {t('API 密钥', 'API key')}
                  </h4>
                  <p className="muted">
                    {t(
                      '同一渠道的模型共用此密钥，保存到本机系统凭据库。',
                      'Models in this channel share one key, stored in the system credential vault.',
                    )}
                  </p>
                  <label className="inference-field">
                    <span className="visually-hidden">
                      {t('输入渠道 API 密钥', 'Enter channel API key')}
                    </span>
                    <input
                      key={`${state.selectedId}/${state.connectionEpoch}`}
                      ref={keyInput}
                      type="password"
                      autoComplete="new-password"
                      spellCheck={false}
                      disabled={blocked || !saved || state.credentialBusy}
                      placeholder={
                        state.credentialConfigured
                          ? t('已保存；输入新密钥以替换', 'Saved; enter a new key to replace it')
                          : t('输入 API 密钥', 'Enter API key')
                      }
                      onChange={(event) => setHasKeyInput(event.target.value.length > 0)}
                    />
                  </label>
                  <div className="channel-actions">
                    <button
                      className="secondary-button"
                      disabled={blocked || !saved || state.credentialBusy || !hasKeyInput}
                      onClick={() => void credential(false)}
                    >
                      {t('保存密钥', 'Save key')}
                    </button>
                    <button
                      className="text-button"
                      disabled={
                        blocked ||
                        !saved ||
                        state.credentialBusy ||
                        state.credentialConfigured !== true
                      }
                      onClick={() => void credential(true)}
                    >
                      {t('移除密钥', 'Remove key')}
                    </button>
                    <span role="status" className="muted">
                      {!saved
                        ? t('先保存渠道，再管理密钥。', 'Save the channel before managing its key.')
                        : state.credentialBusy
                          ? t('正在读取或更新密钥…', 'Reading or updating key…')
                          : state.credentialConfigured === true
                            ? t('密钥已保存', 'Key saved')
                            : state.credentialConfigured === false
                              ? t('尚未保存密钥', 'No key saved')
                              : t('密钥状态未知', 'Key status unknown')}
                    </span>
                  </div>
                  {state.credentialError ? (
                    <p role="alert" className="inference-error">
                      {channelErrorMessage(state.credentialError, t)}
                    </p>
                  ) : null}
                </section>
                <section className="channel-models" aria-label={t('渠道模型', 'Channel models')}>
                  <div className="channel-section-heading">
                    <h4>
                      {t('模型', 'Models')} <small>{spec.models.length}</small>
                    </h4>
                    <button
                      className="secondary-button"
                      disabled={
                        blocked ||
                        !saved ||
                        state.listing ||
                        state.credentialConfigured !== true ||
                        hasKeyInput
                      }
                      onClick={() =>
                        void controller.loadModels((channel) =>
                          call(
                            {
                              kind: 'generation_channel_models_list',
                              payload: { channel_id: channel.id, row_version: channel.row_version },
                            },
                            'remote_generation_models',
                            identity,
                          ),
                        )
                      }
                    >
                      <RefreshCw size={14} aria-hidden="true" />
                      {state.listing
                        ? t('获取中…', 'Fetching…')
                        : t('获取模型列表', 'Fetch models')}
                    </button>
                  </div>
                  {state.catalogError ? (
                    <p role="alert" className="inference-error">
                      {channelErrorMessage(state.catalogError, t)}
                    </p>
                  ) : null}
                  <div className="channel-manual-model">
                    <label className="inference-field">
                      {t('搜索或输入模型 ID', 'Find or enter a model ID')}
                      <Combobox
                        aria-label={t('搜索或输入模型 ID', 'Find or enter a model ID')}
                        value={modelInputs[draft.id] ?? ''}
                        disabled={editingBlocked}
                        allowCustom
                        loading={state.listing}
                        options={(state.catalog?.models ?? []).map((item) => ({
                          value: item.model,
                          label: item.display_name || item.model,
                          description: spec.models.some((model) => model.model === item.model)
                            ? `${item.model} · ${t('已添加', 'Added')}`
                            : item.model,
                          disabled: spec.models.some((model) => model.model === item.model),
                        }))}
                        emptyText={t(
                          '没有匹配模型，可直接输入模型 ID',
                          'No matching model; enter a model ID',
                        )}
                        spellCheck={false}
                        autoComplete="off"
                        placeholder={t(
                          '填写服务商提供的模型 ID',
                          'Model ID supplied by your provider',
                        )}
                        onValueChange={(value) =>
                          setModelInputs((old) => ({ ...old, [draft.id]: value }))
                        }
                      />
                    </label>
                    <button
                      className="secondary-button"
                      disabled={editingBlocked || !(modelInputs[draft.id] ?? '').trim()}
                      onClick={() => addModel()}
                    >
                      <Plus size={14} aria-hidden="true" />
                      {t('添加模型', 'Add model')}
                    </button>
                  </div>
                  {state.catalog ? (
                    <p className="muted" role="status">
                      {t(
                        `已获取 ${state.catalog.models.length} 个模型，可搜索选择或直接填写 ID；添加后保存渠道。`,
                        `${state.catalog.models.length} models available. Search or enter an ID, then save the channel after adding.`,
                      )}
                    </p>
                  ) : null}
                  {spec.models.length === 0 ? (
                    <p className="muted">
                      {t(
                        '可以先保存渠道和密钥，再获取或手动添加模型。',
                        'Save the channel and key first, then fetch or add models.',
                      )}
                    </p>
                  ) : (
                    <div className="channel-model-list">
                      {spec.models.map((model, index) => {
                        const activeModel =
                          profile?.channel_id === draft.id && profile.model === model.model;
                        return (
                          <article key={index} className="channel-model-row">
                            <div className="channel-model-heading">
                              <strong>{model.model}</strong>
                              <div>
                                {activeModel ? (
                                  <span className="channel-badge">
                                    <Check size={12} aria-hidden="true" />
                                    {t('当前模型', 'Current')}
                                  </span>
                                ) : null}
                                {state.verifiedModels.includes(model.model) ? (
                                  <span className="channel-badge">
                                    {t('测试通过', 'Test passed')}
                                  </span>
                                ) : null}
                                <button
                                  className="channel-icon-button"
                                  aria-label={t(
                                    `移除模型 ${model.model}`,
                                    `Remove model ${model.model}`,
                                  )}
                                  title={
                                    activeModel
                                      ? t(
                                          '先选择其他模型或停用，再移除此模型',
                                          'Select another model or disable generation before removing this model',
                                        )
                                      : t('移除模型', 'Remove model')
                                  }
                                  disabled={editingBlocked || activeModel}
                                  onClick={() =>
                                    change({
                                      models: spec.models.filter((_, item) => item !== index),
                                    })
                                  }
                                >
                                  <Trash2 size={14} aria-hidden="true" />
                                </button>
                              </div>
                            </div>
                            <details>
                              <summary>{t('模型参数', 'Model parameters')}</summary>
                              <div className="channel-fields">
                                <label className="inference-field channel-full-field">
                                  {t('模型 ID', 'Model ID')}
                                  <input
                                    value={model.model}
                                    disabled={editingBlocked || activeModel}
                                    onChange={(event) =>
                                      controller.changeModel(index, { model: event.target.value })
                                    }
                                    spellCheck={false}
                                  />
                                </label>
                                <label className="inference-field">
                                  {t('上下文容量', 'Context capacity')}
                                  <input
                                    type="number"
                                    min={256}
                                    max={2097152}
                                    step={1}
                                    value={model.context_tokens}
                                    disabled={editingBlocked}
                                    onChange={(event) =>
                                      controller.changeModel(index, {
                                        context_tokens: Number(event.target.value),
                                      })
                                    }
                                  />
                                </label>
                                <label className="inference-field">
                                  {t('温度', 'Temperature')}
                                  <input
                                    type="number"
                                    min={0}
                                    max={2}
                                    step={0.1}
                                    value={model.temperature}
                                    disabled={editingBlocked}
                                    onChange={(event) =>
                                      controller.changeModel(index, {
                                        temperature: Number(event.target.value),
                                      })
                                    }
                                  />
                                </label>
                                {spec.provider === 'openai_chat' ? (
                                  <div className="choice-field inference-field channel-full-field">
                                    {t('输出预算字段', 'Output budget field')}
                                    <Select
                                      aria-label={t('输出预算字段', 'Output budget field')}
                                      value={model.chat_token_limit ?? 'max_completion_tokens'}
                                      disabled={editingBlocked}
                                      onValueChange={(nextValue) =>
                                        controller.changeModel(index, {
                                          chat_token_limit:
                                            nextValue === 'max_tokens' ? 'max_tokens' : null,
                                        })
                                      }
                                    >
                                      <option value="max_completion_tokens">
                                        max_completion_tokens
                                      </option>
                                      <option value="max_tokens">max_tokens</option>
                                    </Select>
                                  </div>
                                ) : null}
                              </div>
                              <p className="muted">
                                {t(
                                  '上下文容量按模型实际能力填写；手动模型默认 65,536。DeepSeek Chat 的预算字段使用 max_tokens。',
                                  'Use the model’s actual context capacity; manual entries default to 65,536. DeepSeek Chat uses max_tokens for its budget field.',
                                )}
                              </p>
                            </details>
                            <div className="channel-actions">
                              <button
                                className="secondary-button"
                                disabled={
                                  blocked ||
                                  !saved ||
                                  hasKeyInput ||
                                  state.credentialConfigured !== true ||
                                  spec.capabilities.structured_transport === null ||
                                  !!state.checkingModel
                                }
                                onClick={() =>
                                  void controller.verify(model.model, (candidate) =>
                                    call(
                                      {
                                        kind: 'generation_model_verify',
                                        payload: { profile: candidate },
                                      },
                                      'done',
                                      identity,
                                    ),
                                  )
                                }
                              >
                                {state.checkingModel === model.model
                                  ? t('测试中…', 'Testing…')
                                  : t('测试证据回答', 'Test evidence answers')}
                              </button>
                              <button
                                className="text-button"
                                disabled={blocked || !saved || activeModel}
                                onClick={() => void selectModel(model.model)}
                              >
                                {t('设为默认', 'Set as default')}
                              </button>
                            </div>
                          </article>
                        );
                      })}
                    </div>
                  )}
                  {state.verificationError ? (
                    <p className="inference-error" role="alert">
                      {channelErrorMessage(state.verificationError, t)}
                    </p>
                  ) : null}
                  <p className="muted channel-test-note">
                    {spec.capabilities.structured_transport === null
                      ? t(
                          '此渠道已设为仅支持文本，可用于 AI 对话；证据回答及其测试不可用。',
                          'This channel is configured for text only. AI chat is available; evidence answers and their test are unavailable.',
                        )
                      : t(
                          '测试证据回答会发送一次简短的结构化请求，可能计费。保存渠道与获取模型列表不代表已验证推理能力。',
                          'Testing evidence answers sends a short structured request and may incur a charge. Saving a channel or fetching its catalog does not verify inference.',
                        )}
                  </p>
                </section>
                <footer className="channel-detail-footer">
                  <div className="channel-actions">
                    <button
                      className="primary-button"
                      disabled={blocked || !draft.dirty || draft.conflict}
                      onClick={() => void save()}
                    >
                      {t('保存渠道', 'Save channel')}
                    </button>
                    {draft.dirty ? (
                      <button
                        className="text-button"
                        disabled={blocked}
                        onClick={(event) => confirm('discard', event.currentTarget)}
                      >
                        {draft.baseRowVersion === 0
                          ? t('取消新增…', 'Cancel new channel…')
                          : t('放弃更改…', 'Discard changes…')}
                      </button>
                    ) : null}
                  </div>
                  {draft.baseRowVersion > 0 ? (
                    <button
                      className="channel-danger"
                      disabled={blocked || !saved}
                      onClick={(event) => confirm('delete', event.currentTarget)}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                      {t('移除渠道…', 'Remove channel…')}
                    </button>
                  ) : null}
                </footer>
                {confirming ? (
                  <div
                    className="channel-confirm"
                    role="group"
                    aria-label={t('确认渠道操作', 'Confirm channel action')}
                  >
                    <p>
                      {confirming === 'discard'
                        ? t(
                            '放弃当前未保存输入？其他渠道的草稿会保留。',
                            'Discard this channel’s unsaved input? Drafts for other channels are retained.',
                          )
                        : profile?.channel_id === draft.id
                          ? t(
                              '移除此渠道并停用当前模型？已有对话与回答会保留。',
                              'Remove this channel and disable the current model? Existing conversations and answers are retained.',
                            )
                          : t(
                              '移除此渠道？已有对话与回答会保留。',
                              'Remove this channel? Existing conversations and answers are retained.',
                            )}
                    </p>
                    <div className="channel-actions">
                      <button
                        className="secondary-button"
                        disabled={blocked}
                        onClick={() => closeConfirmation()}
                      >
                        {t('取消', 'Cancel')}
                      </button>
                      <button
                        className="channel-danger"
                        disabled={blocked}
                        onClick={() => {
                          if (confirming === 'discard') closeConfirmation(true);
                          else void remove();
                        }}
                      >
                        {confirming === 'discard'
                          ? t('确认放弃更改', 'Discard changes')
                          : profile?.channel_id === draft.id
                            ? t('移除并停用', 'Remove and disable')
                            : t('确认移除渠道', 'Remove channel')}
                      </button>
                    </div>
                  </div>
                ) : null}
              </>
            )}
          </div>
        </div>
      ) : null}
    </section>
  );
}
