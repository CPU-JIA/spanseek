import type { StatisticCount, WorkspaceStatistics } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { diagnosticCountLabel, diagnosticIntegrityText } from './diagnostics-report';
import { estimateCost, type CostPrices } from './usage-cost';
import { formatSourceSize } from '../sources/source-size';

/**
 * One labelled count group. The key vocabulary and its labels live in
 * `diagnostics-report.ts`, which the export preview and the copyable summary read too,
 * so an inventory key can never read as two different things.
 */
function CountTable({ label, values }: { label: string; values: StatisticCount[] }) {
  const { language } = useI18n();
  return (
    <section className="diagnostics-counts">
      <h3>{label}</h3>
      <dl>
        {values.map((item) => (
          <div key={item.key}>
            <dt>{diagnosticCountLabel(item.key, language)}</dt>
            <dd>{item.count.toLocaleString(language)}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
export function WorkspaceInventory({
  statistics,
  prices = null,
}: {
  statistics: WorkspaceStatistics;
  prices?: CostPrices | null;
}) {
  const { t, language } = useI18n();
  const cost = estimateCost(statistics.model_usage_30d, {
    input: prices?.input ?? null,
    output: prices?.output ?? null,
  });
  // The reader states one original's size with the same helper, so the inventory and the
  // reading view cannot describe the same file with different units.
  const fileSize = formatSourceSize(statistics.blob_bytes);
  return (
    <>
      <p className="diagnostics-muted">
        {t(
          '以下为登记总量，包含回收站对象与历史原件。',
          'Inventory includes trashed objects and historical registered files.',
        )}
      </p>
      <div className="diagnostics-totals">
        <div>
          <span>{t('笔记修订', 'Note revisions')}</span>
          <strong>{statistics.note_revisions.toLocaleString(language)}</strong>
        </div>
        <div>
          <span>{t('原件与附件', 'Registered files')}</span>
          <strong>{statistics.blobs.toLocaleString(language)}</strong>
        </div>
        <div>
          <span>{t('已索引对象', 'Indexed objects')}</span>
          <strong>{statistics.indexed_objects.toLocaleString(language)}</strong>
        </div>
        <div>
          <span>{t('知识片段', 'Indexed passages')}</span>
          <strong>{statistics.index_chunks.toLocaleString(language)}</strong>
        </div>
        <div>
          <span>{t('登记文件大小', 'Registered file size')}</span>
          <strong>{fileSize}</strong>
        </div>
      </div>
      <div className="diagnostics-grid">
        <CountTable label={t('知识内容', 'Content')} values={statistics.objects} />
        <CountTable
          label={t('当前资料格式', 'Current source formats')}
          values={statistics.sources_by_format}
        />
        <CountTable label={t('来源任务', 'Source tasks')} values={statistics.source_jobs} />
        <CountTable
          label={t('备份与导出任务', 'Portable tasks')}
          values={statistics.portable_jobs}
        />
        <CountTable
          label={t('索引代际', 'Index generations')}
          values={statistics.index_generations}
        />
        <CountTable
          label={t('索引重建', 'Index rebuilds')}
          values={statistics.index_rebuild_jobs}
        />
        <CountTable label={t('问答', 'Answers')} values={statistics.answers} />
        <CountTable label={t('回顾', 'Review')} values={statistics.review_entries} />
      </div>
      <div className="diagnostics-grid">
        <section className="diagnostics-counts">
          <h3>{t('近 24 小时检索延迟', 'Retrieval latency (24 h)')}</h3>
          {statistics.retrieval_latency_24h.samples === 0 ? (
            <p className="diagnostics-muted">
              {t('这一时段还没有检索记录。', 'No retrieval was recorded in this window.')}
            </p>
          ) : (
            <dl>
              <div>
                <dt>{t('平均', 'Average')}</dt>
                <dd>{statistics.retrieval_latency_24h.average_ms.toLocaleString(language)} ms</dd>
              </div>
              <div>
                <dt>P95</dt>
                <dd>{statistics.retrieval_latency_24h.p95_ms.toLocaleString(language)} ms</dd>
              </div>
              <div>
                <dt>{t('样本数', 'Samples')}</dt>
                <dd>{statistics.retrieval_latency_24h.samples.toLocaleString(language)}</dd>
              </div>
            </dl>
          )}
          <p className="diagnostics-muted">
            {t(
              '本地检索耗时，不含远程网络往返。',
              'Local retrieval time, excluding the remote round trip.',
            )}
          </p>
        </section>
        <section className="diagnostics-counts">
          <h3>{t('近 30 天模型调用与 Token', 'Model calls and tokens (30 d)')}</h3>
          {statistics.model_usage_30d.calls === 0 ? (
            <p className="diagnostics-muted">
              {t('这一时段还没有模型调用。', 'No model call was recorded in this window.')}
            </p>
          ) : (
            <dl>
              <div>
                <dt>{t('调用次数', 'Calls')}</dt>
                <dd>{statistics.model_usage_30d.calls.toLocaleString(language)}</dd>
              </div>
              <div>
                <dt>{t('输入 Token', 'Input tokens')}</dt>
                <dd>{statistics.model_usage_30d.input_tokens.toLocaleString(language)}</dd>
              </div>
              <div>
                <dt>{t('输出 Token', 'Output tokens')}</dt>
                <dd>{statistics.model_usage_30d.output_tokens.toLocaleString(language)}</dd>
              </div>
              {statistics.model_usage_30d.average_duration_ms !== null && (
                <div>
                  <dt>{t('平均总时长', 'Average duration')}</dt>
                  <dd>
                    {statistics.model_usage_30d.average_duration_ms.toLocaleString(language)} ms
                  </dd>
                </div>
              )}
              {statistics.model_usage_30d.average_first_token_ms !== null && (
                <div>
                  <dt>{t('平均首 token', 'Average first token')}</dt>
                  <dd>
                    {statistics.model_usage_30d.average_first_token_ms.toLocaleString(language)} ms
                  </dd>
                </div>
              )}
              {cost !== null && (
                <div>
                  <dt>{t('估算费用（按你的计价单位）', 'Estimated cost (your pricing unit)')}</dt>
                  <dd>
                    {cost.toLocaleString(language, {
                      minimumFractionDigits: 2,
                      maximumFractionDigits: 2,
                    })}
                  </dd>
                </div>
              )}
            </dl>
          )}
          <p className="diagnostics-muted">
            {t(
              `其中 ${statistics.model_usage_30d.estimated_calls} 次为本地估算，${statistics.model_usage_30d.calls_without_usage} 次未返回用量。`,
              `${statistics.model_usage_30d.estimated_calls} estimated locally, ${statistics.model_usage_30d.calls_without_usage} without provider usage.`,
            )}
          </p>
          <p className="diagnostics-muted">
            {cost === null
              ? t(
                  '费用估算未启用：在「模型与连接」填写你自己服务商的参考单价后，这里会按实测 Token 估算。应用不假定任何价格。',
                  'Cost estimate is off: enter your own provider prices in Models & connections and this panel estimates from measured tokens. No price is assumed.',
                )
              : t(
                  '估算费用＝实测 Token × 你填写的参考单价，不是服务商账单；实际扣费以服务商为准。',
                  'Estimated cost = measured tokens × your own reference prices. It is not a provider invoice; the provider’s billing is authoritative.',
                )}
          </p>
        </section>
      </div>
      <p className="diagnostics-muted">
        {t('数据库格式版本', 'Database schema')} {statistics.schema_version} ·{' '}
        {diagnosticIntegrityText(statistics.integrity_check, language)}
      </p>
    </>
  );
}
