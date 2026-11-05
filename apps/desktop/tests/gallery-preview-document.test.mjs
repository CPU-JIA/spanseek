import test from 'node:test';
import assert from 'node:assert/strict';
import {
  projectPreview,
  withoutLeadingTitle,
  previewExcerpt,
  PREVIEW_LIMITS,
} from '../src/gallery/preview-document.ts';

const text = (value, marks = []) => ({ type: 'text', text: value, marks });
const paragraph = (value) => ({ type: 'paragraph', content: [text(value)] });
const document = (...content) => ({ type: 'doc', content });
const flatten = (nodes) => nodes.flatMap((node) => [node, ...flatten(node.content ?? [])]);
function freeze(value) {
  if (value && typeof value === 'object') {
    Object.freeze(value);
    for (const child of Object.values(value)) freeze(child);
  }
  return value;
}

test('projection preserves supported rich formatting without mutating or retaining source-only attributes', () => {
  const source = freeze(
    document(
      {
        type: 'heading',
        attrs: { id: 'block-1', level: 2 },
        content: [text('标题', [{ type: 'bold' }, { type: 'italic' }])],
      },
      {
        type: 'paragraph',
        content: [
          text('链接文字', [{ type: 'link', attrs: { href: 'https://example.test/private' } }]),
          {
            type: 'noteReference',
            attrs: {
              label: '固定引用',
              target_id: 'private-target',
              occurrence_id: 'private-occurrence',
            },
          },
        ],
      },
      {
        type: 'taskList',
        content: [{ type: 'taskItem', attrs: { checked: true }, content: [paragraph('完成任务')] }],
      },
      {
        type: 'image',
        attrs: { blob_hash: 'a'.repeat(64), alt: '本机原图', original_name: 'private-name.png' },
      },
      { type: 'image', attrs: { blob_hash: 'b'.repeat(64) } },
    ),
  );
  const before = JSON.stringify(source),
    result = projectPreview(source);
  assert.equal(JSON.stringify(source), before);
  assert.deepEqual(result.nodes[0].content[0].marks, ['bold', 'italic']);
  assert.equal(result.nodes[2].content[0].attrs.checked, true);
  assert.equal(flatten(result.nodes).filter((node) => node.type === 'image').length, 1);
  assert.equal(result.truncated, true);
  assert.equal(result.empty, false);
  assert.doesNotMatch(
    JSON.stringify(result),
    /private|target_id|occurrence_id|original_name|"href"|"id"/,
  );
});

test('very long Unicode paragraphs stop at code point budget without slicing a surrogate pair', () => {
  const value = '😀汉'.repeat(100_000),
    source = document(paragraph(value));
  const projected = projectPreview(source);
  const preview = projected.nodes[0].content[0].text;
  assert.equal([...preview].length, PREVIEW_LIMITS.characters);
  assert.equal(preview, '😀汉'.repeat(PREVIEW_LIMITS.characters / 2));
  assert.equal(preview.isWellFormed(), true);
  assert.equal(projected.truncated, true);
  assert.equal(source.content[0].content[0].text, value);
});

test('top-level, node and depth limits stop traversal before unneeded large tree branches', () => {
  const many = Array.from({ length: 30 }, (_, index) => paragraph(String(index)));
  Object.defineProperty(many, 8, {
    get: () => {
      throw Error('ninth block traversed');
    },
  });
  assert.equal(projectPreview(document(...many.slice(0, 8))).nodes.length, 8);
  const top = projectPreview({ type: 'doc', content: many });
  assert.equal(top.nodes.length, 8);
  assert.equal(top.truncated, true);

  const wide = projectPreview(
    document({
      type: 'bulletList',
      content: Array.from({ length: 1000 }, () => ({
        type: 'listItem',
        content: [paragraph('x')],
      })),
    }),
  );
  assert.ok(flatten(wide.nodes).length <= PREVIEW_LIMITS.nodes);
  assert.equal(wide.truncated, true);
  let deep = paragraph('deep');
  for (let i = 0; i < 20000; ++i) deep = { type: 'blockquote', content: [deep] };
  const limited = projectPreview(document(deep));
  assert.equal(flatten(limited.nodes).length, PREVIEW_LIMITS.depth);
  assert.equal(limited.truncated, true);
});

test('table crop honors three by four visual grid including row and column spans', () => {
  const rows = Array.from({ length: 12 }, () => ({
    type: 'tableRow',
    content: Array.from({ length: 12 }, () => ({
      type: 'tableCell',
      content: [paragraph('cell')],
    })),
  }));
  rows[0].content[0].attrs = { colspan: 2, rowspan: 2 };
  const source = document({ type: 'table', content: rows }),
    before = JSON.stringify(source);
  const result = projectPreview(source),
    table = result.nodes[0];
  assert.equal(table.content.length, 3);
  assert.deepEqual(
    table.content.map((row) => row.content.length),
    [3, 2, 4],
  );
  assert.deepEqual(table.content[0].content[0].attrs, { colspan: 2, rowspan: 2 });
  assert.equal(JSON.stringify(source), before);
  assert.equal(result.truncated, true);
});

test('unknown nodes stay visibly unsupported and labels and metadata cannot bypass the text budget', () => {
  const result = projectPreview(
    document(
      { type: 'future-node', content: [paragraph('uninterpreted')] },
      { type: 'noteReference', attrs: { label: '字'.repeat(20000) } },
      { type: 'image', attrs: { blob_hash: 'z'.repeat(10000), alt: '图'.repeat(20000) } },
    ),
  );
  assert.equal(result.nodes[0].type, 'unsupported');
  assert.equal(result.nodes[1].attrs.label.length, 240);
  assert.equal(result.nodes[2].attrs.alt.length, 120);
  assert.equal(result.nodes[2].attrs.blob_hash, '');
  assert.ok(JSON.stringify(result).length < 1500);
  assert.equal(result.empty, false);
});

test('the card body drops only the leading H1 that repeats the card title', () => {
  const source = projectPreview(
    document(
      { type: 'heading', attrs: { level: 1 }, content: [text('  中英文 Mixed Title  ')] },
      paragraph('正文第一段'),
    ),
  );
  const body = withoutLeadingTitle(source, '中英文 mixed title');
  assert.equal(body.nodes.length, 1);
  assert.equal(body.nodes[0].type, 'paragraph');
  assert.equal(body.empty, false);
  assert.equal(source.nodes.length, 2);
});

test('a heading that differs from the title and a non-leading heading both stay', () => {
  const different = projectPreview(
    document({ type: 'heading', attrs: { level: 1 }, content: [text('另一个标题')] }),
  );
  assert.equal(withoutLeadingTitle(different, '笔记标题').nodes.length, 1);
  const later = projectPreview(
    document(paragraph('开头'), {
      type: 'heading',
      attrs: { level: 1 },
      content: [text('笔记标题')],
    }),
  );
  assert.equal(withoutLeadingTitle(later, '笔记标题').nodes.length, 2);
  const levelTwo = projectPreview(
    document({ type: 'heading', attrs: { level: 2 }, content: [text('笔记标题')] }),
  );
  assert.equal(withoutLeadingTitle(levelTwo, '笔记标题').nodes.length, 1);
});

test('frontmatter is hidden from cards even when it precedes the duplicated title', () => {
  const source = projectPreview(
    document(
      { type: 'frontmatter', attrs: { source: 'tags: [a]' } },
      { type: 'heading', attrs: { level: 1 }, content: [text('只有标题')] },
    ),
  );
  const body = withoutLeadingTitle(source, '只有标题');
  assert.deepEqual(body.nodes, []);
  assert.equal(body.empty, true);
  assert.deepEqual(
    withoutLeadingTitle(source, '   ').nodes.map((node) => node.type),
    ['heading'],
  );
});

test('the placeholder excerpt drops a leading copy of the card title', () => {
  assert.equal(previewExcerpt('笔记标题 正文开头', '笔记标题'), '正文开头');
  assert.equal(previewExcerpt('  Seed Title\n\nBody text', 'seed title'), 'Body text');
  assert.equal(previewExcerpt('Other text', '笔记标题'), 'Other text');
  assert.equal(previewExcerpt('   ', '笔记标题'), '');
  assert.equal(previewExcerpt('笔记标题 正文', ''), '笔记标题 正文');
  assert.equal(previewExcerpt('---\ntype: note\n---\n笔记标题 正文开头', '笔记标题'), '正文开头');
});
