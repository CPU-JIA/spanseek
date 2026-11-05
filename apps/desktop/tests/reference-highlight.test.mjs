import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState, TextSelection } from '@tiptap/pm/state';
import { Decoration } from '@tiptap/pm/view';
import { history, undoDepth } from '@tiptap/pm/history';
import {
  createReferenceHighlightPlugin,
  REFERENCE_HIGHLIGHT_MS,
  referenceHighlightKey,
  showReferenceHighlight,
} from '../src/editor/reference-highlight-extension.ts';

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'inline*', attrs: { id: { default: null } } },
    heading: { group: 'block', content: 'inline*', attrs: { id: { default: null } } },
    text: { group: 'inline' },
    noteReference: {
      inline: true,
      group: 'inline',
      atom: true,
      attrs: { occurrence_id: { default: null }, label: { default: null } },
    },
    wikiEmbed: {
      group: 'block',
      atom: true,
      attrs: { id: { default: null }, occurrence_id: { default: null } },
    },
  },
});

const document = {
  type: 'doc',
  content: [
    { type: 'paragraph', attrs: { id: 'before' }, content: [{ type: 'text', text: 'Same' }] },
    { type: 'heading', attrs: { id: 'target' }, content: [{ type: 'text', text: 'Same' }] },
    {
      type: 'paragraph',
      attrs: { id: 'references' },
      content: [
        { type: 'noteReference', attrs: { occurrence_id: 'first', label: 'Same' } },
        { type: 'text', text: ' and ' },
        { type: 'noteReference', attrs: { occurrence_id: 'second', label: 'Same' } },
      ],
    },
    { type: 'wikiEmbed', attrs: { id: 'embed-block', occurrence_id: 'embed-occurrence' } },
  ],
};

function mount(context) {
  const plugin = createReferenceHighlightPlugin();
  const transactions = [];
  let lifecycle;
  // Use real PM state, mappings, decorations and history. Only the view's
  // dispatch/lifecycle boundary is adapted; this does not claim native DOM QA.
  const view = {
    state: EditorState.create({
      schema,
      doc: schema.nodeFromJSON(document),
      plugins: [history(), plugin],
    }),
    isDestroyed: false,
    dispatch(transaction) {
      assert.equal(view.isDestroyed, false);
      const previous = view.state;
      const result = previous.applyTransaction(transaction);
      view.state = result.state;
      transactions.push(...result.transactions);
      lifecycle?.update(view, previous);
    },
  };
  lifecycle = plugin.spec.view(view);
  const destroy = () => {
    if (view.isDestroyed) return;
    lifecycle.destroy();
    view.isDestroyed = true;
  };
  context.after(destroy);
  return {
    view,
    plugin,
    transactions,
    destroy,
    decorations: () => plugin.props.decorations(view.state).find(),
  };
}

function at(view, attribute, value) {
  let result;
  view.state.doc.descendants((node, position) => {
    if (node.attrs[attribute] === value) result = { node, position };
  });
  assert.ok(result, attribute + '=' + value);
  return result;
}

test('block highlights survive PM state updates and map edits without changing document or undo history', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const mounted = mount(context);
  const { view, transactions, decorations } = mounted;
  const before = view.state.doc.toJSON();
  const selection = view.state.selection;
  const target = at(view, 'id', 'target');
  const clear = showReferenceHighlight(view, { requestId: 'block', blockId: 'target' });
  assert.equal(typeof clear, 'function');
  assert.ok(
    decorations()[0].eq(
      Decoration.node(target.position, target.position + target.node.nodeSize, {
        class: 'reference-highlight',
      }),
    ),
  );
  assert.ok(view.state.selection.eq(selection));
  for (const transaction of transactions) {
    assert.equal(transaction.steps.length, 0);
    assert.equal(transaction.docChanged, false);
    assert.equal(transaction.getMeta('addToHistory'), false);
    assert.equal(transaction.getMeta('preventUpdate'), true);
  }
  view.dispatch(view.state.tr);
  view.dispatch(
    view.state.tr.setSelection(TextSelection.create(view.state.doc, target.position + 1)),
  );
  assert.equal(decorations().length, 1);
  assert.deepEqual(view.state.doc.toJSON(), before);
  assert.equal(undoDepth(view.state), 0);

  view.dispatch(view.state.tr.insertText('Prefix ', 1));
  const shifted = at(view, 'id', 'target');
  assert.equal(decorations()[0].from, shifted.position);
  assert.equal(decorations()[0].to, shifted.position + shifted.node.nodeSize);
  assert.equal(undoDepth(view.state), 1);
  const edited = view.state.doc.toJSON();
  clear();
  assert.equal(decorations().length, 0);
  assert.deepEqual(view.state.doc.toJSON(), edited);
  assert.equal(undoDepth(view.state), 1);
});

test('occurrence highlights use exact inline or block identities, remain pane-local and disappear on deletion', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const mounted = mount(context);
  const otherPane = mount(context);
  const { view, decorations } = mounted;
  const before = view.state.doc.toJSON();
  showReferenceHighlight(view, {
    requestId: 'inline',
    blockId: 'before',
    occurrenceId: 'second',
  });
  const inline = at(view, 'occurrence_id', 'second');
  assert.ok(
    decorations()[0].eq(
      Decoration.inline(inline.position, inline.position + inline.node.nodeSize, {
        class: 'reference-highlight',
      }),
    ),
  );
  assert.equal(otherPane.decorations().length, 0);
  assert.deepEqual(view.state.doc.toJSON(), before);
  assert.deepEqual(otherPane.view.state.doc.toJSON(), before);
  assert.equal(undoDepth(view.state), 0);
  view.dispatch(view.state.tr.insertText('Prefix ', inline.position));
  const shifted = at(view, 'occurrence_id', 'second');
  assert.equal(decorations()[0].from, shifted.position);
  view.dispatch(view.state.tr.delete(shifted.position, shifted.position + shifted.node.nodeSize));
  assert.equal(decorations().length, 0);
  assert.equal(referenceHighlightKey.getState(view.state).requestId, null);
  assert.ok(at(view, 'occurrence_id', 'first'));

  showReferenceHighlight(view, {
    requestId: 'embedded',
    blockId: null,
    occurrenceId: 'embed-occurrence',
  });
  const embedded = at(view, 'occurrence_id', 'embed-occurrence');
  assert.ok(
    decorations()[0].eq(
      Decoration.node(embedded.position, embedded.position + embedded.node.nodeSize, {
        class: 'reference-highlight',
      }),
    ),
  );
  assert.equal(
    showReferenceHighlight(view, {
      requestId: 'missing',
      blockId: 'target',
      occurrenceId: 'absent',
    }),
    undefined,
  );
  assert.equal(decorations().length, 0, 'a missing occurrence must not fall back to its block');
});

test('request changes, expiry, explicit cleanup and unmount cancel highlight timers without document changes', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const { view, decorations, transactions, destroy } = mount(context);
  const before = view.state.doc.toJSON();
  const clearFirst = showReferenceHighlight(view, { requestId: 'first', blockId: 'before' });
  context.mock.timers.tick(1000);
  showReferenceHighlight(view, { requestId: 'second', blockId: 'target' });
  clearFirst();
  context.mock.timers.tick(REFERENCE_HIGHLIGHT_MS - 1000);
  assert.equal(referenceHighlightKey.getState(view.state).requestId, 'second');
  assert.equal(decorations().length, 1, 'the old request timer must not clear the new request');
  context.mock.timers.tick(999);
  assert.equal(decorations().length, 1);
  context.mock.timers.tick(1);
  assert.equal(decorations().length, 0);

  const clear = showReferenceHighlight(view, { requestId: 'explicit', blockId: 'target' });
  clear();
  assert.equal(decorations().length, 0);
  const afterCleanup = transactions.length;
  context.mock.timers.tick(REFERENCE_HIGHLIGHT_MS);
  assert.equal(transactions.length, afterCleanup);

  showReferenceHighlight(view, { requestId: 'unmount', blockId: 'target' });
  destroy();
  const afterUnmount = transactions.length;
  context.mock.timers.tick(REFERENCE_HIGHLIGHT_MS);
  assert.equal(transactions.length, afterUnmount);
  assert.deepEqual(view.state.doc.toJSON(), before);
  assert.equal(undoDepth(view.state), 0);
  assert.ok(
    transactions.every((transaction) => !transaction.docChanged && !transaction.steps.length),
  );
});
