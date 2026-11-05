import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

/** React SSR separates adjacent text nodes with `<!-- -->`; that is not copy. */
const copy = (html) => html.replace(/<!--[\s\S]*?-->/g, '');

const open = async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  return {
    server,
    Preview: (await server.ssrLoadModule('/src/diagnostics/DiagnosticsPreview.tsx'))
      .DiagnosticsPreview,
    Runtime: (await server.ssrLoadModule('/src/diagnostics/RuntimeEnvironment.tsx'))
      .RuntimeEnvironment,
    LanguageContext: (await server.ssrLoadModule('/src/i18n.tsx')).LanguageContext,
  };
};

/**
 * One server for the file: the desktop suite runs many test files at once, and one of them
 * gives a worker thread a three-second budget, so this file does not pay for four Vite
 * servers of its own.
 */
let shared = null;
before(async (context) => {
  shared = await open(context);
});
after(async () => {
  await shared?.server.close();
});

const versions = (over = {}) => ({
  application: '0.1.0',
  app_schema: 3,
  knowledge_schema: 24,
  platform: 'windows',
  architecture: 'x86_64',
  webview_version: '111.0.1661.44',
  source_worker_protocol: 3,
  inference_protocol: 2,
  embedding_model_revision: 'pkms-estimated-tokens-v1',
  ...over,
});

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
  indexed_objects: 3,
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

const snapshot = {
  versions: versions(),
  statistics,
  errors: [
    stored('source', 'docx_invalid_xml', 's1', { stored_message: '这是笔记正文片段' }),
    stored('portable', 'storage_full', 'p1'),
    stored('index', 'vector_open_failed', 'i1'),
    stored('answer', 'model_total_timeout', 'a1'),
    stored('somewhere_new', 'unknown_error', 'x1'),
  ],
};

/**
 * What the export section rendered before this change: `JSON.stringify(preview.snapshot)` in
 * one `<pre>`. Nothing in it is labelled, and every field the snapshot happens to carry is on
 * screen. The assertions below fail against that markup, which is what makes them a
 * regression lock rather than a description of the new layout.
 */
const beforeChange = (value) => `<pre>${JSON.stringify(value, null, 2)}</pre>`;

test('the preview classifies the snapshot instead of printing it', () => {
  const html = copy(renderToStaticMarkup(createElement(shared.Preview, { snapshot })));
  const before = beforeChange(snapshot);
  assert.equal(before.includes('资料处理'), false, 'the old markup labels nothing');
  assert.ok(before.includes('这是笔记正文片段'), 'the old markup passes unknown fields through');

  // The bundle contents come first, then one section per selected category, in order.
  let previous = -1;
  for (const heading of [
    '<h3>诊断包内文件</h3>',
    '<h3>应用与运行环境版本</h3>',
    '<h3>知识库统计</h3>',
    '<h3>近期任务错误（最多 100 条）</h3>',
  ]) {
    const at = html.indexOf(heading);
    assert.ok(at > previous, `expected ${heading} after the previous section`);
    previous = at;
  }
  assert.ok(html.includes('<dt><code>versions.json</code></dt>'));
  assert.ok(html.includes('<dt><code>statistics.json</code></dt>'));
  assert.ok(html.includes('<dd>5 条任务错误</dd>'));

  // Failures are counted per component, in a fixed order, each with its own sentence.
  for (const heading of [
    '<h4>资料处理 · 1</h4>',
    '<h4>导出与备份 · 1</h4>',
    '<h4>索引重建 · 1</h4>',
    '<h4>问答 · 1</h4>',
    '<h4>其他来源 · 1</h4>',
  ])
    assert.ok(html.includes(heading), `expected ${heading}`);
  assert.ok(html.indexOf('<h4>资料处理 · 1</h4>') < html.indexOf('<h4>索引重建 · 1</h4>'));
  assert.ok(html.indexOf('<h4>索引重建 · 1</h4>') < html.indexOf('<h4>其他来源 · 1</h4>'));
  assert.ok(html.includes('文件已损坏或不是有效的 DOCX，请检查原文件。'));
  assert.ok(html.includes('目标磁盘空间不足。请释放空间或选择其他位置后重试。'));
  assert.ok(html.includes('无法打开本机向量索引。请重试，若仍失败请重建索引。'));
  assert.ok(html.includes('模型请求达到总时限，请稍后重新发起。'));
  assert.ok(html.includes('这条记录来自未知来源，请对照原始代码排查。'));
  // The kernel's own code stays reachable next to the sentence.
  assert.ok(html.includes('title="docx_invalid_xml"'));

  // The JSON stays one click away, and the fields the panel does not list are gone.
  const details = html.indexOf('<details class="diagnostics-json">');
  assert.ok(details > -1, 'the JSON is behind a disclosure');
  assert.ok(html.indexOf('<pre class="diagnostics-preview"') > details);
  assert.ok(html.includes('JSON（仅白名单字段，格式版本 1）'));
  assert.ok(html.includes('&quot;format_version&quot;: 1'));
  assert.equal(html.includes('这是笔记正文片段'), false);
});

test('the preview speaks English when the interface does', () => {
  const { Preview, LanguageContext } = shared;
  const html = copy(
    renderToStaticMarkup(
      createElement(
        LanguageContext.Provider,
        { value: 'en' },
        createElement(Preview, { snapshot }),
      ),
    ),
  );
  assert.ok(html.includes('<h3>Files in the bundle</h3>'));
  assert.ok(html.includes('<h3>Recent task errors (up to 100)</h3>'));
  assert.ok(html.includes('<h4>Document processing · 1</h4>'));
  assert.ok(html.includes('<h4>Other sources · 1</h4>'));
  assert.ok(html.includes('The file is damaged or is not a valid DOCX. Check the original.'));
  assert.ok(html.includes('5 task errors'));
  assert.ok(html.includes('JSON (whitelisted fields, format version 1)'));
});

test('a preview of one category says nothing about the others', () => {
  const html = copy(
    renderToStaticMarkup(
      createElement(shared.Preview, {
        snapshot: { versions: null, statistics: null, errors: snapshot.errors },
      }),
    ),
  );
  assert.ok(html.includes('<dt><code>errors.json</code></dt>'));
  assert.equal(html.includes('versions.json'), false);
  assert.equal(html.includes('statistics.json'), false);
  assert.equal(html.includes('应用与运行环境版本'), false);
  assert.equal(html.includes('知识库统计'), false);
  assert.ok(html.includes('资料处理 · 1'));

  const clean = copy(
    renderToStaticMarkup(
      createElement(shared.Preview, { snapshot: { versions: null, statistics: null, errors: [] } }),
    ),
  );
  assert.ok(clean.includes('<dd>0 条任务错误</dd>'));
  assert.ok(clean.includes('这段记录里没有失败任务。'));
});

test('the runtime panel names an outdated WebView2 and stays quiet when it is new enough', () => {
  const render = (webview) =>
    copy(
      renderToStaticMarkup(
        createElement(shared.Runtime, { versions: versions({ webview_version: webview }) }),
      ),
    );

  const outdated = render('111.0.1661.44');
  assert.ok(outdated.includes('<dt>WebView2 版本</dt>'));
  assert.ok(outdated.includes('<dd>111.0.1661.44</dd>'));
  assert.ok(outdated.includes('<dd>windows · x86_64</dd>'));
  assert.ok(outdated.includes('class="diagnostics-notice diagnostics-notice-warn"'));
  assert.ok(outdated.includes('低于界面所用的最新特性 123.0.0.0'));
  assert.ok(outdated.includes('field-sizing'));
  assert.ok(
    outdated.includes(
      '请更新 Microsoft Edge WebView2 Runtime 后重启应用；离线机器请先准备对应的 Runtime 安装包。',
    ),
  );
  assert.ok(outdated.includes('安装包不含 WebView2 运行时'));

  const current = render('131.0.2903.86');
  assert.equal(current.includes('diagnostics-notice-warn'), false);
  assert.ok(current.includes('界面所需的样式与脚本特性都可用。'));

  const silent = render(null);
  assert.ok(silent.includes('<dd>未回报</dd>'));
  assert.ok(silent.includes('本机未回报 WebView2 版本，无法判断界面特性是否完整。'));
  assert.equal(silent.includes('diagnostics-notice-warn'), false);
});
