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
    View: (await server.ssrLoadModule('/src/inference/ai-apply-confirm.tsx'))
      .ProposalApplyConfirmDialog,
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
          noteTitle: 'G07 审阅样本',
          baseRevisionId: '39de27e7-4d63-4bc5-a2c6-664486fd1083',
          summary: { added: 1, changed: 2, removed: 1, total: 4 },
          busy: false,
          onCancel: () => {},
          onConfirm: () => {},
          ...overrides,
        }),
      ),
    ),
  );
}

test('apply confirmation names the note, base revision and exact change counts', () => {
  const html = render('zh-CN');
  assert.match(html, /确认写入这份提案？/);
  assert.match(html, /G07 审阅样本/);
  assert.match(html, /整份写入为新的可恢复修订/);
  assert.match(html, /新增 1 · 修改 2 · 移除 1 · 共 4 个变更块/);
  assert.match(html, /39de27e7-4d63-4bc5-a2c6-664486fd1083/);
  assert.match(html, /AI 提案可能有误/);
  assert.match(html, /确认写入新修订/);
});

test('apply confirmation has an English path and disables both actions while busy', () => {
  const html = render('en', { busy: true });
  assert.match(html, /Apply this proposal\?/);
  assert.match(html, /1 added · 2 changed · 1 removed · 4 changed blocks/);
  assert.match(html, /Applying…/);
  assert.equal((html.match(/disabled=""/g) ?? []).length >= 2, true);
});
