import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import {
  FEEDBACK_REPORT_FORMAT,
  feedbackRatingLabel,
  feedbackReportJson,
  feedbackReportText,
  feedbackSummaryLine,
} from '../src/inference/feedback-report.ts';

/**
 * The feedback report is what a user copies out of the app, so its shape is a
 * contract: the counts have to be the backend's counts, the rows have to be the
 * whitelist and nothing else, and both languages have to read the same way.
 */

const row = (overrides = {}) => ({
  answer_id: '11111111-1111-4111-8111-111111111111',
  rating: 1,
  comment: '',
  question: '备份保留期由谁批准？',
  changed_at: '2026-10-16T13:05:00Z',
  ...overrides,
});

const summary = (overrides = {}) => ({
  total: 2,
  helpful: 1,
  unhelpful: 1,
  neutral: 0,
  commented: 1,
  newest_at: '2026-10-16T13:05:00Z',
  ...overrides,
});

test('a rating reads as a verdict in both languages', () => {
  assert.equal(feedbackRatingLabel(1, 'zh-CN'), '有帮助');
  assert.equal(feedbackRatingLabel(-1, 'zh-CN'), '没帮助');
  assert.equal(feedbackRatingLabel(0, 'zh-CN'), '未评价');
  assert.equal(feedbackRatingLabel(1, 'en'), 'helpful');
  assert.equal(feedbackRatingLabel(-1, 'en'), 'not helpful');
  assert.equal(feedbackRatingLabel(0, 'en'), 'no rating');
});

test('an empty report says so instead of counting zeroes', () => {
  const empty = summary({ total: 0, helpful: 0, unhelpful: 0, commented: 0, newest_at: null });
  assert.equal(feedbackSummaryLine(empty, 'zh-CN'), '还没有反馈');
  assert.equal(feedbackSummaryLine(empty, 'en'), 'No ratings yet');
});

test('the summary line carries every count a user can act on', () => {
  const zh = feedbackSummaryLine(summary(), 'zh-CN');
  assert.match(zh, /共 2 条/);
  assert.match(zh, /有帮助 1/);
  assert.match(zh, /没帮助 1/);
  assert.match(zh, /带备注 1/);
  const en = feedbackSummaryLine(summary(), 'en');
  assert.match(en, /2 collected/);
  assert.match(en, /1 helpful/);
  assert.match(en, /1 not helpful/);
  assert.match(en, /1 with a note/);
});

test('the readable report keeps the counts, the change time and each row', () => {
  const rows = [
    row({ rating: -1, comment: '答非所问', question: '第二个问题' }),
    row({ answer_id: '22222222-2222-4222-8222-222222222222', comment: '' }),
  ];
  const text = feedbackReportText(summary(), rows, 'zh-CN');
  assert.ok(text.startsWith(`spanseek 问答反馈摘要（${FEEDBACK_REPORT_FORMAT}）`));
  assert.match(text, /共 2 条 · 有帮助 1 · 没帮助 1 · 带备注 1/);
  assert.match(text, /最近一次修改：/);
  assert.match(text, /1\. \[没帮助\] 第二个问题 · .+ — 答非所问/);
  assert.match(text, /2\. \[有帮助\] 备份保留期由谁批准？/);
  assert.ok(!text.includes('未评价'), 'a zero-neutral report hides the neutral line');

  const withNeutral = feedbackReportText(summary({ neutral: 1 }), rows, 'en');
  assert.match(withNeutral, /1 without a verdict/);
  assert.match(withNeutral, /— rows —/);
});

test('a rating without a question still reads as one line', () => {
  const text = feedbackReportText(summary({ newest_at: null }), [row({ question: '' })], 'en');
  assert.match(text, /\[helpful\] \(no question\)/);
  assert.ok(!text.includes('Last change:'), 'no change time is reported as no line at all');
});

test('the JSON report is the whitelist with a fixed key order', () => {
  const json = feedbackReportJson(summary(), [row({ extra: 'must not travel' })]);
  const parsed = JSON.parse(json);
  assert.deepEqual(Object.keys(parsed), ['format', 'summary', 'rows']);
  assert.equal(parsed.format, FEEDBACK_REPORT_FORMAT);
  assert.deepEqual(Object.keys(parsed.summary), [
    'total',
    'helpful',
    'unhelpful',
    'neutral',
    'commented',
    'newest_at',
  ]);
  assert.deepEqual(Object.keys(parsed.rows[0]), [
    'answer_id',
    'rating',
    'comment',
    'question',
    'changed_at',
  ]);
  assert.equal(parsed.rows[0].extra, undefined);
  const empty = JSON.parse(feedbackReportJson(summary({ newest_at: null }), []));
  assert.equal(empty.summary.newest_at, null);
  assert.deepEqual(empty.rows, []);
});

test('the panel reads both commands through the shared pager', async () => {
  const panel = await readFile(
    fileURLToPath(new URL('../src/inference/FeedbackPanel.tsx', import.meta.url)),
    'utf8',
  );
  assert.match(panel, /usePagedList<FeedbackRow>/);
  assert.match(panel, /<ListPagination list=\{rows\}/);
  assert.match(panel, /kind: 'feedback_list'/);
  assert.match(panel, /kind: 'feedback_summary'/);
  assert.match(panel, /'feedback_summary',/);

  const page = await readFile(
    fileURLToPath(new URL('../src/inference/AnswersPage.tsx', import.meta.url)),
    'utf8',
  );
  assert.match(
    page,
    /<FeedbackPanel identity=\{identity\} generation=\{reload\} \/>/,
    'the page where feedback is given must show the report',
  );
});
