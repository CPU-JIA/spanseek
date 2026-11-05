import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { AllSelection, EditorState, NodeSelection, TextSelection } from '@tiptap/pm/state';
import { reconcileSelection } from '../src/editor/reconcile-selection.ts';

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    text: { group: 'inline' },
    paragraph: {
      group: 'block',
      content: 'inline*',
      attrs: { id: { default: null }, block_id: { default: null } },
    },
    frontmatter: {
      group: 'block',
      atom: true,
      attrs: { id: { default: null }, source: { default: '' } },
    },
    noteReference: {
      group: 'inline',
      inline: true,
      atom: true,
      attrs: { occurrence_id: { default: null }, label: { default: '' } },
    },
  },
});

const frontmatter = { type: 'frontmatter', attrs: { id: 'yaml', source: 'title: QA' } };
const paragraph = (id, text) => ({
  type: 'paragraph',
  attrs: { id },
  content: [{ type: 'text', text }],
});
const doc = (...content) => schema.nodeFromJSON({ type: 'doc', content });

function replaceSnapshot(previous, selection, document) {
  const state = EditorState.create({ schema, doc: previous, selection });
  const transaction = state.tr.replaceWith(0, previous.content.size, document.content);
  const stepCount = transaction.steps.length;
  transaction.setSelection(reconcileSelection(selection, transaction.doc));
  assert.equal(transaction.steps.length, stepCount, 'restoring a selection adds no document step');
  assert.deepEqual(transaction.doc.toJSON(), document.toJSON());
  return state.apply(transaction);
}

test('snapshot reconciliation preserves frontmatter and occurrence NodeSelections and uses a valid fallback for removed targets', (context) => {
  const warn = context.mock.method(console, 'warn');
  const original = doc(frontmatter, paragraph('body', 'Content'));
  const restored = replaceSnapshot(
    original,
    NodeSelection.create(original, 0),
    doc(frontmatter, paragraph('body', 'Content')),
  );
  assert.ok(restored.selection instanceof NodeSelection);
  assert.equal(restored.selection.from, 0);
  assert.equal(restored.selection.to, 1);
  assert.equal(restored.selection.node.attrs.id, 'yaml');

  const reference = {
    type: 'noteReference',
    attrs: { occurrence_id: 'occurrence', label: 'Target' },
  };
  const references = (prefix) =>
    doc({
      type: 'paragraph',
      attrs: { id: 'references' },
      content: [{ type: 'text', text: prefix }, reference],
    });
  const before = references('A');
  const moved = replaceSnapshot(before, NodeSelection.create(before, 2), references('Longer'));
  assert.ok(moved.selection instanceof NodeSelection);
  assert.equal(moved.selection.from, 7);
  assert.equal(moved.selection.node.attrs.occurrence_id, 'occurrence');

  const removed = replaceSnapshot(
    original,
    NodeSelection.create(original, 0),
    doc(paragraph('body', 'Content')),
  );
  assert.ok(removed.selection instanceof TextSelection);
  assert.equal(removed.selection.from, 1);
  assert.equal(removed.selection.$from.parent.inlineContent, true);
  assert.equal(warn.mock.callCount(), 0);
});

test('snapshot reconciliation retains backward text and all selections, and normalizes shortened or block-only targets', (context) => {
  const warn = context.mock.method(console, 'warn');
  const original = doc(frontmatter, paragraph('body', 'abcdef'));
  const backward = TextSelection.create(original, 7, 3);
  const restored = replaceSnapshot(
    original,
    backward,
    doc(frontmatter, paragraph('body', 'abcdef')),
  );
  assert.ok(restored.selection instanceof TextSelection);
  assert.equal(restored.selection.anchor, 7);
  assert.equal(restored.selection.head, 3);

  const shortened = replaceSnapshot(original, backward, doc(frontmatter, paragraph('body', 'x')));
  assert.ok(shortened.selection instanceof TextSelection);
  assert.equal(shortened.selection.$from.parent.inlineContent, true);
  assert.ok(shortened.selection.to <= shortened.doc.content.size);

  const blockOnly = replaceSnapshot(original, backward, doc(frontmatter));
  assert.ok(blockOnly.selection instanceof NodeSelection);
  assert.equal(blockOnly.selection.node.attrs.id, 'yaml');
  const all = replaceSnapshot(original, new AllSelection(original), doc(frontmatter));
  assert.ok(all.selection instanceof AllSelection);
  assert.equal(all.selection.from, 0);
  assert.equal(all.selection.to, all.doc.content.size);
  assert.equal(warn.mock.callCount(), 0);
});
