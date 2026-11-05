import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

const copy = (html) => html.replace(/<!--[\s\S]*?-->/g, '');
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
    View: (await server.ssrLoadModule('/src/ui/ActionConfirmDialog.tsx')).ActionConfirmDialog,
    LanguageContext: (await server.ssrLoadModule('/src/i18n.tsx')).LanguageContext,
  };
});

after(async () => {
  await shared?.server.close();
});

function render(language, overrides = {}) {
  return copy(
    renderToStaticMarkup(
      createElement(
        shared.LanguageContext.Provider,
        { value: language },
        createElement(shared.View, {
          title: '应用这条自动化提案？',
          description: '此操作会写入或移动知识。',
          confirmLabel: '确认应用',
          onCancel: () => {},
          onConfirm: () => {},
          ...overrides,
        }),
      ),
    ),
  );
}

test('action confirmation is an alertdialog with a title, description and cancel default', () => {
  const html = render('zh-CN');
  assert.match(html, /role="alertdialog"/);
  assert.match(html, /aria-labelledby="[^"]+"/);
  assert.match(html, /aria-describedby="[^"]+"/);
  assert.match(html, /应用这条自动化提案？/);
  assert.match(html, /此操作会写入或移动知识。/);
  assert.match(html, /确认应用/);
  assert.match(html, /取消/);
  assert.equal(/<input/.test(html), false);
});

test('prompt mode renders a labelled optional input', () => {
  const html = render('zh-CN', {
    title: '取消研究任务？',
    description: '任务会停止推进；已产生的步骤、证据和审计记录都会保留。',
    confirmLabel: '确认取消任务',
    input: { label: '取消原因（可留空）', placeholder: '例如：换个问题重新开始' },
  });
  assert.match(html, /取消原因（可留空）/);
  assert.match(html, /<input[^>]*placeholder="例如：换个问题重新开始"/);
  assert.match(html, /确认取消任务/);
});

test('danger tone and English copy are applied', () => {
  const html = render('en', {
    title: 'Delete this canvas?',
    description: 'Only the canvas layout is deleted.',
    confirmLabel: 'Delete canvas',
    tone: 'danger',
  });
  assert.match(html, /is-danger/);
  assert.match(html, /Delete this canvas\?/);
  assert.match(html, /Delete canvas/);
  assert.match(html, /Cancel/);
});
