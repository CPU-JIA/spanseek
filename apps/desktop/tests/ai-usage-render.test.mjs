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
    View: (await server.ssrLoadModule('/src/inference/ai-usage.tsx')).AiUsageSummaryView,
    LanguageContext: (await server.ssrLoadModule('/src/i18n.tsx')).LanguageContext,
  };
});

after(async () => {
  await shared?.server.close();
});

const usage = {
  calls: 3,
  estimated_calls: 1,
  calls_without_usage: 1,
  input_tokens: 2_000_000,
  output_tokens: 500_000,
  timed_calls: 2,
  average_duration_ms: 1200,
  average_first_token_ms: 240,
};

function render(language, props) {
  return copy(
    renderToStaticMarkup(
      createElement(
        shared.LanguageContext.Provider,
        { value: language },
        createElement(shared.View, { usage, ...props }),
      ),
    ),
  );
}

test('usage view shows measured tokens, estimates and user-price cost', () => {
  const html = render('zh-CN', { prices: { input: 1, output: 2 } });
  assert.match(html, /调用次数/);
  assert.match(html, /2,000,000/);
  assert.match(html, /500,000/);
  assert.match(html, /估算费用/);
  assert.match(html, /3\.00/);
  assert.match(html, /其中 1 次为本地估算，1 次未返回用量/);
  assert.match(html, /不是服务商账单/);
});

test('usage view does not invent a cost when no reference price is set', () => {
  const html = render('zh-CN', { prices: { input: null, output: null } });
  assert.match(html, /调用次数/);
  assert.match(html, /未设置参考单价/);
  assert.equal(html.includes('估算费用'), false);
});

test('usage view has a dedicated empty state with zero calls', () => {
  const html = render('en', {
    usage: { ...usage, calls: 0, input_tokens: 0, output_tokens: 0 },
    prices: null,
  });
  assert.match(html, /No model calls in the last 30 days/);
  assert.equal(html.includes('Estimated cost'), false);
});
