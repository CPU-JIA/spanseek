import { useCallback, useEffect, useRef, useState } from 'react';
import type { RetrievalParams, RetrievalSettings, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import {
  MAX_ANSWER_TIMEOUT_SECONDS,
  MAX_CITATION_COUNT,
  MAX_TOP_K,
  MIN_ANSWER_TIMEOUT_SECONDS,
  clampParams,
  minimumCosineEnabled,
  suggestedCosine,
} from './retrieval-params';
import './retrieval-settings.css';

export {
  DEFAULT_RETRIEVAL_PARAMS,
  MAX_ANSWER_TIMEOUT_SECONDS,
  MAX_CITATION_COUNT,
  MAX_TOP_K,
  MIN_ANSWER_TIMEOUT_SECONDS,
  clampParams,
  clampAnswerTimeout,
  minimumCosineEnabled,
  suggestedCosine,
} from './retrieval-params';
export async function loadRetrievalSettings(
  identity: WorkspaceIdentity,
): Promise<RetrievalSettings> {
  return call({ kind: 'retrieval_settings_get' }, 'retrieval_settings', identity);
}

export async function saveRetrievalParams(
  identity: WorkspaceIdentity,
  baseRowVersion: number,
  params: RetrievalParams,
): Promise<RetrievalSettings> {
  const mutation = await call(
    {
      kind: 'retrieval_settings_set',
      payload: {
        command_id: crypto.randomUUID(),
        request: { base_row_version: baseRowVersion, params },
      },
    },
    'retrieval_settings_mutation',
    identity,
  );
  return mutation.settings;
}

export function useRetrievalSettings(identity: WorkspaceIdentity | null) {
  const [stored, setStored] = useState<{ key: string; value: RetrievalSettings } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const live = useRef(true);
  const key = identity ? `${identity.local_instance_id}/${identity.workspace_id}` : '';
  const activeKey = useRef(key);
  activeKey.current = key;
  const request = useRef(0);
  const settings = stored?.key === key ? stored.value : null;
  const setSettings = useCallback(
    (value: RetrievalSettings) => {
      if (!live.current || activeKey.current !== key) return;
      request.current += 1;
      setStored({ key, value });
      setError(null);
      setBusy(false);
    },
    [key],
  );
  const reload = useCallback(async () => {
    const pending = ++request.current;
    if (!identity) {
      setStored(null);
      setError(null);
      setBusy(false);
      return null;
    }
    setBusy(true);
    setError(null);
    const current = () => live.current && activeKey.current === key && pending === request.current;
    try {
      const value = await loadRetrievalSettings(identity);
      if (!current()) return null;
      setStored({ key, value });
      return value;
    } catch (failure) {
      if (current()) setError(failure instanceof Error ? failure.message : '无法读取检索参数。');
      return null;
    } finally {
      if (current()) setBusy(false);
    }
  }, [key]);
  useEffect(() => {
    live.current = true;
    setStored(null);
    void reload();
    return () => {
      live.current = false;
      request.current += 1;
    };
  }, [reload]);
  return { settings, setSettings, error, busy, reload };
}

export function RetrievalParamsForm({
  value,
  onChange,
  disabled,
  model,
  dimensions,
}: {
  value: RetrievalParams;
  onChange: (next: RetrievalParams) => void;
  disabled?: boolean;
  model?: string | null;
  dimensions?: number | null;
}) {
  const { t } = useI18n();
  const suggestion = suggestedCosine(model ?? null, dimensions ?? null);
  const thresholdEnabled = minimumCosineEnabled(value.minimum_cosine);
  return (
    <div className="retrieval-params">
      <label className="retrieval-param">
        <span className="retrieval-param-copy">
          <strong>{t('检索候选数 Top-K', 'Retrieval candidates (Top-K)')}</strong>
          <small>
            {t('宽召回范围，可调 1–50，默认 8。', 'Wide recall window, 1–50, default 8.')}
          </small>
        </span>
        <input
          type="number"
          className="retrieval-param-number"
          min={1}
          max={MAX_TOP_K}
          step={1}
          disabled={disabled}
          value={value.top_k}
          onChange={(event) =>
            onChange(clampParams({ ...value, top_k: Number(event.target.value) }))
          }
        />
      </label>
      <label className="retrieval-param">
        <span className="retrieval-param-copy">
          <strong>{t('回答引用条数', 'Answer citations')}</strong>
          <small>
            {t(
              '最终交给模型引用的片段数，可调 1–20，默认 5。',
              'Passages offered to the model, 1–20, default 5.',
            )}
          </small>
        </span>
        <input
          type="number"
          className="retrieval-param-number"
          min={1}
          max={MAX_CITATION_COUNT}
          step={1}
          disabled={disabled}
          value={value.citation_count}
          onChange={(event) =>
            onChange(clampParams({ ...value, citation_count: Number(event.target.value) }))
          }
        />
      </label>
      <label className="retrieval-param">
        <span className="retrieval-param-copy">
          <strong>{t('回答总时限（秒）', 'Answer time budget (seconds)')}</strong>
          <small>
            {t(
              `留空时按供应商默认：本机模型给更长预算，远程服务更短（10–${MAX_ANSWER_TIMEOUT_SECONDS} 秒）。`,
              `Blank uses the provider default: a local model gets a longer budget, a remote service a shorter one (10–${MAX_ANSWER_TIMEOUT_SECONDS} seconds).`,
            )}
          </small>
        </span>
        <input
          type="number"
          className="retrieval-param-number"
          min={MIN_ANSWER_TIMEOUT_SECONDS}
          max={MAX_ANSWER_TIMEOUT_SECONDS}
          step={1}
          disabled={disabled}
          placeholder={t('留空＝按供应商默认', 'Blank = provider default')}
          value={value.answer_timeout_seconds ?? ''}
          onChange={(event) =>
            onChange(
              clampParams({
                ...value,
                answer_timeout_seconds:
                  event.target.value.trim() === '' ? null : Number(event.target.value),
              }),
            )
          }
        />
      </label>
      <label className="retrieval-param retrieval-param-switch">
        <span className="retrieval-param-copy">
          <strong>{t('启用语义检索（向量检索）', 'Enable semantic (vector) retrieval')}</strong>
          <small>
            {t(
              '默认开启。关闭后，检索与有据问答只走本机关键词；不会请求向量服务，也不会把文本发送到远程服务。',
              'On by default. When off, retrieval and cited answers use local keywords only: no embedding request and no text sent to a remote service.',
            )}
          </small>
        </span>
        <input
          type="checkbox"
          className="retrieval-param-checkbox"
          disabled={disabled}
          checked={value.semantic_enabled !== false}
          onChange={(event) => onChange({ ...value, semantic_enabled: event.target.checked })}
        />
      </label>
      <label className="retrieval-param retrieval-param-switch">
        <span className="retrieval-param-copy">
          <strong>{t('启用最小相关度过滤', 'Enable a minimum similarity floor')}</strong>
          <small>
            {t(
              '默认关闭；开启后只保留高于当前模型参考下限的候选。',
              'Off by default; when enabled, keeps candidates above the current profile reference floor.',
            )}
          </small>
        </span>
        <input
          type="checkbox"
          className="retrieval-param-checkbox"
          disabled={disabled}
          checked={thresholdEnabled}
          onChange={(event) =>
            onChange({ ...value, minimum_cosine: event.target.checked ? suggestion : null })
          }
        />
      </label>
      <label className="retrieval-param">
        <span className="retrieval-param-copy">
          <strong>{t('最小相关度', 'Minimum similarity')}</strong>
          <small>
            {t(
              `默认关闭；余弦分布随模型与维度变化，当前模型/维度参考下限约 ${suggestion.toFixed(2)}。`,
              `Off by default; cosine distributions vary by model and dimension, reference floor ≈ ${suggestion.toFixed(2)} for the current profile.`,
            )}
          </small>
        </span>
        <input
          type="number"
          className="retrieval-param-number"
          min={-1}
          max={1}
          step={0.01}
          disabled={disabled || !thresholdEnabled}
          value={value.minimum_cosine ?? suggestion}
          onChange={(event) =>
            onChange(clampParams({ ...value, minimum_cosine: Number(event.target.value) }))
          }
        />
      </label>
    </div>
  );
}
