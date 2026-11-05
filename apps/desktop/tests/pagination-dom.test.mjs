import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

/**
 * The pager is one component with two shapes. An empty list must not hide it:
 * the footer stays on screen reading "共 0 项" with both step buttons disabled,
 * and a named variant only changes the shape (footer draws the top rule, inline
 * does not). The step label is a span so a narrow pane can drop it without
 * losing the button's accessible name.
 */
let shared = null;

before(async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  shared = {
    server,
    Pagination: (await server.ssrLoadModule('/src/layout/Pagination.tsx')).Pagination,
    LanguageContext: (await server.ssrLoadModule('/src/i18n.tsx')).LanguageContext,
  };
});

after(async () => {
  await shared?.server.close();
});

function render(language, overrides = {}) {
  return renderToStaticMarkup(
    createElement(
      shared.LanguageContext.Provider,
      { value: language },
      createElement(shared.Pagination, {
        index: 1,
        size: 10,
        count: 0,
        canPrevious: false,
        canNext: false,
        onSize: () => {},
        onPrevious: () => {},
        onNext: () => {},
        ...overrides,
      }),
    ),
  );
}

test('an empty list still renders the footer: 共 0 项 with both step buttons disabled', () => {
  const html = render('zh-CN');
  assert.match(html, /class="pagination pagination--footer"/);
  assert.match(html, /共 0 项/);
  assert.match(html, /<button[^>]*aria-label="上一页"[^>]*disabled/);
  assert.match(html, /<button[^>]*aria-label="下一页"[^>]*disabled/);
  assert.equal((html.match(/disabled/g) ?? []).length, 2, 'only the two step buttons are disabled');
});

test('the inline variant keeps the same controls without the footer class', () => {
  const html = render('zh-CN', { variant: 'inline', count: 12, canPrevious: true, canNext: true });
  assert.match(html, /class="pagination pagination--inline"/);
  assert.doesNotMatch(html, /pagination--footer/);
  assert.match(html, /第 1–12 条 · 本页 12 项/);
  assert.equal((html.match(/disabled/g) ?? []).length, 0);
});

test('an empty inline pager disappears instead of adding furniture to a picker', () => {
  assert.equal(render('zh-CN', { variant: 'inline' }), '');
});

test('the step labels are spans so a narrow pane can hide them, and the aria-label stays', () => {
  const html = render('zh-CN', { count: 10, canNext: true });
  assert.equal((html.match(/class="pagination-step-label"/g) ?? []).length, 2);
  assert.match(html, /aria-label="上一页"/);
  assert.match(html, /aria-label="下一页"/);
});

test('English copy keeps the empty footer readable', () => {
  const html = render('en');
  assert.match(html, /0 items/);
  assert.match(html, /aria-label="Previous page"/);
  assert.match(html, /aria-label="Next page"/);
});
