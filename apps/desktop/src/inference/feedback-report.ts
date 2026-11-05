import type { FeedbackRow, FeedbackSummary } from '@pkms/contracts';
import { displayDate } from '../format.ts';
import type { Language } from '../language.ts';

/**
 * What the collected answer ratings say, written once so the panel, the copied
 * summary and any later export cannot drift apart.
 *
 * The report is a whitelist on purpose: the rating, the comment, the question the
 * rating was given for and when it changed. Nothing else from the answer travels
 * with it, and nothing here reads the library directly.
 */
export const FEEDBACK_REPORT_FORMAT = 'spanseek.answer_feedback.v1';

export function feedbackRatingLabel(rating: number, language: Language): string {
  if (rating > 0) return language === 'en' ? 'helpful' : '有帮助';
  if (rating < 0) return language === 'en' ? 'not helpful' : '没帮助';
  return language === 'en' ? 'no rating' : '未评价';
}

/** One line for the collapsed state and the top of the report. */
export function feedbackSummaryLine(summary: FeedbackSummary, language: Language): string {
  if (summary.total === 0) return language === 'en' ? 'No ratings yet' : '还没有反馈';
  return language === 'en'
    ? `${summary.total} collected · ${summary.helpful} helpful · ${summary.unhelpful} not helpful · ${summary.commented} with a note`
    : `共 ${summary.total} 条 · 有帮助 ${summary.helpful} · 没帮助 ${summary.unhelpful} · 带备注 ${summary.commented}`;
}

function detailLines(rows: readonly FeedbackRow[], language: Language): string[] {
  return rows.map((row, index) => {
    const label = feedbackRatingLabel(row.rating, language);
    const when = displayDate(row.changed_at, language);
    const comment = row.comment.trim();
    const question = row.question.trim() || (language === 'en' ? '(no question)' : '（无问题）');
    if (!comment) return `${index + 1}. [${label}] ${question} · ${when}`;
    return language === 'en'
      ? `${index + 1}. [${label}] ${question} · ${when} — ${comment}`
      : `${index + 1}. [${label}] ${question} · ${when} — ${comment}`;
  });
}

/** The readable copy: counts first, then every row the panel has loaded. */
export function feedbackReportText(
  summary: FeedbackSummary,
  rows: readonly FeedbackRow[],
  language: Language,
): string {
  const header =
    language === 'en'
      ? `spanseek answer feedback (${FEEDBACK_REPORT_FORMAT})`
      : `spanseek 问答反馈摘要（${FEEDBACK_REPORT_FORMAT}）`;
  const lines = [header, feedbackSummaryLine(summary, language)];
  if (summary.neutral > 0)
    lines.push(
      language === 'en' ? `${summary.neutral} without a verdict` : `未评价 ${summary.neutral}`,
    );
  if (summary.newest_at)
    lines.push(
      language === 'en'
        ? `Last change: ${displayDate(summary.newest_at, language)}`
        : `最近一次修改：${displayDate(summary.newest_at, language)}`,
    );
  if (!rows.length) return lines.join('\n');
  lines.push(language === 'en' ? '— rows —' : '—— 明细 ——');
  lines.push(...detailLines(rows, language));
  return lines.join('\n');
}

/** The same report as JSON, with an explicit key order and no extra fields. */
export function feedbackReportJson(summary: FeedbackSummary, rows: readonly FeedbackRow[]): string {
  return JSON.stringify(
    {
      format: FEEDBACK_REPORT_FORMAT,
      summary: {
        total: summary.total,
        helpful: summary.helpful,
        unhelpful: summary.unhelpful,
        neutral: summary.neutral,
        commented: summary.commented,
        newest_at: summary.newest_at ?? null,
      },
      rows: rows.map((row) => ({
        answer_id: row.answer_id,
        rating: row.rating,
        comment: row.comment,
        question: row.question,
        changed_at: row.changed_at,
      })),
    },
    null,
    2,
  );
}
