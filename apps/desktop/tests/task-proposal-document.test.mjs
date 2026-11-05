import test from 'node:test';
import assert from 'node:assert/strict';
import { prepareTaskProposal } from '../src/inference/task-proposal-document.ts';

const material = (id, annotation = null) => ({
  id,
  title: `Material ${id}`,
  text: 'source text',
  target: { object_id: id, content_revision: `${id}-revision`, block_id: `${id}-block` },
  annotation,
});
const walk = (nodes) => nodes.flatMap((node) => [node, ...walk(node.content ?? [])]);
test('numbered citations become versioned links only in the new prose', () => {
  const base = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'original' },
        content: [{ type: 'text', text: 'An existing [1] stays unchanged.' }],
      },
    ],
  };
  const output = prepareTaskProposal(
    base,
    'A supported claim [1] and an unknown reference [99].',
    [material('note')],
    'rich',
  );
  assert.deepEqual(output.document.content[0], base.content[0]);
  const links = walk(output.document.content).filter((node) => node.type === 'wikiLink');
  assert.equal(links.length, 1);
  assert.equal(links[0].attrs.target_id, 'note');
  assert.equal(links[0].attrs.target_revision_id, 'note-revision');
  assert.equal(links[0].attrs.target_block_id, 'note-block');
  assert.equal(links[0].attrs.label, '[1]');
  assert.deepEqual(output.unresolvedCitations, [99]);
  assert.ok(JSON.stringify(output.document).includes('[99]'));
});
test('code examples and existing hyperlinks are not converted into evidence claims', () => {
  const output = prepareTaskProposal(
    { type: 'doc', content: [] },
    '`array[1]`\n\n```js\nitems[1]\n```\n\n[example [1]](https://example.test)',
    [material('note')],
    'rich',
  );
  assert.equal(walk(output.document.content).filter((node) => node.type === 'wikiLink').length, 0);
});
test('annotation citations keep the annotation identity and do not authorize its parent block', () => {
  const output = prepareTaskProposal(
    { type: 'doc', content: [] },
    'Quoted [1].',
    [material('parent', { annotation_id: 'excerpt' })],
    'rich',
  );
  const link = walk(output.document.content).find((node) => node.type === 'wikiLink');
  assert.equal(link.attrs.target_id, 'excerpt');
  assert.equal(link.attrs.target_revision_id, 'parent-revision');
  assert.equal(link.attrs.target_block_id, undefined);
});
test('Markdown source stays unchanged and only explicitly cited materials get reference entries', () => {
  const source = '# Keep this source\n\nOne claim [2].';
  const output = prepareTaskProposal(
    { type: 'doc', content: [] },
    source,
    [material('unused'), material('used')],
    'source',
  );
  assert.ok(JSON.stringify(output.document).includes('# Keep this source'));
  const links = walk(output.document.content).filter((node) => node.type === 'wikiLink');
  assert.equal(links.length, 1);
  assert.equal(links[0].attrs.target_id, 'used');
});
