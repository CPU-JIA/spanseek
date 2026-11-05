import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { parseFragment } from 'parse5';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

/** Element children only: text nodes and comments cannot carry layout. */
function elements(node) {
  return (node.childNodes ?? []).filter((child) => typeof child.tagName === 'string');
}

function classes(node) {
  const attribute = (node.attrs ?? []).find((entry) => entry.name === 'class');
  return (attribute?.value ?? '').split(/\s+/).filter(Boolean);
}

function attribute(node, name) {
  return (node.attrs ?? []).find((entry) => entry.name === name)?.value ?? null;
}

function text(node) {
  if (node.nodeName === '#text') return node.value ?? '';
  return (node.childNodes ?? []).map(text).join('');
}

/**
 * Every parameter row is one two-column row: a copy block and a single control.
 * The defect this locks: the hint used to be a third child of the label, so it
 * landed in the `auto` grid column, that column grew to the paragraph width, and
 * the row titles were squeezed into a column one character wide (the vertical
 * text on the cleaning page).
 */
function parameterRows(html) {
  const container = elements(parseFragment(html)).find((node) =>
    classes(node).includes('retrieval-params'),
  );
  assert.ok(container, 'the form renders its parameter container');
  const labels = elements(container).filter((node) => node.tagName === 'label');
  assert.ok(labels.length >= 3, `expected several parameter rows, saw ${labels.length}`);
  return labels.map((label) => {
    const children = elements(label);
    assert.equal(
      children.length,
      2,
      `a parameter row holds one copy block and one control, saw ${children.length}`,
    );
    const [copy, control] = children;
    assert.ok(classes(copy).includes('retrieval-param-copy'), 'the first child is the copy block');
    assert.equal(control.tagName, 'input', 'the second child is the control');
    const title = elements(copy).find((node) => node.tagName === 'strong');
    assert.ok(title, 'the copy block carries a visible title');
    const hints = elements(copy).filter((node) => node.tagName === 'small');
    return {
      title: text(title),
      hint: hints.map(text).join(' '),
      type: attribute(control, 'type'),
    };
  });
}

test('settings parameter rows keep their copy and control in one row', async (context) => {
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
    const retrievalModule = await server.ssrLoadModule('/src/inference/retrieval-settings.tsx');
    const cleaningModule = await server.ssrLoadModule('/src/sources/cleaning-settings.tsx');
    const render = (component, props) =>
      renderToStaticMarkup(createElement(component, { onChange: () => {}, ...props }));

    const retrieval = parameterRows(
      render(retrievalModule.RetrievalParamsForm, {
        value: retrievalModule.DEFAULT_RETRIEVAL_PARAMS,
      }),
    );
    assert.deepEqual(
      retrieval.map((row) => row.title),
      [
        '检索候选数 Top-K',
        '回答引用条数',
        '回答总时限（秒）',
        '启用语义检索（向量检索）',
        '启用最小相关度过滤',
        '最小相关度',
      ],
    );
    assert.deepEqual(
      retrieval.map((row) => row.type),
      ['number', 'number', 'number', 'checkbox', 'checkbox', 'number'],
    );
    for (const row of retrieval.filter((entry) => entry.type === 'number')) {
      assert.notEqual(row.hint.trim(), '', `"${row.title}" keeps its inline hint`);
    }

    const cleaning = parameterRows(
      render(cleaningModule.CleaningParamsForm, { value: cleaningModule.DEFAULT_CLEANING_PARAMS }),
    );
    assert.deepEqual(
      cleaning.map((row) => row.title),
      ['去除重复页眉页脚', '去除重复水印文本', '去除编码异常字符'],
    );
    assert.deepEqual(
      cleaning.map((row) => row.type),
      ['checkbox', 'checkbox', 'checkbox'],
    );
    for (const row of cleaning) {
      assert.notEqual(row.hint.trim(), '', `"${row.title}" keeps its inline hint`);
      assert.match(row.hint, /默认开启/, `"${row.title}" names its default`);
    }
    const semantic = retrieval.find((row) => row.title === '启用语义检索（向量检索）');
    assert.match(semantic?.hint ?? '', /默认开启/);
    const threshold = retrieval.find((row) => row.title === '启用最小相关度过滤');
    assert.match(threshold?.hint ?? '', /默认关闭/);
  } finally {
    await server.close();
  }
});
