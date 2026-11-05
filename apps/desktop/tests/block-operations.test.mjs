import assert from 'node:assert/strict';
import test from 'node:test';
import { Schema } from '@tiptap/pm/model';
import { EditorState, TextSelection } from '@tiptap/pm/state';
import {
  canMoveBlock,
  duplicateBlockTransaction,
  moveBlockTransaction,
  outermostBlock,
} from '../src/editor/block-operations.ts';

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: {
      group: 'block',
      content: 'text*',
      attrs: { id: { default: null }, block_id: { default: null } },
      toDOM: (node) => ['p', { 'data-id': node.attrs.id }, 0],
    },
    text: { group: 'inline' },
  },
});

function state() {
  const doc = schema.nodeFromJSON({
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'first', block_id: 'stable-alias' },
        content: [{ type: 'text', text: 'First' }],
      },
      {
        type: 'paragraph',
        attrs: { id: 'second', block_id: 'second-alias' },
        content: [{ type: 'text', text: 'Second' }],
      },
      {
        type: 'paragraph',
        attrs: { id: 'third', block_id: null },
        content: [{ type: 'text', text: 'Third' }],
      },
    ],
  });
  return EditorState.create({
    schema,
    doc,
    selection: TextSelection.create(doc, doc.child(0).nodeSize + 1),
  });
}

test('the command targets the outer document block and only moves within bounds', () => {
  const initial = state();
  assert.equal(outermostBlock(initial)?.node.attrs.id, 'second');
  assert.equal(canMoveBlock(initial, -1), true);
  assert.equal(canMoveBlock(initial, 1), true);
  const moved = moveBlockTransaction(initial, -1);
  assert.ok(moved);
  assert.deepEqual(
    moved.doc.content.content.map((node) => node.attrs.id),
    ['second', 'first', 'third'],
  );
  const top = EditorState.create({
    schema,
    doc: initial.doc,
    selection: TextSelection.create(initial.doc, 1),
  });
  assert.equal(canMoveBlock(top, -1), false);
  assert.equal(moveBlockTransaction(top, -1), null);
});

test('duplication preserves content while clearing reusable block identity', () => {
  const initial = state();
  const duplicated = duplicateBlockTransaction(initial);
  assert.ok(duplicated);
  assert.equal(duplicated.doc.childCount, 4);
  const copy = duplicated.doc.child(2);
  assert.equal(copy.textContent, 'Second');
  assert.equal(duplicated.doc.child(1).textContent, 'Second');
  assert.notEqual(copy.attrs.id, 'second');
  assert.equal(copy.attrs.block_id, null);
  assert.equal(new Set(duplicated.doc.content.content.map((node) => node.attrs.id)).size, 4);
});
