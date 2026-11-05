import test from 'node:test';
import assert from 'node:assert/strict';
import { productSection } from '../src/layout/product-navigation.ts';

test('primary entry context distinguishes the inbox, library, graph and review', () => {
  assert.equal(productSection('knowledge'), 'knowledge');
  assert.equal(productSection('knowledge', true), 'inbox');
  assert.equal(productSection('sources'), 'sources');
  assert.equal(productSection('canvas'), 'graph');
  assert.equal(productSection('review'), 'review');
  assert.equal(productSection('applications'), 'answers');
  assert.equal(productSection('proposal'), 'answers');
});
test('content keeps its actual entry context and direct links have a meaningful default', () => {
  for (const origin of ['knowledge', 'inbox', 'sources', 'graph', 'review', 'answers']) {
    assert.equal(productSection('notes', false, origin), origin);
    assert.equal(productSection('reader', false, origin), origin);
  }
  assert.equal(productSection('notes'), 'knowledge');
  assert.equal(productSection('reader'), 'sources');
  assert.equal(productSection('sources', false, 'graph'), 'sources');
});
test('tools do not select an unrelated primary content entry', () => {
  for (const view of [
    'settings',
    'models',
    'diagnostics',
    'backup',
    'automation',
    'operations',
    'trash',
    'voice',
  ]) {
    assert.equal(productSection(view), 'tools', view);
  }
});
