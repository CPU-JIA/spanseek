import test from 'node:test';
import assert from 'node:assert/strict';
import { excerptNoteDocument, inheritedExcerptMetadata } from '../src/sources/excerpt-note.ts';

const identity = { local_instance_id: 'local-a', workspace_id: 'workspace-a' };
const annotation = (id, quote, page = null) => ({
  id,
  source_version_id: `${id}-version`,
  locator_schema: 1,
  locator:
    page === null
      ? {
          kind: 'text',
          source_version_id: `${id}-version`,
          block_id: `${id}-block`,
          start_codepoint: 0,
          end_codepoint: quote.length,
          quote,
          prefix: '',
          suffix: '',
        }
      : {
          kind: 'pdf',
          source_version_id: `${id}-version`,
          page_index: page,
          page_label: String(page + 1),
          crop_box: [0, 0, 1, 1],
          rotation: 0,
          coordinate_space: 'pdf-user-v1',
          rects: [[0, 0, 1, 1]],
          quote,
          prefix: '',
          suffix: '',
        },
  comment: '复制延迟必须显示来源版本。',
  row_version: 2,
  trashed: false,
});

const knowledgeObject = (collections, tags) => ({
  object: { tags: tags.map((display_name, index) => ({ id: `tag-${index}`, display_name })) },
  collections: collections.map(([id, name]) => ({ id, name })),
});

test('excerpt note keeps every quote, fixed source reference and page locator', () => {
  const document = excerptNoteDocument(
    identity,
    [annotation('one', '复制延迟不是异常。'), annotation('two', '反例必须保留。', 127)],
    '复制与分区阅读笔记',
    '查看原始摘录',
  );
  assert.equal(document.type, 'doc');
  const blockquotes = document.content.filter((node) => node.type === 'blockquote');
  assert.equal(blockquotes.length, 2);
  assert.equal(blockquotes[0].content[0].content[0].text, '复制延迟不是异常。');
  const references = document.content.filter(
    (node) =>
      node.type === 'paragraph' && node.content.some((child) => child.type === 'noteReference'),
  );
  assert.equal(references.length, 2);
  const second = references[1].content[0].attrs;
  assert.equal(second.target_id, 'two');
  assert.equal(second.target_revision_id, 'two-version');
  assert.match(second.label, /p\. 128/);
});

test('inherited excerpt metadata deduplicates categories and tags across sources', () => {
  const inherited = inheritedExcerptMetadata([
    knowledgeObject(
      [
        ['embedded', '嵌入式开发'],
        ['reading', '阅读与摘录'],
      ],
      ['CAN', 'RTOS'],
    ),
    knowledgeObject(
      [
        ['embedded', '嵌入式开发'],
        ['research', '个人研究'],
      ],
      ['CAN', '证据链'],
    ),
  ]);
  assert.deepEqual(inherited.collectionIds, ['embedded', 'reading', 'research']);
  assert.deepEqual(inherited.collectionNames, ['嵌入式开发', '阅读与摘录', '个人研究']);
  assert.deepEqual(inherited.tagNames, ['CAN', 'RTOS', '证据链']);
});
