import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';
import { projectPreview, PREVIEW_LIMITS } from '../src/gallery/preview-document.ts';

test('empty preview distinguishes a clipped prefix from a blank note and preserves empty task markers', async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  try {
    const { PreviewDocumentView } = await server.ssrLoadModule(
      '/src/gallery/PreviewDocumentView.tsx',
    );
    const render = (content) => {
      const document = projectPreview({ type: 'doc', content });
      return {
        document,
        html: renderToStaticMarkup(
          createElement(PreviewDocumentView, {
            document,
            title: 'Synthetic note',
            identity: { local_instance_id: 'instance', workspace_id: 'workspace' },
            revision: 'revision',
            imageFailed: false,
            onImageError: () => assert.fail('no image should be requested'),
          }),
        ),
      };
    };
    const blank = { type: 'paragraph', content: [] };
    const clipped = render([
      ...Array.from({ length: PREVIEW_LIMITS.blocks }, () => blank),
      { type: 'paragraph', content: [{ type: 'text', text: 'The real body begins here.' }] },
    ]);
    assert.equal(clipped.document.nodes.length, PREVIEW_LIMITS.blocks);
    assert.equal(clipped.document.truncated, true);
    assert.doesNotMatch(clipped.html, /空白笔记|The real body begins here/);
    assert.match(clipped.html, /预览未包含正文，打开笔记查看/);
    assert.match(render([blank]).html, /空白笔记/);
    for (const checked of [false, true]) {
      const task = render([
        { type: 'taskList', content: [{ type: 'taskItem', attrs: { checked }, content: [blank] }] },
      ]);
      assert.equal(task.document.empty, false);
      assert.match(task.html, new RegExp(`gallery-preview-check" data-checked="${checked}"`));
      assert.doesNotMatch(task.html, /空白笔记|<input|<button|<dialog/);
    }
  } finally {
    await server.close();
  }
});
