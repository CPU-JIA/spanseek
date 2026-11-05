import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

const metadata = (id, state, tags) => ({
  id,
  kind: 'note',
  row_version: 1,
  archived: false,
  trashed: false,
  inbox_state: state,
  search_enabled: true,
  tags: tags.map((name, index) => ({
    id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, '0')}`,
    display_name: name,
    normalized_name: name,
  })),
});
const note = (id, title, state, tags, updatedAt, properties = [], propertyState = {}) => ({
  note: {
    id,
    title,
    revision_id: `${id}-revision`,
    document_schema: 3,
    preview: `${title} preview`,
    trashed: false,
    updated_at: updatedAt,
    row_version: 1,
    inbox_state: state,
    archived: false,
    search_enabled: true,
  },
  metadata: metadata(id, state, tags),
  properties,
  properties_truncated: propertyState.truncated ?? false,
  properties_error: propertyState.error ?? false,
});

test('board, calendar and timeline layouts render their distinct states', async (context) => {
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
  const { ViewLayoutView, ViewMetadataEditor } = await server.ssrLoadModule(
    '/src/organization/view-layouts.tsx',
  );
  const items = [
    note(
      '00000000-0000-4000-8000-000000000001',
      'Inbox item',
      'pending',
      ['研究'],
      '2026-10-18T09:00:00Z',
      [
        { key: 'due', value: '2026-10-18', kind: 'date', truncated: false },
        { key: 'priority', value: '3', kind: 'number', truncated: false },
      ],
      { truncated: true },
    ),
    note(
      '00000000-0000-4000-8000-000000000002',
      'Later item',
      'snoozed',
      ['写作'],
      '2026-10-17T09:00:00Z',
      [{ key: 'due', value: '2026-10-17', kind: 'date', truncated: false }],
    ),
    note(
      '00000000-0000-4000-8000-000000000003',
      'Done item',
      'processed',
      [],
      '2026-10-16T09:00:00Z',
      [],
      { error: true },
    ),
  ];
  const common = {
    items,
    selectedId: null,
    onSelect: () => {},
    onOpenNote: () => {},
    onChanged: () => {},
  };
  const config = {
    visible_properties: ['inbox_state', 'tags', 'updated_at'],
    board_group_by: 'inbox_state',
    calendar_month: '2026-10',
    custom_properties: ['due', 'priority'],
    date_property: 'due',
  };
  for (const layout of ['board', 'calendar', 'timeline']) {
    const html = renderToStaticMarkup(
      createElement(ViewLayoutView, {
        ...common,
        view: { layout, config },
      }),
    );
    assert.match(html, /Inbox item/);
    assert.match(html, /Later item/);
    assert.match(html, /Done item/);
    if (layout === 'board') {
      assert.match(html, /待整理/);
      assert.match(html, /稍后整理/);
      assert.match(html, /已整理/);
      assert.match(html, />due</);
    }
    if (layout === 'calendar') {
      assert.match(html, /按属性 due/);
      assert.match(html, /未排期/);
      assert.match(html, /未排期[\s\S]*Done item/);
    }
    if (layout === 'timeline') {
      assert.match(html, /按属性 due/);
      assert.match(html, /2026-10-18/);
      assert.match(html, /未排期[\s\S]*Done item/);
    }
  }
  const table = renderToStaticMarkup(
    createElement(ViewLayoutView, {
      ...common,
      view: { layout: 'table', config },
    }),
  );
  assert.match(table, /priority/);
  assert.match(table, /10月18日/);
  assert.match(table, /未设置/);
  assert.match(table, /YAML 错误/);
  assert.match(table, /读取失败/);

  const metadataEditor = renderToStaticMarkup(
    createElement(ViewMetadataEditor, {
      item: items[0],
      identity: { local_instance_id: 'local', workspace_id: 'workspace' },
      onChanged: () => {},
      onOpenNote: () => {},
    }),
  );
  assert.match(metadataEditor, /Frontmatter 属性/);
  assert.match(metadataEditor, /due/);
  assert.match(metadataEditor, /属性投影已达到数量或长度上限/);
});
