import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { referencePanelScope } from '../src/editor/reference-panel-scope.ts';

/**
 * The third tab lists notes whose frontmatter properties link to this note. It
 * describes the note, not one saved revision, so its scope key must not carry a
 * revision - and the two other lists must not read while it is showing.
 */

const identity = { local_instance_id: 'instance', workspace_id: 'workspace' };
const scope = (overrides = {}) =>
  referencePanelScope(
    overrides.identity ?? identity,
    overrides.open ?? true,
    overrides.noteId ?? 'note-1',
    overrides.revisionId === undefined ? 'revision-1' : overrides.revisionId,
    overrides.tab ?? 'backlinks',
    overrides.generation ?? 3,
  );

test('the property tab ignores the browsed revision', () => {
  const current = scope({ tab: 'relation', revisionId: 'revision-1' });
  assert.equal(scope({ tab: 'relation', revisionId: 'revision-2' }), current);
  assert.equal(scope({ tab: 'relation', revisionId: null }), current);
  assert.notEqual(current, scope({ tab: 'backlinks' }));
});

test('the property tab still belongs to one note, workspace and generation', () => {
  const base = scope({ tab: 'relation' });
  const others = [
    scope({ tab: 'relation', noteId: 'note-2' }),
    scope({ tab: 'relation', generation: 4 }),
    scope({ tab: 'relation', identity: { local_instance_id: 'other', workspace_id: 'workspace' } }),
    scope({ tab: 'relation', identity: { local_instance_id: 'instance', workspace_id: 'other' } }),
  ];
  for (const other of others) {
    assert.notEqual(other, base);
    assert.equal(typeof other, 'string');
  }
  assert.equal(new Set(others).size, others.length, 'keys stay distinct from each other too');
});

test('a collapsed panel reads nothing, whatever the tab', () => {
  assert.equal(scope({ open: false, tab: 'relation' }), null);
  assert.equal(scope({ open: false, tab: 'backlinks' }), null);
  assert.equal(scope({ open: false, tab: 'outgoing' }), null);
});

test('the panel reads the property list through the shared pager only on its tab', async () => {
  const panel = await readFile(
    fileURLToPath(new URL('../src/editor/reference-panel.tsx', import.meta.url)),
    'utf8',
  );
  assert.match(panel, /usePagedList<RelationBacklink>/);
  assert.match(panel, /kind: 'notes_relation_backlinks'/);
  assert.match(panel, /'relation_backlink_page'/);
  assert.match(panel, /t\('关联到这里', 'Linked by property'\)/);
  // The visible tab owns the request; the other list is handed a null scope.
  assert.match(panel, /activeTab === 'relation' \? null : bodyKey/);
  assert.match(panel, /activeTab === 'relation' \? relationKey : null/);
  assert.match(panel, /<ListPagination[\s\S]*?list=\{relations\}[\s\S]*?variant="inline"/);
});

test('the panel states an incomplete property list and marks trashed sources', async () => {
  const panel = await readFile(
    fileURLToPath(new URL('../src/editor/reference-panel.tsx', import.meta.url)),
    'utf8',
  );
  assert.match(panel, /reference-truncated/);
  assert.match(panel, /relation\.source_trashed && <span>\{t\('已回收', 'In Trash'\)\}<\/span>/);
  assert.match(panel, /reference-field/);
  assert.match(panel, /还没有笔记通过属性关联到这里。/);
});
