import assert from 'node:assert/strict';
import test from 'node:test';
import {
  referenceAnchor,
  referenceBlocks,
  referenceNode,
  referenceText,
} from '../src/editor/reference-model.ts';

test('reference selection preserves stable paragraph identities through nested lists and rich text', () => {
  const document = {
    type: 'doc',
    content: [
      { type: 'heading', attrs: { id: 'heading' }, content: [{ type: 'text', text: 'A title' }] },
      {
        type: 'bulletList',
        attrs: { id: 'list' },
        content: [
          {
            type: 'listItem',
            attrs: { id: 'item' },
            content: [
              {
                type: 'paragraph',
                attrs: { id: 'paragraph' },
                content: [
                  { type: 'text', text: 'Read ' },
                  {
                    type: 'noteReference',
                    attrs: { label: 'Target', target_id: 'target', occurrence_id: 'occurrence' },
                  },
                  { type: 'hardBreak' },
                  { type: 'text', text: 'again' },
                ],
              },
            ],
          },
        ],
      },
      { type: 'paragraph', attrs: { id: 'empty' }, content: [] },
      { type: 'image', attrs: { id: 'image' } },
    ],
  };
  assert.deepEqual(referenceBlocks(document), [
    { id: 'heading', kind: 'heading', text: 'A title' },
    { id: 'paragraph', kind: 'paragraph', text: 'Read Target\nagain' },
  ]);
  assert.equal(referenceText(document.content[1]), 'Read Target\nagain');
});

test('inserting the same target twice creates independent occurrences without changing target identity', () => {
  const target = {
    target_id: 'same-note',
    target_block_id: 'same-paragraph',
    target_revision_id: null,
  };
  const first = referenceNode(target, 'Same title');
  const second = referenceNode(target, 'Same title');
  assert.notEqual(first.attrs.occurrence_id, second.attrs.occurrence_id);
  assert.equal(first.attrs.target_id, target.target_id);
  assert.equal(first.attrs.target_block_id, target.target_block_id);
  assert.equal(second.attrs.target_block_id, target.target_block_id);
  assert.ok(
    !('target_revision_id' in first.attrs),
    'body links follow the current head, not an accidental frozen revision',
  );
});

test('locating an occurrence never silently falls back to its paragraph or another pane', () => {
  const block = { getAttribute: (attribute) => (attribute === 'data-id' ? 'block' : null) };
  const occurrence = {
    getAttribute: (attribute) => (attribute === 'data-reference-occurrence' ? 'first' : null),
  };
  const root = { querySelectorAll: (query) => (query === '[data-id]' ? [block] : [occurrence]) };
  assert.equal(referenceAnchor(root, 'block'), block);
  assert.equal(referenceAnchor(root, 'block', 'first'), occurrence);
  assert.equal(referenceAnchor(root, 'block', 'missing'), null);
  assert.equal(referenceAnchor(root, 'same-title'), null);
});

import { Editor, Node } from '@tiptap/core';
import UniqueID from '@tiptap/extension-unique-id';
import { EditorState } from '@tiptap/pm/state';
import { Slice, Fragment } from '@tiptap/pm/model';
import { referenceOccurrenceIDs } from '../src/editor/reference-occurrences.ts';
import { MIRROR, mirrorTransactions } from '../src/editor/mirror-transactions.ts';

function richReferenceState() {
  const content = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'original-block' },
        content: [
          {
            type: 'noteReference',
            attrs: {
              target_id: 'target-note',
              target_block_id: 'target-block',
              occurrence_id: 'original-occurrence',
              label: 'Target',
            },
          },
        ],
      },
    ],
  };
  const editor = new Editor({
    element: null,
    extensions: [
      Node.create({ name: 'doc', topNode: true, content: 'paragraph+' }),
      Node.create({ name: 'paragraph', group: 'block', content: 'inline*' }),
      Node.create({ name: 'text', group: 'inline' }),
      Node.create({
        name: 'noteReference',
        inline: true,
        group: 'inline',
        atom: true,
        addAttributes: () => ({
          target_id: { default: null },
          target_block_id: { default: null },
          label: { default: null },
          occurrence_id: { default: null },
        }),
      }),
      UniqueID.configure({
        attributeName: 'id',
        types: ['paragraph'],
        generateID: () => crypto.randomUUID(),
        filterTransaction: (tr) => !tr.getMeta(MIRROR),
      }),
      referenceOccurrenceIDs,
    ],
    content,
  });
  const plugins = editor.extensionManager.plugins.filter((plugin) =>
    plugin.key.startsWith('uniqueID'),
  );
  return {
    state: EditorState.create({
      schema: editor.schema,
      doc: editor.schema.nodeFromJSON(content),
      plugins,
    }),
    plugins,
  };
}

test('the production occurrence plugin remaps pasted IDs while preserving targets and mirror identity', () => {
  const { state, plugins } = richReferenceState();
  const { state: right } = richReferenceState();
  let slice = new Slice(Fragment.from(state.doc.firstChild), 0, 0);
  for (const plugin of plugins) {
    plugin.props.handleDOMEvents.paste();
    slice = plugin.props.transformPasted(slice);
  }
  const applied = state.applyTransaction(state.tr.insert(state.doc.content.size, slice.content));
  const json = applied.state.doc.toJSON();
  assert.equal(json.content[0].attrs.id, 'original-block');
  assert.equal(json.content[0].content[0].attrs.occurrence_id, 'original-occurrence');
  assert.notEqual(json.content[1].attrs.id, 'original-block');
  assert.notEqual(json.content[1].content[0].attrs.occurrence_id, 'original-occurrence');
  assert.equal(json.content[1].content[0].attrs.target_id, 'target-note');
  assert.equal(json.content[1].content[0].attrs.target_block_id, 'target-block');
  const mirrored = right.applyTransaction(mirrorTransactions(right, applied.transactions));
  assert.deepEqual(mirrored.state.doc.toJSON(), json);
});
