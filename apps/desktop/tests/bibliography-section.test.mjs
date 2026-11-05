import test from 'node:test';
import assert from 'node:assert/strict';
import {
  bibliographyFingerprint,
  bibliographyFingerprintFromJson,
  bibliographyRange,
  bibliographySection,
  topLevelNodes,
} from '../src/citations/bibliography-section.ts';

/** Minimal ProseMirror-shaped document: top-level blocks of text with real offsets. */
function fakeDoc(blocks) {
  const items = [];
  let offset = 0;
  for (const block of blocks) {
    const text = block.text ?? '';
    const nodeSize = text.length + 2;
    items.push({
      node: { type: { name: block.type }, textContent: text, nodeSize, attrs: block.attrs },
      offset,
    });
    offset += nodeSize;
  }
  return {
    contentSize: offset,
    forEach(callback) {
      items.forEach((item) => callback(item.node, item.offset));
    },
  };
}

function section(doc) {
  const range = bibliographyRange(doc);
  if (!range) return null;
  const nodes = topLevelNodes(doc);
  const covered = nodes.filter(
    ({ offset, node }) => offset >= range.from && offset + node.nodeSize <= range.to,
  );
  return { range, types: covered.map(({ node }) => node.type.name) };
}

test('a note without a bibliography heading has no section', () => {
  assert.equal(bibliographyRange(fakeDoc([{ type: 'paragraph', text: 'body' }])), null);
});

test('the section covers a trailing entry list and missing-target note', () => {
  const doc = fakeDoc([
    { type: 'paragraph', text: 'body' },
    { type: 'heading', text: '参考文献' },
    { type: 'bulletList', text: 'Vaswani 2017' },
    { type: 'paragraph', text: '缺少引用目标：missingprobe' },
  ]);
  const found = section(doc);
  assert.deepEqual(found.types, ['heading', 'bulletList', 'paragraph']);
  assert.deepEqual(found.range, { from: 6, to: doc.contentSize });
});

test('body text written after the section is never covered by the range', () => {
  const doc = fakeDoc([
    { type: 'heading', text: '参考文献' },
    { type: 'bulletList', text: 'Vaswani 2017' },
    { type: 'paragraph', text: '缺少引用目标：missingprobe' },
    { type: 'paragraph', text: 'later conclusion' },
  ]);
  const found = section(doc);
  assert.deepEqual(found.types, ['heading', 'bulletList', 'paragraph']);
  assert.equal(
    found.range.to,
    2 + 2 + 'Vaswani 2017'.length + 2 + '缺少引用目标：missingprobe'.length + 2 + 2,
  );
  assert.ok(found.range.to < doc.contentSize);
});

test('a writer paragraph after the list stays out of the replaced range', () => {
  const doc = fakeDoc([
    { type: 'heading', text: 'References' },
    { type: 'bulletList', text: 'Vaswani 2017' },
    { type: 'paragraph', text: 'my own closing remark' },
  ]);
  const found = section(doc);
  assert.deepEqual(found.types, ['heading', 'bulletList']);
});

test('a heading without a list still yields a removable heading range', () => {
  const doc = fakeDoc([
    { type: 'paragraph', text: 'body' },
    { type: 'heading', text: 'References' },
    { type: 'paragraph', text: 'notes' },
  ]);
  assert.deepEqual(section(doc).types, ['heading']);
});

test('the last bibliography heading wins when the writer duplicated one', () => {
  const doc = fakeDoc([
    { type: 'heading', text: 'References' },
    { type: 'bulletList', text: 'old' },
    { type: 'paragraph', text: 'middle' },
    { type: 'heading', text: '参考文献' },
    { type: 'bulletList', text: 'new' },
  ]);
  const found = section(doc);
  assert.deepEqual(found.types, ['heading', 'bulletList']);
  assert.equal(found.range.from, 25);
});

test('fingerprints the same JSON content as the persisted top-level nodes', () => {
  const generated = [
    {
      type: 'heading',
      attrs: { level: 2 },
      content: [{ type: 'text', text: 'References' }],
    },
    {
      type: 'bulletList',
      content: [
        {
          type: 'listItem',
          content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Vaswani 2017' }] }],
        },
      ],
    },
  ];
  const doc = fakeDoc([
    { type: 'heading', text: 'References' },
    { type: 'bulletList', text: 'Vaswani 2017' },
  ]);
  assert.equal(
    bibliographyFingerprintFromJson(generated),
    bibliographyFingerprint(
      topLevelNodes(doc).map(({ node }) => ({ type: node.type.name, text: node.textContent })),
    ),
  );
});

test('reads persisted style metadata and detects manual edits', () => {
  const blocks = [
    { type: 'heading', text: 'References' },
    { type: 'bulletList', text: 'Vaswani 2017' },
  ];
  const fingerprint = bibliographyFingerprint(
    blocks.map((block) => ({ type: block.type, text: block.text })),
  );
  blocks[0] = {
    ...blocks[0],
    attrs: {
      bibliography_style_id: 'https://www.zotero.org/styles/apa',
      bibliography_fingerprint: fingerprint,
    },
  };
  const untouched = bibliographySection(fakeDoc(blocks));
  assert.equal(untouched.styleId, 'https://www.zotero.org/styles/apa');
  assert.equal(untouched.modified, false);

  const edited = bibliographySection(fakeDoc([blocks[0], { ...blocks[1], text: 'Edited entry' }]));
  assert.equal(edited.modified, true);
});

test('legacy bibliographies without metadata still have a replaceable range', () => {
  const doc = fakeDoc([
    { type: 'heading', text: 'References' },
    { type: 'bulletList', text: 'Vaswani 2017' },
  ]);
  const found = bibliographySection(doc);
  assert.equal(found.styleId, null);
  assert.equal(found.fingerprint, null);
  assert.equal(found.modified, null);
  assert.deepEqual({ from: found.from, to: found.to }, bibliographyRange(doc));
});
