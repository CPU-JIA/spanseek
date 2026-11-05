import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';
import {
  REFERENCE_SKELETON_ROWS,
  referenceSkeletonRows,
} from '../src/editor/reference-panel-model.ts';

/** React SSR separates adjacent text nodes with `<!-- -->`; that is not copy. */
const copy = (html) => html.replace(/<!--[\s\S]*?-->/g, '');

test('a first load reserves rows instead of collapsing the panel to one line', () => {
  assert.ok(REFERENCE_SKELETON_ROWS >= 2, 'a single line cannot hold the panel height');
  assert.equal(referenceSkeletonRows({ loading: true, rows: 0 }), REFERENCE_SKELETON_ROWS);
});

test('loading more rows keeps the rows on screen and reserves nothing', () => {
  assert.equal(referenceSkeletonRows({ loading: true, rows: 12 }), 0);
  assert.equal(referenceSkeletonRows({ loading: false, rows: 0 }), 0);
  assert.equal(referenceSkeletonRows({ loading: false, rows: 12 }), 0);
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
  const module = await server.ssrLoadModule('/src/editor/reference-panel.tsx');
  return { server, Skeleton: module.ReferenceSkeleton };
};

/**
 * What the panel has to state before this change: one "正在读取引用…" line that
 * disappears as soon as the rows arrive, so the section grows by the height of a
 * page. These assertions fail against that markup.
 */
test('the placeholders keep the panel height and stay out of the reading order', async (context) => {
  const { server, Skeleton } = await openPanel(context);
  try {
    const html = copy(
      renderToStaticMarkup(createElement(Skeleton, { rows: REFERENCE_SKELETON_ROWS })),
    );
    assert.equal(
      (html.match(/class="reference-skeleton-row"/g) ?? []).length,
      REFERENCE_SKELETON_ROWS,
    );
    assert.match(html, /<div class="reference-skeleton" aria-hidden="true">/);
    assert.match(html, /<p role="status" class="reference-visually-hidden">正在读取引用…<\/p>/);

    const i18n = await server.ssrLoadModule('/src/i18n.tsx');
    const enHtml = copy(
      renderToStaticMarkup(
        createElement(
          i18n.LanguageContext.Provider,
          { value: 'en' },
          createElement(Skeleton, { rows: 1 }),
        ),
      ),
    );
    assert.match(
      enHtml,
      /<p role="status" class="reference-visually-hidden">Loading references…<\/p>/,
    );
    assert.equal((enHtml.match(/reference-skeleton-row/g) ?? []).length, 1);
  } finally {
    await server.close();
  }
});

test('no rows means no placeholders at all', async (context) => {
  const { server, Skeleton } = await openPanel(context);
  try {
    assert.equal(renderToStaticMarkup(createElement(Skeleton, { rows: 0 })), '');
  } finally {
    await server.close();
  }
});
