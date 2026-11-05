import test from 'node:test';
import assert from 'node:assert/strict';
import {
  VisitHistory,
  isPaneLayoutChange,
  navigationKey,
  mouseNavigation,
} from '../src/layout/visit-history.ts';

const pane = (id) => ({ object_id: id, kind: 'note' });
const location = (patch = {}) => ({
  view: 'notes',
  panes: [pane('A')],
  activePane: 0,
  gallery: false,
  inbox: null,
  sourceLocations: {},
  referenceLocations: {},
  pageState: {},
  ...patch,
});
const observeLocation = (history, state) =>
  history.observe(
    JSON.stringify(state),
    state,
    isPaneLayoutChange(history.current()?.state, state),
  );

test('opening, focusing and closing a duplicate split retains one visit and the source page', () => {
  const h = new VisitHistory();
  const list = location({
    view: 'knowledge',
    pageState: { page: 11, size: 10, selected: 'A', scroll: 280 },
  });
  observeLocation(h, list);
  observeLocation(h, location());
  const contentId = h.current().id;
  observeLocation(h, location({ panes: [pane('A'), pane('A')] }));
  observeLocation(h, location({ panes: [pane('A'), pane('A')], activePane: 1 }));
  observeLocation(h, location());
  assert.equal(h.current().id, contentId);
  assert.deepEqual(h.peek(-1).state, list);
  h.accept(h.peek(-1).id);
  assert.deepEqual(h.peek(1).state.panes, [pane('A')]);
});

test('opening a different second document remains navigable and focus does not erase forward history', () => {
  const h = new VisitHistory();
  const split = location({ panes: [pane('A'), pane('A')] });
  observeLocation(h, location({ view: 'knowledge' }));
  observeLocation(h, split);
  const second = location({ panes: [pane('A'), pane('B')], activePane: 1 });
  observeLocation(h, second);
  assert.deepEqual(h.peek(-1).state, split);
  const secondId = h.current().id;
  h.accept(h.peek(-1).id);
  observeLocation(h, { ...split, activePane: 1 });
  assert.equal(h.peek(1).id, secondId);
  assert.deepEqual(h.peek(1).state, second);
  h.accept(secondId);
  observeLocation(h, location());
  assert.equal(h.current().id, secondId, 'closing adds no third content visit');
});

test('a source version or reference target change is a visit even alongside a focus/layout change', () => {
  const before = location({
    panes: [pane('A'), pane('B')],
    sourceLocations: { 1: { versionId: 'v1' } },
  });
  assert.equal(isPaneLayoutChange(before, { ...before, activePane: 1 }), true);
  assert.equal(
    isPaneLayoutChange(before, {
      ...before,
      activePane: 1,
      sourceLocations: { 1: { versionId: 'v2' } },
    }),
    false,
  );
  assert.equal(
    isPaneLayoutChange(before, {
      ...before,
      activePane: 1,
      referenceLocations: { 1: { target: { target_id: 'B', target_block_id: 'new-block' } } },
    }),
    false,
  );
  const single = location({ sourceLocations: { 0: { versionId: 'v1' } } });
  assert.equal(
    isPaneLayoutChange(single, {
      ...single,
      panes: [pane('A'), pane('A')],
      sourceLocations: { ...single.sourceLocations, 1: { versionId: 'v2' } },
    }),
    false,
  );
  assert.equal(isPaneLayoutChange(single, { ...single, panes: [pane('A'), pane('B')] }), false);
});

test('back/forward retains page context and a failed navigation does not move the cursor', () => {
  const h = new VisitHistory();
  h.observe('search', { query: '鸢尾', mode: 'lexical' });
  h.observe('note:A', { note: 'A' });
  const target = h.peek(-1);
  assert.deepEqual(target.state, { query: '鸢尾', mode: 'lexical' });
  assert.equal(h.peek(1), undefined);
  assert.equal(h.peek(-1).id, target.id); // Failed flush: no accept, same target.
  assert(h.accept(target.id));
  assert.equal(h.peek(-1), undefined);
  assert.equal(h.peek(1).key, 'note:A');
});

test('changing a filter updates the visit, navigating after back discards its forward branch', () => {
  const h = new VisitHistory();
  h.observe('library', { filter: '' });
  h.observe('library', { filter: 'PDF' });
  h.observe('reader:A', {});
  assert.equal(h.peek(-1).state.filter, 'PDF');
  h.accept(h.peek(-1).id);
  assert.equal(h.peek(-1), undefined);
  h.observe('collection:B', {});
  assert.equal(h.peek(1), undefined);
  assert.equal(h.peek(-1).key, 'library');
});

test('bounded visits and separate workspace histories cannot cross instances', () => {
  const h = new VisitHistory();
  for (let i = 0; i < 110; i++) h.observe(String(i), { index: i });
  let count = 1;
  while (h.peek(-1)) {
    h.accept(h.peek(-1).id);
    count++;
  }
  assert.equal(count, 100);
  assert.equal(new VisitHistory().peek(-1), undefined);
  assert.equal(h.accept(-1), false);
});

test('only navigation keys/side buttons navigate; editor undo and IME are untouched', () => {
  const event = {
    key: 'ArrowLeft',
    altKey: true,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    isComposing: false,
  };
  assert.equal(navigationKey(event), -1);
  assert.equal(navigationKey({ ...event, key: 'ArrowRight' }), 1);
  assert.equal(navigationKey({ ...event, isComposing: true }), null);
  assert.equal(navigationKey({ ...event, keyCode: 229 }), null);
  assert.equal(navigationKey({ ...event, altKey: false }), null);
  assert.equal(navigationKey({ ...event, ctrlKey: true, key: 'z' }), null);
  assert.deepEqual([0, 1, 2, 3, 4].map(mouseNavigation), [null, null, null, -1, 1]);
});
