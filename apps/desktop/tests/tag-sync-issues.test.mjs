import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { parse } from 'parse5';
import { isolatedSsrCache } from './ssr-test-cache.mjs';

let module;
let server;
let LanguageContext;
const zh = (zh) => zh;
const en = (_, en) => en;
const permissions = { configured: true, canPush: true, canReadRemote: true };
const tag = (id, name, deleted_at = null) => ({
  id,
  display_name: name,
  description: 'A real description',
  created_at: null,
  updated_at: null,
  deleted_at,
});
const impact = () => ({
  tag: { id: 'tag-id', display_name: '本地定义' },
  description: '说明',
  row_version: 9,
  created_at: null,
  updated_at: null,
  object_count: 12,
  active_object_count: 10,
  trashed_object_count: 2,
  note_count: 4,
  source_count: 3,
  annotation_count: 2,
  collection_count: 3,
  saved_views: [
    { id: 'view-one', name: '待阅读' },
    { id: 'view-two', name: '写作资料' },
  ],
  impact_token: 'frozen-impact-token',
});
function dictionary(overrides = {}) {
  return {
    sync_object_id: 'sync-id',
    kind: 'tag',
    entity_id: 'tag-id',
    entity_title: '本地定义',
    local_tags: [],
    remote_tags: [],
    payload: { sync_kind: 'tag', tag: tag('tag-id', '本地定义') },
    local_exists: true,
    local_generation: 7,
    local_fingerprint: 'local-fingerprint',
    local_impact_token: null,
    delete_impact: null,
    dirty: true,
    remote_version: 13,
    base_version: 10,
    base_fingerprint: 'base-fingerprint',
    pending: {
      payload: { sync_kind: 'tag', tag: tag('tag-id', '远端定义') },
      reason: 'local_changes',
      baseline_fingerprint: 'pending-baseline',
      received_at: '2026-11-05T00:00:00Z',
    },
    ...overrides,
  };
}
function assignments(overrides = {}) {
  const local = [{ id: 'local-tag', display_name: '本地标签', deleted: false }];
  const remote = [{ id: 'remote-tag', display_name: null, deleted: false }];
  return dictionary({
    kind: 'tag_assignments',
    entity_id: 'note-id',
    entity_title: '阅读材料',
    payload: {
      sync_kind: 'tag_assignments',
      assignments: { object_id: 'note-id', object_kind: 'note', tag_ids: ['local-tag'] },
    },
    local_tags: local,
    remote_tags: remote,
    pending: {
      payload: {
        sync_kind: 'tag_assignments',
        assignments: { object_id: 'note-id', object_kind: 'note', tag_ids: ['remote-tag'] },
      },
      reason: 'local_changes',
      baseline_fingerprint: 'old-set',
      received_at: '2026-11-05T00:00:00Z',
    },
    ...overrides,
  });
}
function deletion() {
  const value = dictionary({ delete_impact: impact(), local_impact_token: 'frozen-impact-token' });
  value.pending.payload.tag.deleted_at = '2026-11-04T00:00:00Z';
  value.pending.reason = 'assignment_changes';
  return value;
}
const elements = (node, name) => [
  ...(node.tagName === name ? [node] : []),
  ...(node.childNodes ?? []).flatMap((child) => elements(child, name)),
];
const attribute = (node, name) => node.attrs?.find((attr) => attr.name === name)?.value;
function render(snapshot, language = 'zh-CN', onOpenView) {
  return renderToStaticMarkup(
    createElement(
      LanguageContext.Provider,
      { value: language },
      createElement(module.TagSyncComparison, { snapshot, onOpenView }),
    ),
  ).replace(/<!--[\s\S]*?-->/g, '');
}

before(async (context) => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('..', import.meta.url)),
    cacheDir: await isolatedSsrCache(context),
    logLevel: 'error',
    esbuild: { jsx: 'automatic' },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    appType: 'custom',
  });
  module = await server.ssrLoadModule('/src/portability/TagSyncIssues.tsx');
  ({ LanguageContext } = await server.ssrLoadModule('/src/i18n.tsx'));
});
after(async () => {
  await server?.close();
});

test('a decision retains the entire click-time snapshot even if a refreshed source object changes', () => {
  const source = deletion();
  source.future_material = { nested: ['must remain part of the reviewed snapshot'] };
  const expected = structuredClone(source);
  const frozen = module.freezeTagSyncDecision(source);
  source.remote_version = 99;
  source.local_fingerprint = 'new-local-fingerprint';
  source.local_impact_token = 'new-impact-token';
  source.pending.payload.tag.display_name = 'new remote name';
  source.delete_impact.saved_views[0].name = 'changed view';
  source.future_material.nested.push('later');
  assert.deepEqual(frozen, expected);
  assert.throws(() => {
    frozen.pending.payload.tag.display_name = 'mutation';
  }, TypeError);
  assert.throws(() => {
    frozen.delete_impact.saved_views.push({ id: 'later' });
  }, TypeError);
  assert.throws(() => {
    frozen.future_material.nested.push('mutation');
  }, TypeError);
});

test('viewer, disconnected and unread connection states never authorize a local push', () => {
  const snapshot = dictionary();
  assert.deepEqual(module.tagSyncAvailability(snapshot, permissions), {
    retry: true,
    local: true,
    remote: true,
    openItem: false,
  });
  assert.equal(
    module.tagSyncAvailability(snapshot, { ...permissions, canPush: false }).local,
    false,
  );
  assert.equal(
    module.tagSyncAvailability(snapshot, { ...permissions, canPush: false }).remote,
    true,
  );
  for (const configured of [false, null]) {
    const actions = module.tagSyncAvailability(snapshot, { ...permissions, configured });
    assert.equal(actions.local, false);
    assert.equal(actions.remote, false);
    assert.equal(actions.retry, false);
  }
  assert.equal(
    module.tagSyncAvailability(snapshot, { ...permissions, canReadRemote: false }).remote,
    false,
  );
});

test('missing or mismatched objects cannot offer a fictitious open or overwrite path', () => {
  const missing = assignments({
    local_exists: false,
    payload: null,
    local_fingerprint: null,
    entity_title: null,
  });
  missing.pending.reason = 'missing_object';
  const actions = module.tagSyncAvailability(missing, permissions);
  assert.equal(actions.openItem, false);
  assert.equal(actions.local, false);
  assert.equal(actions.remote, false);
  assert.match(module.tagSyncTitle(missing, zh), /尚未到达的笔记/);
  const wrong = assignments();
  wrong.pending.reason = 'wrong_object_kind';
  assert.equal(module.tagSyncAvailability(wrong, permissions).remote, false);
  assert.equal(module.tagSyncAvailability(wrong, permissions).local, false);
  assert.equal(module.tagSyncAvailability(wrong, permissions).openItem, true);
});

test('deletion requires the reviewed local impact and cannot bypass saved-view dependencies', () => {
  const snapshot = deletion();
  assert.equal(module.tagSyncAvailability(snapshot, permissions).remote, true);
  assert.equal(
    module.tagSyncAvailability({ ...snapshot, delete_impact: null }, permissions).remote,
    false,
  );
  assert.equal(
    module.tagSyncAvailability({ ...snapshot, delete_impact: undefined }, permissions).remote,
    false,
  );
  assert.equal(
    module.tagSyncAvailability({ ...snapshot, local_impact_token: null }, permissions).remote,
    false,
  );
  snapshot.pending.reason = 'view_dependency';
  assert.equal(module.tagSyncAvailability(snapshot, permissions).remote, false);
  assert.equal(module.tagSyncAvailability(snapshot, permissions).local, true);
});

test('all nine pending reasons have actionable localized explanations and only processed clean records get success', () => {
  for (const reason of [
    'local_changes',
    'name_conflict',
    'missing_object',
    'wrong_object_kind',
    'missing_tags',
    'deleted_tags',
    'view_dependency',
    'assignment_changes',
    'deleted_tag',
  ]) {
    const snapshot = dictionary();
    snapshot.pending.reason = reason;
    for (const t of [zh, en]) {
      assert.ok(module.tagSyncReason(reason, t).title.length > 0);
      assert.ok(module.tagSyncReason(reason, t).detail.length > 20);
      assert.equal(module.tagSyncOutcome(snapshot, t).tone, 'attention');
    }
  }
  assert.equal(
    module.tagSyncOutcome(dictionary({ pending: null, dirty: true }), zh).tone,
    'attention',
  );
  assert.match(
    module.tagSyncOutcome(dictionary({ pending: null, dirty: true }), en).text,
    /awaits sync confirmation/,
  );
  const clean = module.tagSyncOutcome(dictionary({ pending: null, dirty: false }), zh);
  assert.equal(clean.tone, 'success');
  assert.doesNotMatch(clean.text, /全部|已同步|同步完成/);
});

test('comparison renders actual names, descriptions and deletion markers without active markup', () => {
  const snapshot = deletion();
  snapshot.payload.tag.display_name = '<script>alert("local")</script>';
  snapshot.pending.payload.tag.description = '<img src=x onerror=alert(1)>';
  const html = render(snapshot);
  const tree = parse(html);
  assert.equal(elements(tree, 'script').length, 0);
  assert.equal(elements(tree, 'img').length, 0);
  assert.match(html, /&lt;script&gt;/);
  assert.match(html, /已删除/);
  assert.match(html, /12 个对象/);
  assert.match(html, /4 篇笔记、3 份资料、2 条摘录、3 个集合/);
  assert.match(html, /2 个保存视图/);
});

test('missing names, deleted labels and technical identifiers remain distinct and diagnostics are collapsed', () => {
  const snapshot = assignments();
  snapshot.local_tags.push({ id: 'deleted-tag-id', display_name: '旧标签', deleted: true });
  const html = render(snapshot);
  const tree = parse(html);
  assert.match(html, /尚未收到名称的标签/);
  assert.match(html, /旧标签（已删除）/);
  const details = elements(tree, 'details').find(
    (node) => attribute(node, 'class') === 'tag-sync-technical',
  );
  assert.ok(details);
  assert.equal(attribute(details, 'open'), undefined);
  assert.ok(
    elements(details, 'code').some((node) =>
      node.childNodes.some(
        (child) => child.nodeName === '#text' && child.value.includes('deleted-tag-id'),
      ),
    ),
  );
});

test('saved-view dependency names only become controls when a real navigation adapter is supplied', () => {
  const snapshot = deletion();
  snapshot.pending.reason = 'view_dependency';
  assert.equal(elements(parse(render(snapshot)), 'button').length, 0);
  const clickable = render(snapshot, 'en', () => {});
  assert.equal(elements(parse(clickable), 'button').length, 2);
  assert.match(clickable, /待阅读/);
  assert.match(clickable, /写作资料/);
  assert.match(clickable, /cannot undo deletion/);
});

test('confirmation describes the chosen frozen version, clearing assignments and irreversible deletion impact', () => {
  const source = deletion();
  const frozen = module.freezeTagSyncDecision(source);
  source.delete_impact.source_count = 999;
  const copy = module.tagSyncConfirmation(frozen, 'remote', en);
  assert.match(copy, /3 sources/);
  assert.doesNotMatch(copy, /999/);
  assert.match(copy, /no undo/);
  const empty = assignments();
  empty.remote_tags = [];
  empty.pending.payload.assignments.tag_ids = [];
  assert.match(module.tagSyncConfirmation(empty, 'remote', zh), /无标签（清空分配）/);
  assert.match(module.tagSyncConfirmation(empty, 'remote', en), /not merged automatically/);
});
