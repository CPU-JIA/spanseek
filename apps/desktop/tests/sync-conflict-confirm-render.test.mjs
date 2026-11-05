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
    View: (await server.ssrLoadModule('/src/portability/SyncConflictConfirmDialog.tsx'))
      .SyncConflictConfirmDialog,
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
          mode: 'local',
          localTitle: '本机标题',
          remoteTitle: '远端标题',
          localTrashed: false,
          remoteTombstone: false,
          busy: false,
          onCancel: () => {},
          onConfirm: () => {},
          ...overrides,
        }),
      ),
    ),
  );
}

test('conflict confirmation explains keep-local consequences and both sides', () => {
  const html = render('zh-CN');
  assert.match(html, /保留本机版本？/);
  assert.match(html, /本机标题/);
  assert.match(html, /远端标题/);
  assert.match(html, /其他设备下次拉取后采用它/);
  assert.match(html, /冲突不会在后台自动解决/);
  assert.match(html, /确认保留本机/);
});

test('conflict confirmation distinguishes remote deletion and save-as-new', () => {
  const remote = render('en', { mode: 'remote', remoteTombstone: true });
  assert.match(remote, /Use the remote deletion\?/);
  assert.match(remote, /deleted remotely/);
  assert.match(remote, /Unsynchronized local changes are not merged automatically/);
  const create = render('zh-CN', { mode: 'new' });
  assert.match(create, /另存为新笔记？/);
  assert.match(create, /两边的正文都会保留/);
  assert.match(create, /确认另存新笔记/);
});

test('busy conflict confirmation disables its actions', () => {
  const html = render('zh-CN', { busy: true });
  assert.match(html, /正在处理…/);
  assert.equal((html.match(/disabled=""/g) ?? []).length >= 2, true);
});
