import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

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
    View: (await server.ssrLoadModule('/src/organization/KnowledgeIndexPage.tsx'))
      .KnowledgeIndexPage,
    LanguageContext: (await server.ssrLoadModule('/src/i18n.tsx')).LanguageContext,
    objectTypes: (
      await server.ssrLoadModule('/src/organization/object-types.ts')
    ).objectTypeCatalog(),
    PageNavigationContext: (await server.ssrLoadModule('/src/layout/page-navigation-state.tsx'))
      .PageNavigationContext,
  };
});

after(async () => {
  await shared?.server.close();
});

function render(language) {
  return renderToStaticMarkup(
    createElement(
      shared.LanguageContext.Provider,
      { value: language },
      createElement(
        shared.PageNavigationContext.Provider,
        { value: { state: {}, update: () => {} } },
        createElement(shared.View, {
          identity: { local_instance_id: 'local-a', workspace_id: 'logical-a' },
          objectTypes: shared.objectTypes,
          onOpenNote: () => {},
          onOpenSource: () => {},
          onManageCollections: () => {},
        }),
      ),
    ),
  );
}

test('knowledge index exposes the category rail and mixed-object entry points', () => {
  const html = render('zh-CN');
  assert.match(html, /class="page-frame knowledge-page-frame"/);
  assert.match(html, /知识索引/);
  assert.match(html, /范围与分类/);
  assert.match(html, /全部知识/);
  assert.match(html, /待整理/);
  assert.match(html, /未分类/);
  assert.match(html, /导入资料/);
  assert.match(html, /精确匹配内容/);
  assert.match(html, /模糊查找相关对象/);
  assert.match(html, /按标题或原文短语精确查找/);
  assert.match(html, /打开条目即可阅读或编辑/);
  assert.match(html, /aria-label="筛选知识类型"/);
  assert.match(html, /role="list"/);
  assert.equal((html.match(/class="page-frame-footer/g) ?? []).length, 1);
});

test('knowledge index has an English rendering and labelled search', () => {
  const html = render('en');
  assert.match(html, /Knowledge index/);
  assert.match(html, /Scope and categories/);
  assert.match(html, /All knowledge/);
  assert.match(html, /Find notes and sources by title or exact phrase/);
});
