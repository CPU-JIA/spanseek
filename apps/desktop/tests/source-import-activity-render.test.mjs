import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

/** React SSR separates adjacent text nodes with `<!-- -->`; that is not copy. */
const copy = (html) => html.replace(/<!--[\s\S]*?-->/g, '');

const item = (token, state, jobId) => ({ token, name: `${token}.pdf`, state, jobId });
const batchJob = (id, status, errorCode, inputToken) => ({
  id,
  status,
  error_code: errorCode,
  input: { input_token: inputToken },
});
const otherJob = (id, status, errorCode, name) => ({
  id,
  status,
  error_code: errorCode,
  input: { input_token: `outside-${id}`, original_name: name },
});

// One batch and three library records, chosen so every group the panel can show
// is exercised: a component failure, a timeout, a cancellation and a success.
const itemsA = [item('a', 'accepted', 'j-a'), item('b', 'accepted', 'j-b'), item('c', 'waiting')];
const jobsA = [
  batchJob('j-a', 'succeeded', null, 'a'),
  batchJob('j-b', 'failed', 'worker_unavailable', 'b'),
  otherJob('r1', 'failed', 'webpage_timeout', 'Old page.html'),
  otherJob('r2', 'canceled', 'canceled', 'Canceled.docx'),
  otherJob('r3', 'succeeded', null, 'Kept.md'),
];

const props = (items, jobs) => ({
  items,
  jobs,
  loaded: true,
  error: null,
  pending: false,
  requestError: null,
  importing: true,
  locked: false,
  scan: null,
  onRetry() {},
  onRetryJob() {},
  onConfirmPending() {},
  onContinue() {},
  onCancelRemaining() {},
  onDismissScan() {},
  onOpenJob() {},
});

const openPanel = async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  const module = await server.ssrLoadModule('/src/sources/SourceImportDialog.tsx');
  return { server, Activity: module.SourceImportActivity };
};

/**
 * What the panel has to state before this change: the batch only listed rows
 * (no total, no completed count, no progressbar), and the import history was one
 * flat list whose failures had to be counted by hand. These assertions fail
 * against that markup, which is what makes them a regression lock rather than a
 * description of the new layout.
 */
test('the activity panel states how far the batch got, in both languages', async (context) => {
  const { server, Activity } = await openPanel(context);
  try {
    const zhHtml = copy(renderToStaticMarkup(createElement(Activity, props(itemsA, jobsA))));
    assert.ok(zhHtml.includes('<strong>已完成 2 / 3 份</strong>'), zhHtml.slice(0, 400));
    assert.equal(
      (zhHtml.match(/role="progressbar"/g) ?? []).length,
      1,
      'the batch renders one progressbar',
    );
    assert.match(zhHtml, /aria-valuenow="67"/);
    assert.match(zhHtml, /aria-valuemax="100"/);
    assert.match(zhHtml, /aria-label="本次导入进度"/);
    assert.ok(
      zhHtml.includes('<small>新增成功 1 份 · 失败 1 份 · 等待 1 份</small>'),
      'the non-empty buckets are listed once',
    );

    const i18n = await server.ssrLoadModule('/src/i18n.tsx');
    const enHtml = copy(
      renderToStaticMarkup(
        createElement(
          i18n.LanguageContext.Provider,
          { value: 'en' },
          createElement(Activity, props(itemsA, jobsA)),
        ),
      ),
    );
    assert.ok(enHtml.includes('<strong>2 of 3 done</strong>'), enHtml.slice(0, 400));
    assert.ok(enHtml.includes('<small>1 new · 1 failed · 1 waiting</small>'));
    assert.match(enHtml, /aria-label="Import progress"/);
  } finally {
    await server.close();
  }
});

test('one batch, two causes: the panel bundles failures instead of listing them flat', async (context) => {
  const { server, Activity } = await openPanel(context);
  try {
    const html = copy(renderToStaticMarkup(createElement(Activity, props(itemsA, jobsA))));

    // This batch: one component failure out of three items.
    assert.match(html, /class="source-import-failure-summary"[^>]*>1 项未完成：安装或组件问题 ×1</);
    // The import history: a timeout and a user cancellation, counted apart.
    assert.match(
      html,
      /class="source-import-failure-summary"[^>]*>2 项未完成：处理超时 ×1 · 已取消 ×1</,
    );
    assert.ok(html.includes('<h4>处理超时 · 1</h4>'), 'the timeout record gets a heading');
    assert.ok(html.includes('<h4>已取消 · 1</h4>'), 'the cancellation gets its own heading');
    assert.ok(
      html.indexOf('<h4>处理超时 · 1</h4>') < html.indexOf('<h4>已取消 · 1</h4>'),
      'what the user must repair reads before what the user chose',
    );

    // The per-file sentence and the raw code stay reachable inside the group.
    const timeoutHeading = html.indexOf('<h4>处理超时 · 1</h4>');
    const timeoutRow = html.indexOf('Old page.html');
    const canceledHeading = html.indexOf('<h4>已取消 · 1</h4>');
    const canceledRow = html.indexOf('Canceled.docx');
    assert.ok(timeoutHeading < timeoutRow, 'the timed-out record is filed under its heading');
    assert.ok(canceledHeading < canceledRow, 'the cancelled record is filed under its heading');
    assert.ok(html.includes('网页加载超时'), 'the timeout keeps its actionable sentence');
    assert.ok(html.includes('title="webpage_timeout"'), 'the raw code stays in the title');
    assert.ok(
      html.includes('导入已取消，可重新导入。'),
      'a cancellation reads as a cancellation, not as a failure',
    );
    // Records that did not fail keep their place after the grouped failures.
    assert.ok(html.indexOf('Kept.md') > canceledRow);

    const i18n = await server.ssrLoadModule('/src/i18n.tsx');
    const enHtml = copy(
      renderToStaticMarkup(
        createElement(
          i18n.LanguageContext.Provider,
          { value: 'en' },
          createElement(Activity, props(itemsA, jobsA)),
        ),
      ),
    );
    assert.match(
      enHtml,
      /2 items did not complete: Timed out ×1 · Cancelled ×1/,
      'the bundled line is translated as a whole',
    );
    assert.ok(enHtml.includes('<h4>Timed out · 1</h4>'));
    assert.ok(enHtml.includes('<h4>Cancelled · 1</h4>'));
  } finally {
    await server.close();
  }
});

test('failed and cancelled rows expose their own retry action', async (context) => {
  const { server, Activity } = await openPanel(context);
  try {
    const html = copy(
      renderToStaticMarkup(createElement(Activity, { ...props(itemsA, jobsA), importing: false })),
    );
    assert.equal((html.match(/重试此项/g) ?? []).length, 3);
    assert.ok(!html.includes('disabled=""'), 'a settled failure is retryable');
  } finally {
    await server.close();
  }
});

test('a batch with nothing wrong shows progress without an empty failure line', async (context) => {
  const { server, Activity } = await openPanel(context);
  try {
    const items = [item('a', 'accepted', 'j-a'), item('b', 'accepted', 'j-b')];
    const jobs = [batchJob('j-a', 'succeeded', null, 'a'), batchJob('j-b', 'succeeded', null, 'b')];
    const html = copy(renderToStaticMarkup(createElement(Activity, props(items, jobs))));
    assert.ok(html.includes('<strong>已完成 2 / 2 份</strong>'));
    assert.match(html, /aria-valuenow="100"/);
    assert.ok(!html.includes('source-import-failure-summary'), 'no failure line for a clean batch');
    assert.ok(!html.includes('<h4>'), 'no group headings for a clean batch');
  } finally {
    await server.close();
  }
});
