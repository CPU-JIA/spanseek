import type { DiagnosticSnapshot } from '@pkms/contracts';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import {
  diagnosticBundleFiles,
  diagnosticCategoryLabel,
  diagnosticEnvironment,
  diagnosticErrorGroups,
  diagnosticReportJson,
  diagnosticStatisticsRows,
  type DiagnosticRow,
} from './diagnostics-report.ts';

function Rows({ rows }: { rows: DiagnosticRow[] }) {
  return (
    <dl>
      {rows.map((row) => (
        <div key={row.label}>
          <dt>{row.label}</dt>
          <dd>{row.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/**
 * The previewed snapshot, classified.
 *
 * The panel used to be the snapshot printed as JSON, which left every question about it
 * ("how many failures, from where, and what do they mean") to the reader. It now shows the
 * same information the copied summary carries, grouped by category and component, with the
 * kernel's raw code kept next to each sentence. The whitelisted JSON stays one click away
 * for anyone comparing it with the exported files.
 */
export function DiagnosticsPreview({ snapshot }: { snapshot: DiagnosticSnapshot }) {
  const { t, language } = useI18n();
  const files = diagnosticBundleFiles(snapshot, language);
  const environment = diagnosticEnvironment(snapshot.versions, language);
  const statistics = snapshot.statistics
    ? diagnosticStatisticsRows(snapshot.statistics, language)
    : null;
  const errors = snapshot.errors ? diagnosticErrorGroups(snapshot.errors, language) : null;
  return (
    <div className="diagnostics-report">
      <p className="diagnostics-muted">
        {t(
          '这是刚才预览的内容，导出任务会把它写进诊断包；下面的 JSON 只包含这些字段。',
          'This is what was previewed and what the export writes into the bundle; the JSON below carries these fields only.',
        )}
      </p>
      <section className="diagnostics-counts">
        <h3>{t('诊断包内文件', 'Files in the bundle')}</h3>
        <dl>
          {files.map((file) => (
            <div key={file.name}>
              <dt>
                <code>{file.name}</code>
              </dt>
              <dd>{file.detail}</dd>
            </div>
          ))}
        </dl>
      </section>
      {environment && (
        <section className="diagnostics-counts">
          <h3>{diagnosticCategoryLabel('versions', language)}</h3>
          <Rows rows={environment.rows} />
          <p
            className={
              environment.notice.level === 'warn'
                ? 'diagnostics-notice diagnostics-notice-warn'
                : 'diagnostics-muted'
            }
          >
            {environment.notice.text}
          </p>
          <p className="diagnostics-muted">{environment.installNotice}</p>
        </section>
      )}
      {statistics && (
        <section className="diagnostics-counts">
          <h3>{diagnosticCategoryLabel('statistics', language)}</h3>
          <Rows rows={statistics} />
        </section>
      )}
      {errors && (
        <section className="diagnostics-errors">
          <h3>{diagnosticCategoryLabel('errors', language)}</h3>
          {!errors.length ? (
            <p className="diagnostics-muted">
              {t('这段记录里没有失败任务。', 'No failed task was recorded in this window.')}
            </p>
          ) : (
            errors.map((group) => (
              <div className="diagnostics-error-group" key={group.component}>
                <h4>
                  {group.label} · {group.count}
                </h4>
                <ul>
                  {group.entries.map((entry) => (
                    <li key={entry.id}>
                      <p>{entry.text}</p>
                      <small>
                        {entry.stageText} · {displayDate(entry.occurredAt, language)} ·{' '}
                        <code title={entry.code}>{entry.code}</code>
                      </small>
                    </li>
                  ))}
                </ul>
              </div>
            ))
          )}
        </section>
      )}
      <details className="diagnostics-json">
        <summary>
          {t('JSON（仅白名单字段，格式版本 1）', 'JSON (whitelisted fields, format version 1)')}
        </summary>
        <pre
          className="diagnostics-preview"
          tabIndex={0}
          aria-label={t('诊断内容 JSON', 'Diagnostics JSON')}
        >
          {diagnosticReportJson(snapshot)}
        </pre>
      </details>
    </div>
  );
}
