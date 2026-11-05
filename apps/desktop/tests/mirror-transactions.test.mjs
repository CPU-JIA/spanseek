import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState } from '@tiptap/pm/state';
import { history, undo, redo } from '@tiptap/pm/history';
import { mirrorTransactions } from '../src/editor/mirror-transactions.ts';

function state() {
  const schema = new Schema({
    nodes: {
      doc: { content: 'paragraph+' },
      paragraph: { content: 'text*' },
      text: {},
    },
  });
  return EditorState.create({
    schema,
    plugins: [history()],
    doc: schema.node('doc', null, [schema.node('paragraph', null, [schema.text('原文')])]),
  });
}

test('alternating edits and undo preserve the other pane edits across independent schemas', () => {
  let left = state();
  let right = state();
  const dispatchLeft = (tr) => {
    const applied = left.applyTransaction(tr);
    left = applied.state;
    right = right.apply(mirrorTransactions(right, applied.transactions));
  };
  const dispatchRight = (tr) => {
    const applied = right.applyTransaction(tr);
    right = applied.state;
    left = left.apply(mirrorTransactions(left, applied.transactions));
  };
  dispatchLeft(left.tr.insertText('A', 3));
  dispatchRight(right.tr.insertText('B', 4));
  assert.equal(left.doc.textContent, '原文AB');
  assert.ok(undo(left, dispatchLeft));
  assert.equal(right.doc.textContent, '原文B');
  assert.ok(undo(right, dispatchRight));
  assert.equal(left.doc.textContent, '原文');
  assert.ok(redo(left, dispatchLeft));
  assert.equal(right.doc.textContent, '原文A');
  assert.ok(redo(right, dispatchRight));
  assert.equal(left.doc.textContent, '原文AB');
  assert.deepEqual(left.doc.toJSON(), right.doc.toJSON());
});

test('stale mirror bases fail explicitly without overwriting the receiving document', () => {
  const left = state();
  const right = state();
  const newer = right.apply(right.tr.insertText('本地尚未同步', 1));
  assert.throws(() => mirrorTransactions(newer, [left.tr.insertText('A', 1)]), /不同步/);
  assert.equal(newer.doc.textContent, '本地尚未同步原文');
});
