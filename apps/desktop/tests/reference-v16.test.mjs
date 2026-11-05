import assert from 'node:assert/strict';
import test from 'node:test';
import { opensReference, referenceBlocks, referenceNode } from '../src/editor/reference-model.ts';
import {
  annotationReference,
  noteBlockReference,
  sourceBlockReference,
  sourceReferenceBlocks,
} from '../src/editor/reference-picker-model.ts';

test('editing reference clicks select while modifiers and keyboard activation open', () => {
  const pointer = { ctrlKey: false, metaKey: false, detail: 1 };
  assert.equal(opensReference('edit', pointer), false);
  assert.equal(opensReference('read', pointer), true);
  assert.equal(opensReference('edit', { ...pointer, ctrlKey: true }), true);
  assert.equal(opensReference('edit', { ...pointer, metaKey: true }), true);
  assert.equal(opensReference('edit', { ...pointer, detail: 0 }), true);
});

test('an explicit alias uses the existing wiki contract without losing a pinned target', () => {
  const target = {
    target_id: 'stable-object',
    target_block_id: 'stable-block',
    target_revision_id: 'immutable-revision',
  };
  const aliased = referenceNode(target, 'Title at selection', '  My display name  ');
  assert.equal(aliased.type, 'wikiLink');
  assert.equal(aliased.attrs.wiki_target, target.target_id);
  assert.equal(aliased.attrs.label, 'My display name');
  assert.equal(aliased.attrs.target_id, target.target_id);
  assert.equal(aliased.attrs.target_block_id, target.target_block_id);
  assert.equal(aliased.attrs.target_revision_id, target.target_revision_id);
  const legacy = referenceNode({ ...target, target_revision_id: null }, 'Current title', '  ');
  assert.equal(legacy.type, 'noteReference');
  assert.equal(legacy.attrs.label, 'Current title');
  assert.ok(!('target_revision_id' in legacy.attrs));
  assert.ok(!('wiki_target' in legacy.attrs));
  assert.notEqual(legacy.attrs.occurrence_id, aliased.attrs.occurrence_id);
});

test('an embedded block keeps a fresh block id and the same stable target', () => {
  const target = {
    target_id: 'stable-note',
    target_block_id: 'stable-block',
    target_revision_id: null,
  };
  const embedded = referenceNode(target, 'Target note', '  Live block  ', true);
  assert.equal(embedded.type, 'wikiEmbed');
  assert.match(embedded.attrs.id, /^[0-9a-f-]{36}$/);
  assert.equal(embedded.attrs.wiki_target, 'stable-note');
  assert.equal(embedded.attrs.target_id, 'stable-note');
  assert.equal(embedded.attrs.target_block_id, 'stable-block');
  assert.equal(embedded.attrs.label, 'Live block');
  assert.ok(embedded.attrs.occurrence_id);
});
test('reference discovery includes rich labels and formulas while hidden comments stay out', () => {
  const blocks = referenceBlocks({
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'visible' },
        content: [
          { type: 'wikiLink', attrs: { wiki_target: 'Same#Heading', label: 'Alias' } },
          { type: 'inlineMath', attrs: { source: 'x+y' } },
          { type: 'comment', attrs: { source: 'PRIVATE' } },
        ],
      },
      {
        type: 'commentBlock',
        attrs: { source: 'PRIVATE BLOCK' },
        content: [
          {
            type: 'paragraph',
            attrs: { id: 'not-an-anchor' },
            content: [{ type: 'text', text: 'PRIVATE CHILD' }],
          },
        ],
      },
    ],
  });
  assert.deepEqual(blocks, [{ id: 'visible', kind: 'paragraph', text: 'Aliasx+y' }]);
});

test('PDF page choices preserve the first block identity and a fixed original version', () => {
  const source = {
    id: 'source',
    display_title: 'Same.pdf',
    current_version_id: 'new-original',
    format: 'pdf',
  };
  const blocks = [
    { id: 'p1-a', text: 'Start of page one', page_index: 0, pdf_geometry: null },
    { id: 'p1-b', text: 'Continuation of page one', page_index: 0, pdf_geometry: null },
    { id: 'p100', text: 'Page one hundred', page_index: 99, pdf_geometry: null },
  ];
  const choices = sourceReferenceBlocks(blocks, 'pdf');
  assert.deepEqual(
    choices.map((block) => block.id),
    ['p1-a', 'p100'],
  );
  assert.deepEqual(sourceReferenceBlocks(blocks, 'text'), blocks);
  const chosen = sourceBlockReference(source, 'selected-old-original', choices[1]);
  assert.deepEqual(chosen.target, {
    target_id: 'source',
    target_revision_id: 'selected-old-original',
    target_block_id: 'p100',
  });
  assert.equal(chosen.preview, 'Page one hundred');
  assert.deepEqual(sourceBlockReference(source, 'selected-old-original', null).target, {
    target_id: 'source',
    target_revision_id: 'selected-old-original',
    target_block_id: null,
  });
});

test('a global block result keeps the stable note and block identity', () => {
  const draft = noteBlockReference({
    id: 'stable-block',
    note_id: 'stable-note',
    note_title: 'Target note',
    revision_id: 'current-revision',
    parent_block_id: null,
    position: 3,
    depth: 0,
    kind: 'paragraph',
    text: 'Exact block text',
    alias: 'acceptance',
    reference_count: 2,
  });
  assert.deepEqual(draft.target, {
    target_id: 'stable-note',
    target_revision_id: null,
    target_block_id: 'stable-block',
  });
  assert.equal(draft.label, 'Target note');
  assert.equal(draft.preview, 'Exact block text');
});

test('an excerpt reference targets the excerpt rather than a same-titled source or current original', () => {
  const draft = annotationReference({
    id: 'excerpt-id',
    source_id: 'source-id',
    source_title: 'Same.pdf',
    source_version_id: 'excerpt-original',
    quote_preview: 'Original excerpt',
  });
  assert.deepEqual(draft.target, {
    target_id: 'excerpt-id',
    target_revision_id: 'excerpt-original',
    target_block_id: null,
  });
  assert.equal(draft.preview, 'Original excerpt');
});
