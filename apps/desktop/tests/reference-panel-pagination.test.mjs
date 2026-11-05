import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { referencePanelScope } from '../src/editor/reference-panel-scope.ts';

/**
 * The backlink panel used to grow one list with "show more". It now reads through
 * the shared page-number pager, so what page 1 means has to be pinned down here:
 * everything that changes the rows belongs in the scope key, and a collapsed
 * panel reads nothing at all.
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

test('a collapsed panel reads nothing', () => {
  assert.equal(scope({ open: false }), null);
});

test('the same inputs give the same key, so a re-render does not restart the list', () => {
  assert.equal(scope(), scope());
});

test('every input that changes the rows changes the key', () => {
  const base = scope();
  const others = [
    scope({ noteId: 'note-2' }),
    scope({ revisionId: 'revision-2' }),
    scope({ revisionId: null }),
    scope({ tab: 'outgoing' }),
    scope({ generation: 4 }),
    scope({ identity: { local_instance_id: 'other', workspace_id: 'workspace' } }),
    scope({ identity: { local_instance_id: 'instance', workspace_id: 'other' } }),
  ];
  for (const other of others) {
    assert.notEqual(other, base);
    assert.equal(typeof other, 'string');
  }
  assert.equal(new Set(others).size, others.length, 'keys stay distinct from each other too');
});

test('the panel reads through the shared pager instead of its own append loader', async () => {
  const panel = await readFile(
    fileURLToPath(new URL('../src/editor/reference-panel.tsx', import.meta.url)),
    'utf8',
  );
  assert.match(panel, /usePagedList<BodyReference>/);
  assert.match(panel, /<ListPagination list=\{page\}/);
  assert.ok(!panel.includes('显示更多引用'), 'the append button is gone');
  assert.ok(!panel.includes('ReferencePages'), 'the panel no longer owns a loader');
  assert.ok(
    !existsSync(fileURLToPath(new URL('../src/editor/reference-pages.ts', import.meta.url))),
    'the replaced loader is deleted rather than left as dead code',
  );
});
