import test from 'node:test';
import assert from 'node:assert/strict';
import { navigationForView, viewForNavigation } from '../src/layout/navigation-view.ts';

test('product screens preserve the legacy navigation vocabulary and add applications', () => {
  assert.equal(navigationForView('applications'), 'applications');
  assert.equal(navigationForView('settings'), 'settings');
  assert.equal(navigationForView('tags'), 'knowledge');
  assert.equal(navigationForView('knowledge'), 'knowledge');
  assert.equal(navigationForView('sources'), 'sources');
  assert.equal(navigationForView('reader'), 'sources');
  assert.equal(navigationForView('graph'), 'graph');
  assert.equal(navigationForView('notes'), 'knowledge');
  for (const view of ['search', 'answers', 'proposal', 'backup', 'models', 'diagnostics']) {
    assert.equal(navigationForView(view), 'knowledge', view);
  }
  assert.equal(navigationForView('review'), 'review');
  assert.equal(navigationForView('automation'), 'review');
  assert.equal(navigationForView('operations'), 'tasks');
  assert.equal(navigationForView('trash'), 'trash');
  for (const view of ['collections', 'views', 'canvas']) {
    assert.equal(navigationForView(view), 'topics', view);
  }
});

test('a stored entry restores the real screen and inbox mode', () => {
  assert.deepEqual(viewForNavigation('knowledge'), { view: 'knowledge', inbox: null });
  assert.deepEqual(viewForNavigation('inbox'), {
    view: 'knowledge',
    inbox: null,
    pageState: { 'knowledge.mode': 'inbox' },
  });
  assert.deepEqual(viewForNavigation('sources'), { view: 'sources', inbox: null });
  assert.deepEqual(viewForNavigation('graph'), { view: 'graph', inbox: null });
  assert.deepEqual(viewForNavigation('review'), { view: 'review', inbox: null });
  assert.deepEqual(viewForNavigation('tasks'), { view: 'operations', inbox: null });
  assert.deepEqual(viewForNavigation('trash'), { view: 'trash', inbox: null });
  assert.deepEqual(viewForNavigation('topics'), { view: 'collections', inbox: null });
  assert.deepEqual(viewForNavigation('notes'), { view: 'knowledge', inbox: null });
  assert.deepEqual(viewForNavigation('search'), { view: 'knowledge', inbox: null });
  assert.deepEqual(viewForNavigation('settings'), { view: 'settings', inbox: null });
  assert.deepEqual(viewForNavigation('applications'), { view: 'applications', inbox: null });
});

test('every page survives a store-and-restore round trip', () => {
  const pages = [
    'applications',
    'settings',
    'tags',
    'knowledge',
    'notes',
    'operations',
    'collections',
    'views',
    'canvas',
    'backup',
    'trash',
    'sources',
    'reader',
    'search',
    'models',
    'answers',
    'proposal',
    'review',
    'automation',
    'diagnostics',
  ];
  for (const page of pages) {
    const restored = viewForNavigation(navigationForView(page));
    assert.ok(pages.includes(restored.view), page);
    assert.equal(navigationForView(restored.view), navigationForView(page), page);
  }
});
