import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DIAGNOSTIC_COMPONENTS,
  diagnosticBundleFiles,
  diagnosticCategoryLabel,
  diagnosticComponent,
  diagnosticCountLabel,
  diagnosticEnvironment,
  diagnosticErrorGroups,
  diagnosticIntegrityText,
  diagnosticReportJson,
  diagnosticReportText,
  diagnosticStageText,
  diagnosticStatisticsRows,
} from '../src/diagnostics/diagnostics-report.ts';
import {
  WEBVIEW2_THRESHOLDS,
  compareWebViewVersions,
  parseWebViewVersion,
  webView2Compatibility,
  webView2Notice,
} from '../src/diagnostics/webview-compat.ts';

const versions = {
  application: '0.1.0',
  app_schema: 3,
  knowledge_schema: 24,
  platform: 'windows',
  architecture: 'x86_64',
  webview_version: '111.0.1661.44',
  source_worker_protocol: 3,
  inference_protocol: 2,
  embedding_model_revision: 'pkms-estimated-tokens-v1',
};

const statistics = {
  schema_version: 24,
  objects: [
    { key: 'note', count: 3 },
    { key: 'collection', count: 1 },
  ],
  note_revisions: 12,
  blobs: 4,
  blob_bytes: 1_572_864,
  sources_by_format: [{ key: 'pdf', count: 2 }],
  source_jobs: [{ key: 'failed', count: 1 }],
  portable_jobs: [{ key: 'succeeded', count: 2 }],
  index_generations: [{ key: 'ready', count: 1 }],
  indexed_objects: 3n,
  index_chunks: 120,
  index_rebuild_jobs: [{ key: 'completed', count: 1 }],
  answers: [{ key: 'completed', count: 5 }],
  review_entries: [{ key: 'pending', count: 2 }],
  integrity_check: 'not_run',
  retrieval_latency_24h: { samples: 3, average_ms: 42, p95_ms: 88 },
  model_usage_30d: {
    calls: 7,
    estimated_calls: 2,
    calls_without_usage: 1,
    input_tokens: 1200,
    output_tokens: 340,
    timed_calls: 6,
    average_duration_ms: 1500,
    average_first_token_ms: 220,
  },
};

const stored = (component, code, id, extra = {}) => ({
  component,
  id,
  stage: 'failed',
  code,
  occurred_at: '2026-10-16T11:22:33+00:00',
  ...extra,
});

test('runtime versions compare as numbers, not as text', () => {
  assert.deepEqual(parseWebViewVersion('123.0.2420.65'), [123, 0, 2420, 65]);
  assert.deepEqual(parseWebViewVersion('123.0.2420.65 '), [123, 0, 2420, 65]);
  assert.equal(parseWebViewVersion(''), null);
  assert.equal(parseWebViewVersion(null), null);
  assert.equal(parseWebViewVersion('not-a-version'), null);
  // Sorting these as strings puts 9 above 105, which is the bug this locks out.
  assert.equal(compareWebViewVersions('9.0.0.0', '105.0.0.0'), -1);
  // The runtime does not always report four parts.
  assert.equal(compareWebViewVersions('105.0', '105.0.0.0'), 0);
  assert.equal(compareWebViewVersions('105.0.1343.27', '123.0.0.0'), -1);
  assert.equal(compareWebViewVersions('131.0.2903.86', '123.0.0.0'), 1);
  assert.equal(compareWebViewVersions('nonsense', '123.0.0.0'), null);
});

test('the thresholds are ordered newest first and each one is a version', () => {
  // `webView2Notice` names `missing[0]` as the version the user has to reach, so the table
  // order is part of the message, not a detail.
  assert.deepEqual(
    WEBVIEW2_THRESHOLDS.map((threshold) => threshold.version),
    ['123.0.0.0', '111.0.0.0', '105.0.0.0', '94.0.0.0'],
  );
  for (const [index, threshold] of WEBVIEW2_THRESHOLDS.entries()) {
    assert.ok(parseWebViewVersion(threshold.version), threshold.version);
    if (index) {
      assert.equal(
        compareWebViewVersions(WEBVIEW2_THRESHOLDS[index - 1].version, threshold.version),
        1,
        `${WEBVIEW2_THRESHOLDS[index - 1].version} must be newer than ${threshold.version}`,
      );
    }
    assert.equal(typeof threshold.feature, 'string');
    assert.equal(threshold.effect.length, 2);
  }
});

test('a runtime that reports nothing is unknown, never a pass', () => {
  const compatibility = webView2Compatibility(null);
  assert.equal(compatibility.state, 'unknown');
  assert.equal(compatibility.version, null);
  assert.deepEqual(compatibility.missing, []);
  const notice = webView2Notice(compatibility, 'zh-CN');
  assert.equal(notice.level, 'info');
  assert.match(notice.text, /未回报 WebView2 版本/);
  assert.match(webView2Notice(webView2Compatibility(''), 'en').text, /reported no WebView2/);
});

test('an old runtime names the newest feature it drops and the version that restores it', () => {
  const compatibility = webView2Compatibility('111.0.1661.44');
  assert.equal(compatibility.state, 'outdated');
  assert.deepEqual(
    compatibility.missing.map((threshold) => threshold.version),
    ['123.0.0.0'],
  );
  const zh = webView2Notice(compatibility, 'zh-CN');
  assert.equal(zh.level, 'warn');
  assert.match(zh.text, /111\.0\.1661\.44/);
  assert.match(zh.text, /123\.0\.0\.0/);
  assert.match(zh.text, /field-sizing/);
  assert.match(zh.text, /输入框随内容自动增高/);
  const en = webView2Notice(compatibility, 'en');
  assert.match(en.text, /older than 123\.0\.0\.0/);
  assert.match(en.text, /growing with its text/);

  // A runtime below every threshold reports all of them, newest first.
  assert.deepEqual(
    webView2Compatibility('90.0.4430.72').missing.map((threshold) => threshold.version),
    ['123.0.0.0', '111.0.0.0', '105.0.0.0', '94.0.0.0'],
  );
  const supported = webView2Compatibility('131.0.2903.86');
  assert.equal(supported.state, 'supported');
  assert.deepEqual(supported.missing, []);
  assert.equal(webView2Notice(supported, 'zh-CN').level, 'info');
  assert.match(webView2Notice(supported, 'zh-CN').text, /都可用/);
});

test('errors are bundled by component in a fixed order, with a sentence per code', () => {
  const groups = diagnosticErrorGroups(
    [
      stored('answer', 'model_total_timeout', 'a1'),
      stored('source', 'docx_invalid_xml', 's1'),
      stored('source', 'pdf_password_required', 's2'),
      stored('portable', 'storage_full', 'p1'),
      stored('index', 'vector_open_failed', 'i1'),
      stored('somewhere_new', 'unknown_error', 'x1'),
    ],
    'zh-CN',
  );
  assert.deepEqual(
    groups.map((group) => group.component),
    ['source', 'portable', 'index', 'answer', 'other'],
  );
  assert.deepEqual(
    groups.map((group) => group.count),
    [2, 1, 1, 1, 1],
  );
  assert.deepEqual(
    groups.map((group) => group.label),
    ['资料处理', '导出与备份', '索引重建', '问答', '其他来源'],
  );
  // Rows inside a group keep the order the kernel returned.
  assert.deepEqual(
    groups[0].entries.map((entry) => entry.id),
    ['s1', 's2'],
  );
  assert.equal(groups[0].entries[0].text, '文件已损坏或不是有效的 DOCX，请检查原文件。');
  assert.equal(groups[0].entries[1].text, 'PDF 需要密码，请先解锁副本后重新导入。');
  assert.equal(groups[1].entries[0].text, '目标磁盘空间不足。请释放空间或选择其他位置后重试。');
  assert.equal(groups[2].entries[0].text, '无法打开本机向量索引。请重试，若仍失败请重建索引。');
  assert.equal(groups[3].entries[0].text, '模型请求达到总时限，请稍后重新发起。');
  assert.equal(groups[4].entries[0].text, '这条记录来自未知来源，请对照原始代码排查。');
  assert.equal(groups[0].entries[0].stageText, '失败');
  assert.equal(groups[0].entries[0].occurredAt, '2026-10-16T11:22:33+00:00');
  assert.deepEqual(diagnosticErrorGroups([], 'zh-CN'), []);
  assert.equal(
    diagnosticErrorGroups([stored('source', 'x', 's1')], 'en')[0].label,
    'Document processing',
  );
});

test('a code no table knows keeps its raw form and a neutral sentence', () => {
  const [source] = diagnosticErrorGroups([stored('source', 'brand_new_failure', 's1')], 'zh-CN');
  assert.equal(source.entries[0].code, 'brand_new_failure');
  assert.equal(source.entries[0].text, '读取资料失败，可在任务详情查看原因或重试。');
  const [index] = diagnosticErrorGroups([stored('index', 'brand_new_failure', 'i1')], 'en');
  assert.equal(index.entries[0].code, 'brand_new_failure');
  assert.equal(index.entries[0].text, 'The operation did not complete. Retry.');
});

test('the report JSON is whitelisted, so a field the interface does not know cannot leak', () => {
  const snapshot = {
    versions: { ...versions, install_path: 'C:/Users/me/AppData/Local/spanseek' },
    statistics,
    errors: [
      stored('source', 'docx_invalid_xml', 's1', {
        stored_message: '这是笔记正文片段',
        token: 'sk-secret-token',
      }),
    ],
  };
  // Before this change the page rendered `JSON.stringify(preview.snapshot, null, 2)`, which
  // passes every one of those fields straight through; that is the behaviour these
  // assertions forbid. (The contract types `indexed_objects` as bigint while IPC delivers a
  // number, so the contrast uses the number form the page really saw.)
  const before = JSON.stringify(
    { ...snapshot, statistics: { ...statistics, indexed_objects: 3 } },
    null,
    2,
  );
  assert.ok(before.includes('AppData/Local/spanseek'));
  assert.ok(before.includes('这是笔记正文片段'));
  assert.ok(before.includes('sk-secret-token'));

  const json = diagnosticReportJson(snapshot);
  assert.equal(json.includes('AppData'), false);
  assert.equal(json.includes('这是笔记正文片段'), false);
  assert.equal(json.includes('sk-secret-token'), false);
  const text = diagnosticReportText(snapshot, 'zh-CN');
  assert.equal(text.includes('AppData'), false);
  assert.equal(text.includes('这是笔记正文片段'), false);
  assert.equal(text.includes('sk-secret-token'), false);

  const parsed = JSON.parse(json);
  assert.deepEqual(Object.keys(parsed), ['format_version', 'versions', 'statistics', 'errors']);
  assert.deepEqual(Object.keys(parsed.versions), [
    'application',
    'app_schema',
    'knowledge_schema',
    'platform',
    'architecture',
    'webview_version',
    'source_worker_protocol',
    'inference_protocol',
    'embedding_model_revision',
  ]);
  assert.deepEqual(Object.keys(parsed.errors[0]), [
    'id',
    'component',
    'stage',
    'code',
    'occurred_at',
  ]);
});

test('the report JSON keeps a fixed shape: same keys, same order, null for what is gone', () => {
  assert.equal(
    diagnosticReportJson({ versions: null, statistics: null, errors: null }),
    '{\n  "format_version": 1,\n  "versions": null,\n  "statistics": null,\n  "errors": null\n}',
  );
  const parsed = JSON.parse(diagnosticReportJson({ versions: null, statistics, errors: null }));
  assert.deepEqual(Object.keys(parsed.statistics), [
    'schema_version',
    'integrity_check',
    'note_revisions',
    'blobs',
    'blob_bytes',
    'indexed_objects',
    'index_chunks',
    'objects',
    'sources_by_format',
    'source_jobs',
    'portable_jobs',
    'index_generations',
    'index_rebuild_jobs',
    'answers',
    'review_entries',
    'retrieval_latency_24h',
    'model_usage_30d',
  ]);
  // The contract types these two as bigint; JSON.stringify would throw on them untouched.
  assert.equal(parsed.statistics.indexed_objects, 3);
  assert.deepEqual(parsed.statistics.objects, [
    { key: 'note', count: 3 },
    { key: 'collection', count: 1 },
  ]);
  assert.deepEqual(Object.keys(parsed.statistics.model_usage_30d), [
    'calls',
    'estimated_calls',
    'calls_without_usage',
    'input_tokens',
    'output_tokens',
    'timed_calls',
    'average_duration_ms',
    'average_first_token_ms',
  ]);

  // A field the kernel stops sending, or sends as another type, reads as null.
  const partial = JSON.parse(
    diagnosticReportJson({
      versions: { ...versions, application: 42 },
      statistics: { ...statistics, note_revisions: undefined, objects: undefined },
      errors: null,
    }),
  );
  assert.equal(partial.versions.application, null);
  assert.equal(partial.statistics.note_revisions, null);
  assert.deepEqual(partial.statistics.objects, []);
});

test('the copyable text names every section, groups the errors and stays identical', () => {
  const snapshot = {
    versions,
    statistics,
    errors: [stored('source', 'docx_invalid_xml', 's1')],
  };
  const text = diagnosticReportText(snapshot, 'zh-CN');
  assert.match(text, /^跨索 spanseek 诊断摘要\n格式版本 1\n/);
  assert.ok(text.includes('[应用与运行环境版本]\n应用版本: 0.1.0\n'));
  assert.ok(text.includes('兼容性: 本机 WebView2 111.0.1661.44 低于'));
  assert.ok(text.includes('[知识库统计]\n数据库格式版本: 24\n'));
  assert.ok(
    text.includes(
      '[近期任务错误（最多 100 条）]\n资料处理 · 1\n  · docx_invalid_xml · 失败 · 2026-10-16T11:22:33+00:00\n    文件已损坏或不是有效的 DOCX，请检查原文件。\n',
    ),
  );
  assert.ok(text.endsWith('\n'));
  assert.equal(text, diagnosticReportText(snapshot, 'zh-CN'));

  const english = diagnosticReportText(snapshot, 'en');
  assert.ok(english.includes('[Recent task errors (up to 100)]\nDocument processing · 1\n'));
  assert.ok(english.includes('WebView2 111.0.1661.44 is older than 123.0.0.0'));

  const bare = diagnosticReportText({ versions: null, statistics: null, errors: null }, 'zh-CN');
  assert.equal((bare.match(/（本次未包含）/g) ?? []).length, 3);
  assert.ok(
    diagnosticReportText({ versions: null, statistics: null, errors: [] }, 'zh-CN').includes(
      '[近期任务错误（最多 100 条）]\n没有失败任务记录。\n',
    ),
  );
});

test('inventory keys and stages nobody labelled keep their own name', () => {
  assert.equal(diagnosticCountLabel('note', 'zh-CN'), '笔记');
  assert.equal(diagnosticCountLabel('note', 'en'), 'Notes');
  assert.equal(diagnosticCountLabel('docx', 'zh-CN'), 'Word 文档');
  assert.equal(diagnosticCountLabel('brand_new_kind', 'zh-CN'), 'brand_new_kind');
  assert.equal(diagnosticStageText('ready', 'en'), 'Ready');
  assert.equal(diagnosticStageText('unknown_state', 'zh-CN'), '未知状态');
  assert.equal(diagnosticStageText('brand_new_state', 'zh-CN'), 'brand_new_state');
  assert.equal(diagnosticIntegrityText('not_run', 'zh-CN'), '本页未执行完整性检查。');
  assert.equal(diagnosticIntegrityText('passed', 'zh-CN'), '完整性检查状态未识别：passed');
  assert.equal(diagnosticCategoryLabel('errors', 'zh-CN'), '近期任务错误（最多 100 条）');
  assert.deepEqual(DIAGNOSTIC_COMPONENTS, ['source', 'portable', 'index', 'answer', 'other']);
  assert.equal(diagnosticComponent('index'), 'index');
  assert.equal(diagnosticComponent('somewhere_new'), 'other');
});

test('the bundle file list only names the categories that were previewed', () => {
  const names = (snapshot, language = 'zh-CN') =>
    diagnosticBundleFiles(snapshot, language).map((file) => file.name);
  assert.deepEqual(names({ versions, statistics: null, errors: null }), ['versions.json']);
  assert.deepEqual(names({ versions: null, statistics, errors: null }), ['statistics.json']);
  assert.deepEqual(names({ versions: null, statistics: null, errors: [] }), ['errors.json']);
  assert.deepEqual(names({ versions, statistics, errors: [] }), [
    'versions.json',
    'statistics.json',
    'errors.json',
  ]);
  assert.deepEqual(names({ versions: null, statistics: null, errors: null }), []);
  const one = diagnosticBundleFiles(
    { versions: null, statistics: null, errors: [stored('source', 'x', 's1')] },
    'en',
  );
  assert.equal(one[0].detail, '1 task error');
  const two = diagnosticBundleFiles(
    {
      versions: null,
      statistics: null,
      errors: [stored('source', 'x', 's1'), stored('source', 'y', 's2')],
    },
    'zh-CN',
  );
  assert.equal(two[0].detail, '2 条任务错误');
});

test('the environment rows name the runtime and carry the compatibility line', () => {
  const environment = diagnosticEnvironment(versions, 'zh-CN');
  assert.deepEqual(
    environment.rows.map((row) => row.label),
    [
      '应用版本',
      '平台',
      'WebView2 版本',
      '应用数据格式',
      '知识库格式',
      '解析组件协议',
      '推理协议',
      '嵌入模型版本',
    ],
  );
  assert.equal(environment.rows[1].value, 'windows · x86_64');
  assert.equal(environment.rows[2].value, '111.0.1661.44');
  assert.equal(environment.compatibility.state, 'outdated');
  assert.equal(environment.notice.level, 'warn');
  assert.match(environment.installNotice, /联网下载/);
  assert.equal(
    diagnosticEnvironment({ ...versions, webview_version: null }, 'en').rows[2].value,
    'Not reported',
  );
  assert.equal(diagnosticEnvironment(null, 'zh-CN'), null);
});

test('the statistics rows read the numbers the export writes', () => {
  const rows = diagnosticStatisticsRows(statistics, 'zh-CN');
  const row = (label) => rows.find((entry) => entry.label === label).value;
  assert.equal(row('数据库格式版本'), '24');
  assert.equal(row('笔记修订'), '12');
  assert.equal(row('已索引对象'), '3');
  assert.equal(row('知识片段'), '120');
  assert.equal(row('登记文件大小'), '1.5 MB');
  assert.equal(row('知识内容'), '笔记 ×3 · 主题 ×1');
  assert.equal(row('当前资料格式'), 'PDF ×2');
  assert.equal(row('近 24 小时检索延迟'), '平均 42 ms · P95 88 ms · 样本 3');
  assert.match(row('近 30 天模型调用'), /调用 7 次 · 输入 1,200 Token · 输出 340 Token/);
  assert.equal(row('其中'), '本地估算 2 次 · 未返回用量 1 次');
  assert.equal(row('响应时间'), '平均总时长 1,500 ms · 平均首 token 220 ms');

  const empty = diagnosticStatisticsRows(
    {
      ...statistics,
      retrieval_latency_24h: { samples: 0, average_ms: 0, p95_ms: 0 },
      model_usage_30d: { ...statistics.model_usage_30d, calls: 0 },
    },
    'zh-CN',
  );
  const emptyRow = (label) => empty.find((entry) => entry.label === label).value;
  assert.equal(emptyRow('近 24 小时检索延迟'), '这一时段还没有检索记录。');
  assert.equal(emptyRow('近 30 天模型调用'), '这一时段还没有模型调用。');
  assert.equal(
    empty.some((entry) => entry.label === '响应时间'),
    false,
    'no response-time row without measured calls',
  );
});
