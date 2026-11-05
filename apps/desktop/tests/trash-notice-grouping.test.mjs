import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { parseFragment } from 'parse5';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

function walk(node, visit) {
  if (typeof node.tagName === 'string') visit(node);
  for (const child of node.childNodes ?? []) walk(child, visit);
}

function text(node) {
  if (node.nodeName === '#text') return node.value ?? '';
  return (node.childNodes ?? []).map(text).join('');
}

const failure = (index, code, error) => ({
  object_id: `object-${index}`,
  kind: 'note',
  title: `已删除项目 ${index}`,
  status: 'failed',
  error,
  code,
});

test('a restore batch that fails for two reasons renders two rows, not one per item', async (context) => {
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
    const notice = await server.ssrLoadModule('/src/organization/TrashActionNotice.tsx');
    const items = [
      ...Array.from({ length: 180 }, (_, index) =>
        failure(index, 'object_trashed', '项目在回收站'),
      ),
      ...Array.from({ length: 120 }, (_, index) =>
        failure(1000 + index, 'metadata_conflict', '内容已有变化'),
      ),
    ];
    const html = renderToStaticMarkup(
      createElement(notice.TrashActionNotice, {
        state: {
          busy: false,
          pending: null,
          error: null,
          code: null,
          generation: 1,
          result: { operation_id: 'op-1', kind: 'restore', total_count: 300, items },
        },
        onRetry: () => {},
        onDismiss: () => {},
      }),
    );
    const root = parseFragment(html);
    const rows = [];
    walk(root, (node) => {
      if (node.tagName === 'li') rows.push(text(node));
    });
    // Before: the notice listed every failed item, so this batch produced 300 rows.
    assert.equal(items.filter((item) => item.status === 'failed').length, 300);
    assert.equal(rows.length, 2);
    const body = text(root);
    assert.match(body, /已恢复 0 项，300 项未恢复/);
    assert.match(body, /查看未恢复项目（2 类原因）/);
    assert.match(rows[0], /180 项 · 项目在回收站/);
    assert.match(rows[0], /已删除项目 0 · 已删除项目 1 · 已删除项目 2/);
    assert.match(rows[0], /等 177 项/);
    assert.match(rows[1], /120 项 · 内容已有变化/);
  } finally {
    await server.close();
  }
});
