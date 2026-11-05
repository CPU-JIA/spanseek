import { Gauge, RefreshCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { ModelUsageStats, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { estimateCost, type CostPrices } from '../diagnostics/usage-cost';
import { loadRetrievalSettings } from './retrieval-settings';
import './ai-usage.css';

export type AiUsageState =
  | { kind: 'idle' }
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; usage: ModelUsageStats; prices: CostPrices | null; priceError: string | null };

export function AiUsageSummaryView({
  usage,
  prices = null,
  loading = false,
  error = null,
  priceError = null,
  onReload,
}: {
  usage: ModelUsageStats | null;
  prices?: CostPrices | null;
  loading?: boolean;
  error?: string | null;
  priceError?: string | null;
  onReload?: () => void;
}) {
  const { t, language } = useI18n();
  if (loading)
    return <p role="status">{t('正在读取近 30 天用量…', 'Loading the last 30 days of usage…')}</p>;
  if (error) {
    return (
      <div className="ai-usage-error" role="alert">
        <p>{error}</p>
        {onReload && (
          <button type="button" className="text-button" onClick={onReload}>
            <RefreshCw size={13} aria-hidden="true" />
            {t('重试', 'Retry')}
          </button>
        )}
      </div>
    );
  }
  if (!usage) return null;

  const cost = estimateCost(usage, {
    input: prices?.input ?? null,
    output: prices?.output ?? null,
  });
  const hasPrices = prices?.input != null || prices?.output != null;
  const number = (value: number) => value.toLocaleString(language);
  const totalTokens = usage.input_tokens + usage.output_tokens;

  if (usage.calls === 0) {
    return (
      <div className="ai-usage-body">
        <p className="ai-usage-empty">
          {t(
            '近 30 天还没有模型调用。产生回答或对话后，这里会显示真实 Token 与可选费用估算。',
            'No model calls in the last 30 days. Real token counts and an optional cost estimate will appear here after you use chat or cited answers.',
          )}
        </p>
        <p className="ai-usage-note">
          {t(
            '应用不预设价格；费用只按你在模型与连接中填写的参考单价估算。',
            'No price is assumed. Cost is estimated only from the reference prices you enter in Models & connections.',
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="ai-usage-body">
      <div className="ai-usage-grid">
        <div className="ai-usage-metric">
          <span>{t('调用次数', 'Calls')}</span>
          <strong>{number(usage.calls)}</strong>
        </div>
        <div className="ai-usage-metric">
          <span>{t('输入 Token', 'Input tokens')}</span>
          <strong>{number(usage.input_tokens)}</strong>
        </div>
        <div className="ai-usage-metric">
          <span>{t('输出 Token', 'Output tokens')}</span>
          <strong>{number(usage.output_tokens)}</strong>
        </div>
        <div className="ai-usage-metric">
          <span>{t('合计 Token', 'Total tokens')}</span>
          <strong>{number(totalTokens)}</strong>
        </div>
      </div>
      <p className="ai-usage-note">
        {t(
          `其中 ${number(usage.estimated_calls)} 次为本地估算，${number(usage.calls_without_usage)} 次未返回用量。`,
          `${number(usage.estimated_calls)} estimated locally; ${number(usage.calls_without_usage)} returned no usage.`,
        )}
      </p>
      {cost !== null ? (
        <p className="ai-usage-cost">
          {t('估算费用（按你的计价单位）', 'Estimated cost (your pricing unit)')} ·{' '}
          <strong>
            {cost.toLocaleString(language, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
          </strong>
        </p>
      ) : (
        <p className="ai-usage-note">
          {t(
            '未设置参考单价，因此不显示费用估算。可在模型与连接中填写你自己的输入/输出单价。',
            'No reference prices are set, so no cost estimate is shown. Add your own input/output prices in Models & connections.',
          )}
        </p>
      )}
      {hasPrices && (
        <p className="ai-usage-note">
          {t(
            '这是按实测 Token 与你填写的单价计算的估算，不是服务商账单；实际扣费以服务商为准。',
            'This estimate uses measured tokens and your prices. It is not a provider invoice; the provider’s billing is authoritative.',
          )}
        </p>
      )}
      {priceError && (
        <p className="ai-usage-note" role="status">
          {t(
            `参考单价读取失败：${priceError}`,
            `Reference prices could not be read: ${priceError}`,
          )}
        </p>
      )}
    </div>
  );
}

export function AiUsagePanel({ identity }: { identity: WorkspaceIdentity }) {
  const { t } = useI18n();
  const [state, setState] = useState<AiUsageState>({ kind: 'idle' });
  const details = useRef<HTMLDetailsElement>(null);
  const key = `${identity.local_instance_id}/${identity.workspace_id}`;

  useEffect(() => {
    const wasOpen = details.current?.open ?? false;
    setState({ kind: 'idle' });
    if (wasOpen) void load();
  }, [key]);

  async function load() {
    setState({ kind: 'loading' });
    try {
      const statistics = await call(
        { kind: 'workspace_statistics' },
        'workspace_statistics',
        identity,
      );
      let prices: CostPrices | null = null;
      let priceError: string | null = null;
      try {
        const settings = await loadRetrievalSettings(identity);
        prices = {
          input: settings.params.cost_per_million_input ?? null,
          output: settings.params.cost_per_million_output ?? null,
        };
      } catch (failure) {
        priceError = message(failure);
      }
      setState({ kind: 'ready', usage: statistics.model_usage_30d, prices, priceError });
    } catch (failure) {
      setState({ kind: 'error', message: message(failure) });
    }
  }

  const summary =
    state.kind === 'ready'
      ? state.usage.calls === 0
        ? t('近 30 天 · 暂无调用', 'Last 30 days · no calls')
        : t(
            `近 30 天 · ${state.usage.calls} 次 · ${state.usage.input_tokens + state.usage.output_tokens} Token`,
            `Last 30 days · ${state.usage.calls} calls · ${state.usage.input_tokens + state.usage.output_tokens} tokens`,
          )
      : t('近 30 天', 'Last 30 days');

  return (
    <details
      ref={details}
      className="ai-usage"
      onToggle={(event) => {
        if (event.currentTarget.open && state.kind === 'idle') void load();
      }}
    >
      <summary>
        <Gauge size={15} aria-hidden="true" />
        <span>{t('用量与费用', 'Usage & cost')}</span>
        <small>{summary}</small>
      </summary>
      <AiUsageSummaryView
        usage={state.kind === 'ready' ? state.usage : null}
        prices={state.kind === 'ready' ? state.prices : null}
        loading={state.kind === 'loading'}
        error={state.kind === 'error' ? state.message : null}
        priceError={state.kind === 'ready' ? state.priceError : null}
        onReload={() => void load()}
      />
    </details>
  );
}
