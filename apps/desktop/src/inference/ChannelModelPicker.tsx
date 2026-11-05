import { Select } from '../ui/Select';
import { useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { RefreshCw, Settings2 } from 'lucide-react';
import type { GenerationSettings, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { GenerationChannelDraft } from './generation-channel-draft';
import { channelErrorMessage } from './channel-messages';
import { useInferenceAction } from './useInferenceAction';
import './generation-channels.css';

export type ChannelModelPickerProps = {
  identity: WorkspaceIdentity;
  scope: string;
  disabled?: boolean;
  purpose?: 'chat' | 'structured';
  onChanged?: (settings: GenerationSettings) => void;
  onManage?: () => void;
  refreshKey?: number;
};

/** Chooses a saved model without starting inference or changing its channel configuration. */
export function ChannelModelPicker({
  identity,
  scope,
  disabled = false,
  purpose = 'chat',
  onChanged,
  onManage,
  refreshKey = 0,
}: ChannelModelPickerProps) {
  const { t } = useI18n();
  const controller = useMemo(
    () => new GenerationChannelDraft(),
    [identity.local_instance_id, identity.workspace_id, refreshKey],
  );
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const actions = useInferenceAction(identity, `channel-picker/${scope}`);
  const changed = useRef(onChanged);
  changed.current = onChanged;
  async function load() {
    const loaded = await controller.load(() =>
      call({ kind: 'generation_channels' }, 'generation_channels', identity),
    );
    const settings = controller.getSnapshot().server?.settings;
    if (loaded && settings) changed.current?.(settings);
  }
  useEffect(() => {
    void load();
    return () => controller.invalidate();
  }, [controller]);
  const channels = state.server?.channels.filter((channel) => !channel.deleted) ?? [];
  const profile = state.server?.settings.profile;
  const noModels = state.loaded && !channels.some((channel) => channel.spec.models.length > 0);
  const value = profile?.channel_id
    ? JSON.stringify([profile.channel_id, profile.model])
    : profile
      ? 'legacy'
      : '';
  const blocked = disabled || controller.locked || actions.locked;
  async function select(value: string) {
    const found = channels
      .flatMap((channel) => channel.spec.models.map((model) => ({ channel, model })))
      .find((item) => JSON.stringify([item.channel.id, item.model.model]) === value);
    if (
      blocked ||
      !found ||
      (purpose === 'structured' && found.channel.spec.capabilities.structured_transport === null)
    )
      return;
    controller.viewChannel(found.channel.id);
    const result = await controller.selectModel(found.model.model, (request) =>
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
  async function recover() {
    const result = await actions.retry<'generation_settings_mutation'>();
    if (result && controller.isActive) {
      controller.confirmSettings(result.settings);
      changed.current?.(result.settings);
    }
  }
  return (
    <div className="channel-model-picker">
      <div className="choice-field">
        <span>{t('模型', 'Model')}</span>
        <Select
          aria-label={
            purpose === 'structured'
              ? t('证据回答模型', 'Evidence answer model')
              : t('对话模型', 'Chat model')
          }
          value={value}
          disabled={blocked}
          onValueChange={(nextValue) => void select(nextValue)}
        >
          <option value="" disabled>
            {state.loading
              ? t('正在读取模型…', 'Loading models…')
              : noModels
                ? t('未配置模型', 'No model configured')
                : t('选择已配置模型', 'Choose a configured model')}
          </option>
          {profile && !profile.channel_id ? (
            <option value="legacy" disabled>
              {profile.model} · {t('已有配置', 'Existing configuration')}
            </option>
          ) : null}
          {channels.map((channel) => (
            <optgroup key={channel.id} label={channel.spec.name}>
              {channel.spec.models.map((model) => (
                <option
                  key={model.model}
                  value={JSON.stringify([channel.id, model.model])}
                  disabled={
                    purpose === 'structured' &&
                    channel.spec.capabilities.structured_transport === null
                  }
                >
                  {model.model}
                  {purpose === 'structured' &&
                  channel.spec.capabilities.structured_transport === null
                    ? ` · ${t('不支持证据回答', 'No evidence answers')}`
                    : ''}
                </option>
              ))}
            </optgroup>
          ))}
        </Select>
      </div>
      <button
        type="button"
        className="channel-icon-button"
        aria-label={t('刷新模型选择', 'Refresh model choices')}
        title={t('刷新模型选择', 'Refresh model choices')}
        disabled={disabled || !!state.writing || state.loading || actions.busy}
        onClick={() => void load()}
      >
        <RefreshCw size={14} aria-hidden="true" />
      </button>
      {onManage ? (
        <button
          type="button"
          className="channel-icon-button"
          aria-label={t('打开模型与连接', 'Open models & connections')}
          title={t('打开模型与连接', 'Open models & connections')}
          disabled={disabled || !!state.writing || actions.busy}
          onClick={onManage}
        >
          <Settings2 size={14} aria-hidden="true" />
        </button>
      ) : null}
      {state.error ? (
        <p className="channel-picker-message" role="alert">
          {channelErrorMessage(state.error, t)}
        </p>
      ) : null}
      {actions.error ? (
        <p className="channel-picker-message" role="alert">
          {actions.error}
        </p>
      ) : null}
      {!actions.ready || actions.pending ? (
        <div className="channel-picker-message">
          <span>
            {t('模型切换等待读取或确认。', 'Model selection awaits reading or confirmation.')}
          </span>{' '}
          <button type="button" disabled={actions.busy || disabled} onClick={() => void recover()}>
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      ) : null}
      {noModels ? (
        <p className="channel-picker-message muted">
          {profile ? (
            t(
              '正在使用已有配置；可在「模型与连接」中添加更多模型。',
              'Using the existing configuration. Add more models in Models & connections.',
            )
          ) : onManage ? (
            <>
              {t('还没有可用模型。', 'No model is available yet.')}{' '}
              <button
                type="button"
                className="channel-picker-action"
                disabled={disabled}
                onClick={onManage}
              >
                {t('配置模型', 'Configure model')}
              </button>
            </>
          ) : (
            t(
              '请先在「模型与连接」中添加 AI 渠道和模型。',
              'Add an AI channel and model in Models & connections first.',
            )
          )}
        </p>
      ) : null}
    </div>
  );
}
