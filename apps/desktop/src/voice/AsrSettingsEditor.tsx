import { useEffect, useState } from 'react';
import { Check, LoaderCircle, Save, Sparkles, Trash2 } from 'lucide-react';
import type { AsrSettingsState, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { PageState } from '../layout/PageState';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { getAsrSettings, saveAsrSettings, testAsrSettings } from './api';
import { voiceErrorText } from './voice-errors';
import './asr-settings.css';

export function AsrSettingsEditor({ identity }: { identity: WorkspaceIdentity }) {
  const { t } = useI18n();
  const [saved, setSaved] = useState<AsrSettingsState | null>(null);
  const [draft, setDraft] = useState({ endpoint: '', model: '', language: '' });
  const [credential, setCredential] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [confirmClear, setConfirmClear] = useState(false);
  useEffect(() => {
    let live = true;
    setSaved(null);
    setError(null);
    void getAsrSettings(identity)
      .then((state) => {
        if (!live) return;
        setSaved(state);
        setDraft({
          endpoint: state.settings.endpoint,
          model: state.settings.model,
          language: state.settings.language,
        });
      })
      .catch((failure) => {
        if (live) setError(voiceErrorText(failure, t));
      });
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, attempt]);
  const dirty = Boolean(
    saved &&
    (credential.trim() ||
      Object.entries(draft).some(
        ([key, value]) => value !== saved.settings[key as keyof typeof draft],
      )),
  );
  async function save(clear = false) {
    if (!saved || busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const fields = clear ? saved.settings : draft;
      const state = await saveAsrSettings(
        identity,
        {
          base_row_version: saved.settings.row_version,
          endpoint: fields.endpoint,
          model: fields.model,
          language: fields.language,
        },
        clear || !credential.trim() ? null : credential,
        clear,
      );
      setSaved(state);
      if (!clear) {
        setDraft({
          endpoint: state.settings.endpoint,
          model: state.settings.model,
          language: state.settings.language,
        });
        setCredential('');
      }
      setConfirmClear(false);
      setNotice(
        clear
          ? t(
              '本机密钥已删除，录音与文本保留。',
              'The local key was removed. Audio and text are kept.',
            )
          : t('转写设置已保存。', 'Transcription settings saved.'),
      );
    } catch (failure) {
      setError(voiceErrorText(failure, t));
    } finally {
      setBusy(false);
    }
  }
  async function test() {
    if (!saved || busy || dirty || !saved.credential_configured) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const result = await testAsrSettings(identity);
      setNotice(t(`连接成功，${result.latency_ms} ms。`, `Connected in ${result.latency_ms} ms.`));
    } catch (failure) {
      setError(voiceErrorText(failure, t));
    } finally {
      setBusy(false);
    }
  }
  if (!saved)
    return error ? (
      <PageState
        kind="error"
        title={t('无法读取转写设置', 'Could not load transcription settings')}
        detail={error}
        onRetry={() => setAttempt((value) => value + 1)}
      />
    ) : (
      <PageState kind="loading" title={t('正在读取转写设置…', 'Loading transcription settings…')} />
    );
  return (
    <div className="asr-settings">
      <p className="muted">
        {t(
          '设置语音转文字使用的服务。原始录音始终保留在知识库中。',
          'Choose the service used for speech-to-text. Original recordings stay in your workspace.',
        )}
      </p>
      <p className="asr-settings-state" role="status">
        {dirty
          ? t('有未保存的设置', 'Unsaved settings')
          : saved.credential_configured
            ? t('服务已配置', 'Service configured')
            : t('尚未配置密钥', 'No API key configured')}
      </p>
      {error && (
        <p className="asr-settings-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="asr-settings-notice" role="status">
          <Check size={15} aria-hidden="true" />
          {notice}
        </p>
      )}
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label className="asr-field">
          <span>{t('服务地址', 'Service endpoint')}</span>
          <input
            className="ui-input"
            value={draft.endpoint}
            disabled={busy}
            placeholder="https://api.openai.com/v1"
            onChange={(event) => setDraft((value) => ({ ...value, endpoint: event.target.value }))}
          />
        </label>
        <p className="muted">
          {t(
            '填写兼容 OpenAI 转写接口的完整 API 根地址。',
            'Enter the full API root for an OpenAI-compatible transcription service.',
          )}
        </p>
        <label className="asr-field">
          <span>{t('模型', 'Model')}</span>
          <input
            className="ui-input"
            value={draft.model}
            disabled={busy}
            placeholder="whisper-1"
            onChange={(event) => setDraft((value) => ({ ...value, model: event.target.value }))}
          />
        </label>
        <label className="asr-field">
          <span>{t('语言（可选）', 'Language (optional)')}</span>
          <input
            className="ui-input"
            value={draft.language}
            disabled={busy}
            maxLength={32}
            placeholder="zh"
            onChange={(event) => setDraft((value) => ({ ...value, language: event.target.value }))}
          />
        </label>
        <label className="asr-field">
          <span>{t('API 密钥', 'API key')}</span>
          <input
            className="ui-input"
            type="password"
            autoComplete="off"
            value={credential}
            disabled={busy}
            placeholder={
              saved.credential_configured
                ? t('已保存；留空保留现有密钥', 'Saved; leave blank to keep it')
                : t('输入服务提供的密钥', 'Enter your service API key')
            }
            onChange={(event) => setCredential(event.target.value)}
          />
        </label>
        <p className="muted">
          {t(
            '密钥保存在本机凭据管理器中。修改地址或模型后，请重新检查密钥是否适用。',
            'Keys are stored in the system credential vault. Check the key after changing the endpoint or model.',
          )}
        </p>
        <div className="asr-settings-actions">
          <button className="primary-button" type="submit" disabled={busy || !dirty}>
            {busy ? (
              <LoaderCircle size={15} aria-hidden="true" />
            ) : (
              <Save size={15} aria-hidden="true" />
            )}
            {t('保存设置', 'Save settings')}
          </button>
          <button
            className="secondary-button"
            type="button"
            disabled={busy || dirty || !saved.credential_configured}
            onClick={() => void test()}
          >
            <Sparkles size={15} aria-hidden="true" />
            {t('测试连接', 'Test connection')}
          </button>
          {dirty && (
            <button
              className="text-button"
              type="button"
              disabled={busy}
              onClick={() => {
                setDraft({
                  endpoint: saved.settings.endpoint,
                  model: saved.settings.model,
                  language: saved.settings.language,
                });
                setCredential('');
                setError(null);
              }}
            >
              {t('取消修改', 'Discard changes')}
            </button>
          )}
          {saved.credential_configured && (
            <button
              className="text-button is-danger"
              type="button"
              disabled={busy}
              onClick={() => setConfirmClear(true)}
            >
              <Trash2 size={15} aria-hidden="true" />
              {t('删除本机密钥', 'Remove local key')}
            </button>
          )}
        </div>
      </form>
      {confirmClear && (
        <ActionConfirmDialog
          title={t('删除本机转写密钥？', 'Remove the local transcription key?')}
          description={t(
            '录音和文本会保留；重新转写前需要配置密钥。',
            'Audio and text are kept. A key is needed before transcribing again.',
          )}
          confirmLabel={t('删除密钥', 'Remove key')}
          tone="danger"
          onCancel={() => setConfirmClear(false)}
          onConfirm={() => void save(true)}
        />
      )}
    </div>
  );
}
