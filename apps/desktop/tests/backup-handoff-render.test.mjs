import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

test('backup page exposes cross-device handoff actions', async (context) => {
  const server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  context.after(() => server.close());
  const { BackupPage } = await server.ssrLoadModule('/src/portability/BackupPage.tsx');
  const { PageNavigationContext } = await server.ssrLoadModule(
    '/src/layout/page-navigation-state.tsx',
  );
  const html = renderToStaticMarkup(
    createElement(
      PageNavigationContext.Provider,
      { value: { state: {}, update: () => {} } },
      createElement(BackupPage, {
        identity: { local_instance_id: 'instance-1', workspace_id: 'workspace-1' },
        withSavedWorkspace: async (_identity, action) => action(),
        onTasks: () => {},
        onHandoffImported: () => {},
        onConfigurationChanged: async () => {},
        onOpenTagSyncItem: () => {},
        onOpenTags: () => {},
        onOpenView: () => {},
      }),
    ),
  );
  assert.match(html, /跨设备续读/);
  assert.match(html, /导出续读包/);
  assert.match(html, /导入续读包/);
});
