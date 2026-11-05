/**
 * What a diagnostics preview means, and the one stable form it can be carried in.
 *
 * The kernel hands the interface a `DiagnosticSnapshot` and exports the same object as
 * per-category JSON files; the interface's job is to say what is in it. Two rules shape
 * this module:
 *
 * 1. Labels come from the tables the rest of the interface already uses — the task
 *    vocabulary in `errors.ts`, the import vocabulary in `sources/source-import-failure.ts`
 *    and the inference vocabulary in `inference/inference-errors.ts`. No third copy of the
 *    code vocabulary is created here, so one code never reads as two different things.
 * 2. The copyable text and the JSON shown in the page are built field by field from a
 *    whitelist. A field a later kernel adds — a path, a stored message, a token — cannot
 *    reach the clipboard or the panel through this module, while `JSON.stringify(snapshot)`
 *    would pass it through. `format_version` records the shape, so a change is visible
 *    instead of silent.
 */

import type {
  DiagnosticCategory,
  DiagnosticError,
  DiagnosticSnapshot,
  DiagnosticVersions,
  StatisticCount,
  WorkspaceStatistics,
} from '@pkms/contracts';
import type { Language } from '../language.ts';
import { operationErrorMessage } from '../errors.ts';
import { inferenceErrorMessage } from '../inference/inference-errors.ts';
import { sourceImportFailure } from '../sources/source-import-failure.ts';
import { formatSourceSize } from '../sources/source-size.ts';
import {
  WEBVIEW2_INSTALL_NOTICE,
  webView2Compatibility,
  webView2Notice,
  type WebView2Compatibility,
  type WebView2Notice,
} from './webview-compat.ts';

type Bilingual = readonly [zh: string, en: string];
export type DiagnosticRow = { label: string; value: string };

const pick = (pair: Bilingual, language: Language) => (language === 'en' ? pair[1] : pair[0]);

const CATEGORY_LABELS: Record<DiagnosticCategory, Bilingual> = {
  versions: ['应用与运行环境版本', 'Application and runtime versions'],
  statistics: ['知识库统计', 'Workspace statistics'],
  errors: ['近期任务错误（最多 100 条）', 'Recent task errors (up to 100)'],
};

export function diagnosticCategoryLabel(
  category: DiagnosticCategory,
  language: Language = 'zh-CN',
): string {
  return pick(CATEGORY_LABELS[category] ?? CATEGORY_LABELS.versions, language);
}

/**
 * The labels for every key the kernel's inventory can report; the vocabulary is owned by
 * `crates/pkms-core/src/statistics.rs`, which refuses to group a key it does not know.
 */
const COUNT_LABELS: Record<string, Bilingual> = {
  note: ['笔记', 'Notes'],
  collection: ['主题', 'Collections'],
  source: ['资料', 'Sources'],
  annotation: ['摘录', 'Annotations'],
  text: ['文本', 'Text'],
  markdown: ['Markdown', 'Markdown'],
  pdf: ['PDF', 'PDF'],
  html: ['网页', 'Webpages'],
  docx: ['Word 文档', 'Word documents'],
  doc: ['旧版 Word 文档', 'Legacy Word documents'],
  epub: ['EPUB 电子书', 'EPUB books'],
  mobi: ['MOBI 电子书', 'MOBI books'],
  csv: ['CSV 表格', 'CSV tables'],
  xlsx: ['Excel 工作簿', 'Excel workbooks'],
  xls: ['旧版 Excel 工作簿', 'Legacy Excel workbooks'],
  pptx: ['PowerPoint 演示', 'PowerPoint decks'],
  queued: ['等待中', 'Queued'],
  running: ['处理中', 'Running'],
  succeeded: ['已完成', 'Succeeded'],
  completed: ['已完成', 'Completed'],
  ready: ['已就绪', 'Ready'],
  failed: ['失败', 'Failed'],
  canceled: ['已取消', 'Canceled'],
  timed_out: ['超时', 'Timed out'],
  no_evidence: ['无证据', 'No evidence'],
  invalid_citation: ['引用无效', 'Invalid citation'],
  evidence_changed: ['证据已变化', 'Evidence changed'],
  model_unconfigured: ['未配置模型', 'Model unconfigured'],
  pending: ['待回顾', 'Pending'],
  removed: ['已移出', 'Removed'],
  unknown_state: ['未知状态', 'Unknown state'],
};

/** An unknown key keeps its own name: a new kernel value shows up instead of reading as a label. */
export function diagnosticCountLabel(key: string, language: Language = 'zh-CN'): string {
  if (!Object.prototype.hasOwnProperty.call(COUNT_LABELS, key)) return key;
  return pick(COUNT_LABELS[key], language);
}

/** The stage vocabulary `crates/pkms-core/src/statistics.rs::diagnostic_errors` can produce. */
export function diagnosticStageText(stage: string, language: Language = 'zh-CN'): string {
  if (!Object.prototype.hasOwnProperty.call(COUNT_LABELS, stage)) return stage;
  return pick(COUNT_LABELS[stage], language);
}

/**
 * The inventory states its own integrity field instead of a fixed sentence, so a state
 * this build does not know is shown as itself rather than as "not run".
 */
export function diagnosticIntegrityText(state: string, language: Language = 'zh-CN'): string {
  if (state === 'not_run') {
    return pick(['本页未执行完整性检查。', 'This page has not run an integrity check.'], language);
  }
  return pick(
    [`完整性检查状态未识别：${state}`, `Unrecognised integrity state: ${state}`],
    language,
  );
}

export type DiagnosticComponent = 'source' | 'portable' | 'index' | 'answer' | 'other';

/** Reading order: what the user can repair in the import chain first, the catch-all last. */
export const DIAGNOSTIC_COMPONENTS: readonly DiagnosticComponent[] = [
  'source',
  'portable',
  'index',
  'answer',
  'other',
];

const COMPONENT_LABELS: Record<DiagnosticComponent, Bilingual> = {
  source: ['资料处理', 'Document processing'],
  portable: ['导出与备份', 'Exports and backups'],
  index: ['索引重建', 'Index rebuilds'],
  answer: ['问答', 'Answers'],
  other: ['其他来源', 'Other sources'],
};

/** The kernel unions exactly `source`/`portable`/`index`/`answer`; anything else is `other`. */
export function diagnosticComponent(component: string): DiagnosticComponent {
  return (DIAGNOSTIC_COMPONENTS as readonly string[]).includes(component)
    ? (component as DiagnosticComponent)
    : 'other';
}

export function diagnosticComponentLabel(component: string, language: Language = 'zh-CN'): string {
  const group = diagnosticComponent(component);
  return pick(COMPONENT_LABELS[group], language);
}

/**
 * The sentence for one stored error code, from the table that already owns that vocabulary.
 * A code no table knows keeps its raw form in the row, next to a neutral sentence, rather
 * than being dropped from the list.
 */
function diagnosticErrorText(
  component: DiagnosticComponent,
  code: string,
  language: Language,
): string {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  if (component === 'source') return sourceImportFailure(code, t);
  if (component === 'portable') return operationErrorMessage(code, language);
  if (component === 'index' || component === 'answer') return inferenceErrorMessage(code, t);
  return t(
    '这条记录来自未知来源，请对照原始代码排查。',
    'This record came from an unknown source; check the raw code.',
  );
}

export type DiagnosticErrorEntry = {
  id: string;
  code: string;
  stage: string;
  stageText: string;
  occurredAt: string;
  text: string;
};
export type DiagnosticErrorGroup = {
  component: DiagnosticComponent;
  label: string;
  count: number;
  entries: DiagnosticErrorEntry[];
};

/**
 * Bundles the stored errors by the component that produced them, in a fixed order, so the
 * list already answers "how many, and where" instead of asking the reader to count rows of
 * UUIDs. Rows inside a group keep the order the kernel returned (newest first).
 */
export function diagnosticErrorGroups(
  errors: readonly DiagnosticError[],
  language: Language = 'zh-CN',
): DiagnosticErrorGroup[] {
  const buckets = new Map<DiagnosticComponent, DiagnosticErrorEntry[]>();
  for (const error of errors) {
    const component = diagnosticComponent(error.component);
    const entry: DiagnosticErrorEntry = {
      id: String(error.id),
      code: String(error.code),
      stage: String(error.stage),
      stageText: diagnosticStageText(String(error.stage), language),
      occurredAt: String(error.occurred_at),
      text: diagnosticErrorText(component, String(error.code), language),
    };
    const bucket = buckets.get(component);
    if (bucket) bucket.push(entry);
    else buckets.set(component, [entry]);
  }
  return DIAGNOSTIC_COMPONENTS.flatMap((component) => {
    const entries = buckets.get(component);
    return entries
      ? [
          {
            component,
            label: diagnosticComponentLabel(component, language),
            count: entries.length,
            entries,
          },
        ]
      : [];
  });
}

export type DiagnosticEnvironment = {
  rows: DiagnosticRow[];
  compatibility: WebView2Compatibility;
  /** One line about the runtime the preview was taken on; `warn` below a threshold. */
  notice: WebView2Notice;
  installNotice: string;
};

/** Everything the versions category says, including whether the runtime is too old for the styles. */
export function diagnosticEnvironment(
  versions: DiagnosticVersions | null,
  language: Language = 'zh-CN',
): DiagnosticEnvironment | null {
  if (!versions) return null;
  const compatibility = webView2Compatibility(versions.webview_version);
  return {
    rows: [
      { label: pick(['应用版本', 'Application'], language), value: versions.application },
      {
        label: pick(['平台', 'Platform'], language),
        value: `${versions.platform} · ${versions.architecture}`,
      },
      {
        label: pick(['WebView2 版本', 'WebView2 version'], language),
        value: versions.webview_version ?? pick(['未回报', 'Not reported'], language),
      },
      {
        label: pick(['应用数据格式', 'App data schema'], language),
        value: String(versions.app_schema),
      },
      {
        label: pick(['知识库格式', 'Workspace schema'], language),
        value: String(versions.knowledge_schema),
      },
      {
        label: pick(['解析组件协议', 'Parse worker protocol'], language),
        value: String(versions.source_worker_protocol),
      },
      {
        label: pick(['推理协议', 'Inference protocol'], language),
        value: String(versions.inference_protocol),
      },
      {
        label: pick(['嵌入模型版本', 'Embedding model revision'], language),
        value: versions.embedding_model_revision,
      },
    ],
    compatibility,
    notice: webView2Notice(compatibility, language),
    installNotice: pick(WEBVIEW2_INSTALL_NOTICE, language),
  };
}

export type DiagnosticBundleFile = { name: string; detail: string };

/**
 * The payload files the export job will write. The names are owned by
 * `crates/pkms-core/src/diagnostics.rs::prepare_diagnostics_archive`; the job adds
 * `manifest.json` itself, which is a checksum list rather than a category.
 */
export function diagnosticBundleFiles(
  snapshot: DiagnosticSnapshot,
  language: Language = 'zh-CN',
): DiagnosticBundleFile[] {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const files: DiagnosticBundleFile[] = [];
  if (snapshot.versions)
    files.push({
      name: 'versions.json',
      detail: t('1 份版本与环境信息', '1 version and environment record'),
    });
  if (snapshot.statistics)
    files.push({
      name: 'statistics.json',
      detail: t('知识库统计与用量', 'Workspace statistics and usage'),
    });
  if (snapshot.errors) {
    const count = snapshot.errors.length;
    files.push({
      name: 'errors.json',
      detail:
        language === 'en' ? `${count} task error${count === 1 ? '' : 's'}` : `${count} 条任务错误`,
    });
  }
  return files;
}

const number = (value: number | bigint, language: Language) =>
  Number(value).toLocaleString(language);

const countRows = (values: readonly StatisticCount[], language: Language): string =>
  values
    .map(
      (entry) => `${diagnosticCountLabel(entry.key, language)} ×${number(entry.count, language)}`,
    )
    .join(' · ');

/** A compact reading of the statistics category: the same rows the copied text carries. */
export function diagnosticStatisticsRows(
  statistics: WorkspaceStatistics,
  language: Language = 'zh-CN',
): DiagnosticRow[] {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const rows: DiagnosticRow[] = [
    { label: t('数据库格式版本', 'Database schema'), value: String(statistics.schema_version) },
    {
      label: t('完整性检查', 'Integrity check'),
      value: diagnosticIntegrityText(statistics.integrity_check, language),
    },
    { label: t('笔记修订', 'Note revisions'), value: number(statistics.note_revisions, language) },
    { label: t('原件与附件', 'Registered files'), value: number(statistics.blobs, language) },
    {
      label: t('登记文件大小', 'Registered file size'),
      value: formatSourceSize(statistics.blob_bytes),
    },
    {
      label: t('已索引对象', 'Indexed objects'),
      value: number(statistics.indexed_objects, language),
    },
    { label: t('知识片段', 'Indexed passages'), value: number(statistics.index_chunks, language) },
    { label: t('知识内容', 'Content'), value: countRows(statistics.objects, language) },
    {
      label: t('当前资料格式', 'Current source formats'),
      value: countRows(statistics.sources_by_format, language),
    },
    { label: t('来源任务', 'Source tasks'), value: countRows(statistics.source_jobs, language) },
    {
      label: t('备份与导出任务', 'Portable tasks'),
      value: countRows(statistics.portable_jobs, language),
    },
    {
      label: t('索引代际', 'Index generations'),
      value: countRows(statistics.index_generations, language),
    },
    {
      label: t('索引重建', 'Index rebuilds'),
      value: countRows(statistics.index_rebuild_jobs, language),
    },
    { label: t('问答', 'Answers'), value: countRows(statistics.answers, language) },
    { label: t('回顾', 'Review'), value: countRows(statistics.review_entries, language) },
  ];
  const latency = statistics.retrieval_latency_24h;
  rows.push({
    label: t('近 24 小时检索延迟', 'Retrieval latency (24 h)'),
    value:
      latency.samples === 0
        ? t('这一时段还没有检索记录。', 'No retrieval was recorded in this window.')
        : t(
            `平均 ${number(latency.average_ms, language)} ms · P95 ${number(latency.p95_ms, language)} ms · 样本 ${number(latency.samples, language)}`,
            `Average ${number(latency.average_ms, language)} ms · P95 ${number(latency.p95_ms, language)} ms · ${number(latency.samples, language)} samples`,
          ),
  });
  const usage = statistics.model_usage_30d;
  rows.push({
    label: t('近 30 天模型调用', 'Model calls (30 d)'),
    value:
      usage.calls === 0
        ? t('这一时段还没有模型调用。', 'No model call was recorded in this window.')
        : t(
            `调用 ${number(usage.calls, language)} 次 · 输入 ${number(usage.input_tokens, language)} Token · 输出 ${number(usage.output_tokens, language)} Token`,
            `${number(usage.calls, language)} calls · ${number(usage.input_tokens, language)} input tokens · ${number(usage.output_tokens, language)} output tokens`,
          ),
  });
  if (usage.calls > 0) {
    rows.push({
      label: t('其中', 'Of which'),
      value: t(
        `本地估算 ${number(usage.estimated_calls, language)} 次 · 未返回用量 ${number(usage.calls_without_usage, language)} 次`,
        `${number(usage.estimated_calls, language)} estimated locally · ${number(usage.calls_without_usage, language)} without provider usage`,
      ),
    });
    if (usage.average_duration_ms !== null || usage.average_first_token_ms !== null) {
      const parts: string[] = [];
      if (usage.average_duration_ms !== null)
        parts.push(
          t(
            `平均总时长 ${number(usage.average_duration_ms, language)} ms`,
            `Average duration ${number(usage.average_duration_ms, language)} ms`,
          ),
        );
      if (usage.average_first_token_ms !== null)
        parts.push(
          t(
            `平均首 token ${number(usage.average_first_token_ms, language)} ms`,
            `Average first token ${number(usage.average_first_token_ms, language)} ms`,
          ),
        );
      rows.push({ label: t('响应时间', 'Response time'), value: parts.join(' · ') });
    }
  }
  return rows;
}

const REPORT_FORMAT_VERSION = 1;

/** A field the kernel stopped sending reads as null here instead of disappearing. */
const reportNumber = (value: unknown): number | null => {
  const parsed = typeof value === 'bigint' ? Number(value) : value;
  return typeof parsed === 'number' && Number.isFinite(parsed) ? parsed : null;
};
const reportText = (value: unknown): string | null => (typeof value === 'string' ? value : null);

function reportCounts(values: readonly StatisticCount[] | null | undefined) {
  return (values ?? []).map((entry) => ({
    key: String(entry.key),
    count: reportNumber(entry.count),
  }));
}

function reportStatistics(statistics: WorkspaceStatistics) {
  const usage = statistics.model_usage_30d;
  const latency = statistics.retrieval_latency_24h;
  return {
    schema_version: reportNumber(statistics.schema_version),
    integrity_check: reportText(statistics.integrity_check),
    note_revisions: reportNumber(statistics.note_revisions),
    blobs: reportNumber(statistics.blobs),
    blob_bytes: reportNumber(statistics.blob_bytes),
    indexed_objects: reportNumber(statistics.indexed_objects),
    index_chunks: reportNumber(statistics.index_chunks),
    objects: reportCounts(statistics.objects),
    sources_by_format: reportCounts(statistics.sources_by_format),
    source_jobs: reportCounts(statistics.source_jobs),
    portable_jobs: reportCounts(statistics.portable_jobs),
    index_generations: reportCounts(statistics.index_generations),
    index_rebuild_jobs: reportCounts(statistics.index_rebuild_jobs),
    answers: reportCounts(statistics.answers),
    review_entries: reportCounts(statistics.review_entries),
    retrieval_latency_24h: {
      samples: reportNumber(latency.samples),
      average_ms: reportNumber(latency.average_ms),
      p95_ms: reportNumber(latency.p95_ms),
    },
    model_usage_30d: {
      calls: reportNumber(usage.calls),
      estimated_calls: reportNumber(usage.estimated_calls),
      calls_without_usage: reportNumber(usage.calls_without_usage),
      input_tokens: reportNumber(usage.input_tokens),
      output_tokens: reportNumber(usage.output_tokens),
      timed_calls: reportNumber(usage.timed_calls),
      average_duration_ms: reportNumber(usage.average_duration_ms),
      average_first_token_ms: reportNumber(usage.average_first_token_ms),
    },
  };
}

function reportVersions(versions: DiagnosticVersions) {
  return {
    application: reportText(versions.application),
    app_schema: reportNumber(versions.app_schema),
    knowledge_schema: reportNumber(versions.knowledge_schema),
    platform: reportText(versions.platform),
    architecture: reportText(versions.architecture),
    webview_version: reportText(versions.webview_version),
    source_worker_protocol: reportNumber(versions.source_worker_protocol),
    inference_protocol: reportNumber(versions.inference_protocol),
    embedding_model_revision: reportText(versions.embedding_model_revision),
  };
}

function reportErrors(errors: readonly DiagnosticError[]) {
  return errors.map((error) => ({
    id: reportText(error.id),
    component: reportText(error.component),
    stage: reportText(error.stage),
    code: reportText(error.code),
    occurred_at: reportText(error.occurred_at),
  }));
}

/** The whitelisted shape both outputs below are built from; nothing else is ever copied. */
function diagnosticReport(snapshot: DiagnosticSnapshot) {
  return {
    format_version: REPORT_FORMAT_VERSION,
    versions: snapshot.versions ? reportVersions(snapshot.versions) : null,
    statistics: snapshot.statistics ? reportStatistics(snapshot.statistics) : null,
    errors: snapshot.errors ? reportErrors(snapshot.errors) : null,
  };
}

/** Stable, whitelisted JSON: fixed key order, nothing the snapshot may carry beyond it. */
export function diagnosticReportJson(snapshot: DiagnosticSnapshot): string {
  return JSON.stringify(diagnosticReport(snapshot), null, 2);
}

/**
 * The copyable summary. It carries the same rows the preview shows, timestamped with the
 * kernel's own UTC values so two copies of one snapshot are byte-identical.
 */
export function diagnosticReportText(
  snapshot: DiagnosticSnapshot,
  language: Language = 'zh-CN',
): string {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const lines: string[] = [
    t('跨索 spanseek 诊断摘要', '跨索 spanseek diagnostics summary'),
    `${t('格式版本', 'Format version')} ${REPORT_FORMAT_VERSION}`,
  ];
  const environment = diagnosticEnvironment(snapshot.versions, language);
  const section = (title: string, rows: DiagnosticRow[]) => {
    lines.push('', `[${title}]`);
    if (!rows.length) lines.push(t('（本次未包含）', '(not included)'));
    for (const row of rows) lines.push(`${row.label}: ${row.value}`);
  };

  lines.push('', `[${diagnosticCategoryLabel('versions', language)}]`);
  if (environment) {
    for (const row of environment.rows) lines.push(`${row.label}: ${row.value}`);
    lines.push(`${t('兼容性', 'Compatibility')}: ${environment.notice.text}`);
  } else lines.push(t('（本次未包含）', '(not included)'));

  section(
    diagnosticCategoryLabel('statistics', language),
    snapshot.statistics ? diagnosticStatisticsRows(snapshot.statistics, language) : [],
  );

  lines.push('', `[${diagnosticCategoryLabel('errors', language)}]`);
  if (!snapshot.errors) lines.push(t('（本次未包含）', '(not included)'));
  else if (!snapshot.errors.length)
    lines.push(t('没有失败任务记录。', 'No failed task was recorded.'));
  else {
    for (const group of diagnosticErrorGroups(snapshot.errors, language)) {
      lines.push(`${group.label} · ${group.count}`);
      for (const entry of group.entries) {
        lines.push(`  · ${entry.code} · ${entry.stageText} · ${entry.occurredAt}`);
        lines.push(`    ${entry.text}`);
      }
    }
  }
  return `${lines.join('\n')}\n`;
}
