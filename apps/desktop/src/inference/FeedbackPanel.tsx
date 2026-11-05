import { useEffect, useState } from 'react';
import type { FeedbackRow, FeedbackSummary, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import {
  feedbackRatingLabel,
  feedbackReportJson,
  feedbackReportText,
  feedbackSummaryLine,
} from './feedback-report';

/**
 * The collected answer ratings: how many there are, what they say, and a copy of
 * them. The list pages through the backend cursor like every other list, and the
 * copy buttons put the same report on the clipboard instead of a file — the point
 * of this panel is to read the feedback back, and a copy is what the user can
 * paste into a review or an issue.
 */
export function FeedbackPanel({
  identity,
  generation,
}: {
  identity: WorkspaceIdentity;
  generation: number;
}) {
  const { t, language } = useI18n();
  const [open, setOpen] = useState(false);
  const [summary, setSummary] = useState<FeedbackSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<'text' | 'json' | null>(null);
  // A collapsed panel reads nothing at all; every other input is part of the key.
  const scope = open
    ? `${identity.local_instance_id}/${identity.workspace_id}/${generation}`
    : null;
  const rows = usePagedList<FeedbackRow>(scope, (cursor, size) =>
    call({ kind: 'feedback_list', payload: { limit: size, cursor } }, 'feedback', identity),
  );
  useEffect(() => {
    if (!open) return;
    let live = true;
    setError(null);
    void (async () => {
      try {
        const value = await call({ kind: 'feedback_summary' }, 'feedback_summary', identity);
        if (live) setSummary(value);
      } catch (failure) {
        if (live) setError(message(failure));
      }
    })();
    return () => {
      live = false;
    };
  }, [open, generation, identity.local_instance_id, identity.workspace_id]);
  const loaded = rows.page?.items ?? [];
  async function copy(kind: 'text' | 'json') {
    if (!summary) return;
    const text =
      kind === 'text'
        ? feedbackReportText(summary, loaded, language)
        : feedbackReportJson(summary, loaded);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(kind);
      window.setTimeout(() => setCopied((value) => (value === kind ? null : value)), 4000);
    } catch (failure) {
      setError(message(failure));
    }
  }
  return (
    <details className="feedback-report" onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>
        {t('问答反馈', 'Answer feedback')}
        {summary ? ` · ${feedbackSummaryLine(summary, language)}` : ''}
      </summary>
      {(error || rows.error) && (
        <p className="inference-error" role="alert">
          {error || rows.error}
        </p>
      )}
      {!summary && !error && <p className="muted">{t('正在读取反馈…', 'Loading feedback…')}</p>}
      {summary && (
        <>
          <p className="muted">{feedbackSummaryLine(summary, language)}</p>
          <div className="inference-controls">
            <button
              className="text-button"
              disabled={!summary.total || !loaded.length}
              onClick={() => void copy('text')}
            >
              {t('复制摘要', 'Copy summary')}
            </button>
            <button
              className="text-button"
              disabled={!summary.total || !loaded.length}
              onClick={() => void copy('json')}
            >
              {t('复制 JSON', 'Copy JSON')}
            </button>
            {copied && (
              <span className="muted" role="status">
                {copied === 'text'
                  ? t('摘要已复制。', 'Summary copied.')
                  : t('JSON 已复制。', 'JSON copied.')}
              </span>
            )}
          </div>
          <p className="muted">
            {t(
              `复制内容包含当前已加载的 ${loaded.length} 条，共 ${summary.total} 条。`,
              `The copy holds the ${loaded.length} rows loaded here, out of ${summary.total}.`,
            )}
          </p>
        </>
      )}
      {summary && !loaded.length && !rows.loading && (
        <p className="muted">{t('还没有反馈。', 'No ratings yet.')}</p>
      )}
      {loaded.map((row) => (
        <article key={row.answer_id} className="feedback-row">
          <p>
            <strong>{feedbackRatingLabel(row.rating, language)}</strong>
            {' · '}
            {row.question.trim() || t('（无问题）', '(no question)')}
          </p>
          {row.comment.trim() ? <p>{row.comment}</p> : null}
          <p className="muted">{displayDate(row.changed_at, language)}</p>
        </article>
      ))}
      <ListPagination list={rows} />
    </details>
  );
}
