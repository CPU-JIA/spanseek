import { useEffect, useState } from 'react';
import type { RetrievalParams, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { DEFAULT_RETRIEVAL_PARAMS, clampParams } from './retrieval-params';
import { saveRetrievalParams, useRetrievalSettings } from './retrieval-settings';
import './retrieval-settings.css';

/**
 * The two library-level switches that decide whether enabled text may leave this
 * device. Saving stays explicit, so one stray click never turns remote processing
 * on, and the acknowledgement is stored with the workspace instead of the window.
 */
export function RetrievalConsentCard({
  identity,
  onStateChange,
}: {
  identity: WorkspaceIdentity;
  onStateChange?: (params: RetrievalParams) => void;
}) {
  const { t } = useI18n();
  const retrieval = useRetrievalSettings(identity);
  const [params, setParams] = useState<RetrievalParams>(DEFAULT_RETRIEVAL_PARAMS);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const saved = retrieval.settings?.params ?? null;
  useEffect(() => {
    if (retrieval.settings) {
      setParams(retrieval.settings.params);
      onStateChange?.(retrieval.settings.params);
    }
  }, [retrieval.settings]);
  const changed =
    saved !== null &&
    (params.semantic_enabled !== saved.semantic_enabled ||
      params.remote_consent !== saved.remote_consent ||
      (params.cost_per_million_input ?? null) !== (saved.cost_per_million_input ?? null) ||
      (params.cost_per_million_output ?? null) !== (saved.cost_per_million_output ?? null));
  async function persist() {
    const loaded = retrieval.settings;
    if (!loaded) return;
    setBusy(true);
    setNote(null);
    setFailure(null);
    try {
      const result = await saveRetrievalParams(identity, loaded.row_version, clampParams(params));
      retrieval.setSettings(result);
      setParams(result.params);
      onStateChange?.(result.params);
      setNote(t('已保存。', 'Saved.'));
    } catch (error) {
      setFailure(
        error instanceof Error
          ? error.message
          : t('保存失败，请重试。', 'Could not save; try again.'),
      );
    } finally {
      setBusy(false);
    }
  }
  const locked = busy || retrieval.busy || !saved;
  return (
    <section className="inference-card">
      <h2>{t('语义检索与远程数据', 'Semantic retrieval and remote data')}</h2>
      <p className="muted">
        {t(
          '语义检索使用你配置的向量服务，答案生成使用你配置的 AI 渠道。两者都可能把参与检索的文本与你的问题发送到远程服务，因此这两项按知识库记录，换库后需要重新确认。',
          'Semantic retrieval uses the embedding service you configure and answers use the AI channel you configure. Both may send participating text and your question to a remote service, so both switches are stored per workspace and must be confirmed again in another workspace.',
        )}
      </p>
      <label className="retrieval-consent-row">
        <input
          type="checkbox"
          disabled={locked}
          checked={params.semantic_enabled}
          onChange={(event) => {
            setNote(null);
            setParams((old) => ({ ...old, semantic_enabled: event.target.checked }));
          }}
        />
        <span>
          {t('启用语义检索（向量检索）', 'Enable semantic (vector) retrieval')}
          <small>
            {t(
              '关闭后，搜索与有据问答只使用本机关键词检索，不会请求向量服务；笔记、阅读与关键词检索不受影响。',
              'When off, search and cited answers use local keyword retrieval only and never call the embedding service. Notes, reading and keyword search are unaffected.',
            )}
          </small>
        </span>
      </label>
      <label className="retrieval-consent-row">
        <input
          type="checkbox"
          disabled={locked}
          checked={params.remote_consent}
          onChange={(event) => {
            setNote(null);
            setParams((old) => ({ ...old, remote_consent: event.target.checked }));
          }}
        />
        <span>
          {t(
            '我已知情并同意：启用的文本可发送到我配置的远程服务',
            'I acknowledge that enabled text may reach the remote services I configure',
          )}
          <small>
            {t(
              '未确认前，远程向量服务与远程 AI 渠道的配置无法保存。费用由你与所选服务商的账户产生，本应用不代收、不转发凭据；原文、向量与索引仍保存在本机，关闭开关后不再发送。',
              'Until this is acknowledged, remote embedding and remote AI channel configuration cannot be saved. Usage is billed by your own provider account, not by this app, and credentials are never forwarded. Originals, vectors and indexes stay on this device, and turning the switch off stops further sending.',
            )}
          </small>
        </span>
      </label>
      <p className="muted">
        {t(
          '费用估算（可选）：填写你自己服务商的参考单价后，诊断页会用实测 Token 估算近 30 天费用；留空则不显示估算，应用也不会假定任何价格。',
          'Cost estimate (optional): fill in your own provider prices and the diagnostics page estimates the last 30 days from measured tokens. Left blank, no estimate is shown and no price is assumed.',
        )}
      </p>
      <div className="inference-controls">
        <label className="inference-field">
          {t('输入单价（每百万 Token）', 'Input price per million tokens')}
          <input
            type="number"
            min={0}
            step="0.01"
            inputMode="decimal"
            disabled={locked}
            value={params.cost_per_million_input ?? ''}
            onChange={(event) => {
              setNote(null);
              setParams((old) => ({
                ...old,
                cost_per_million_input:
                  event.target.value === '' ? null : Number(event.target.value),
              }));
            }}
          />
        </label>
        <label className="inference-field">
          {t('输出单价（每百万 Token）', 'Output price per million tokens')}
          <input
            type="number"
            min={0}
            step="0.01"
            inputMode="decimal"
            disabled={locked}
            value={params.cost_per_million_output ?? ''}
            onChange={(event) => {
              setNote(null);
              setParams((old) => ({
                ...old,
                cost_per_million_output:
                  event.target.value === '' ? null : Number(event.target.value),
              }));
            }}
          />
        </label>
      </div>
      <div className="inference-controls">
        <button
          type="button"
          className="secondary-button"
          disabled={locked || !changed}
          onClick={() => void persist()}
        >
          {busy ? t('正在保存…', 'Saving…') : t('保存', 'Save')}
        </button>
        {changed && !busy && (
          <span className="muted">{t('有未保存的更改', 'Unsaved changes')}</span>
        )}
      </div>
      {failure && (
        <p className="inference-error" role="alert">
          {failure}
        </p>
      )}
      {note && <p role="status">{note}</p>}
      {retrieval.error && (
        <p className="inference-error" role="alert">
          {retrieval.error}
        </p>
      )}
    </section>
  );
}
