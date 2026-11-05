import test from 'node:test';
import assert from 'node:assert/strict';
import { markdownDocument, plainTextDocument } from '../src/inference/markdown-document.ts';

const flatten = (nodes) => nodes.flatMap((node) => [node, ...flatten(node.content ?? [])]);
const text = (nodes) =>
  flatten(nodes)
    .filter((node) => node.type === 'text')
    .map((node) => node.text)
    .join('');

test('AI Markdown becomes editable headings, formatted paragraphs, tasks, tables and code', () => {
  const result = markdownDocument(
    '# 标题\n\n**加粗**与*斜体*和~~删除~~以及 `code`。\n\n- [x] 已做\n- [ ] 待做\n\n3. 第三项\n4. 第四项\n\n> 引文\n\n| 名称 | 天数 |\n| --- | --- |\n| 鸢尾 | 45 |\n\n```ts\nconst days = 45;\n```\n\n---',
  );
  assert.deepEqual(result.preserved, []);
  const blocks = result.document.content;
  assert.deepEqual(
    blocks.map((b) => b.type),
    [
      'heading',
      'paragraph',
      'taskList',
      'orderedList',
      'blockquote',
      'table',
      'codeBlock',
      'horizontalRule',
    ],
  );
  assert.equal(blocks[0].attrs.level, 1);
  assert.deepEqual(
    blocks[1].content.filter((n) => n.marks).map((n) => n.marks[0].type),
    ['bold', 'italic', 'strike', 'code'],
  );
  assert.deepEqual(
    blocks[2].content.map((n) => n.attrs.checked),
    [true, false],
  );
  assert.equal(blocks[3].attrs.start, 3);
  assert.equal(blocks[5].content[0].content[0].type, 'tableHeader');
  assert.equal(blocks[5].content[1].content[0].type, 'tableCell');
  assert.equal(blocks[6].attrs.language, 'ts');
  assert.equal(text([blocks[6]]), 'const days = 45;');
  const all = flatten(blocks);
  const ids = all.filter((n) => !['text', 'hardBreak'].includes(n.type)).map((n) => n.attrs.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(ids.every((id) => /^[0-9a-f-]{36}$/.test(id)));
  assert.ok(all.filter((n) => ['text', 'hardBreak'].includes(n.type)).every((n) => !n.attrs?.id));
});

test('unsupported syntax is retained verbatim, including unsafe URLs and remote images', () => {
  const samples = [
    ['>', 'unsupported'],
    ['图片 ![说明](https://example.test/image.png)', 'image'],
    ['[危险链接](javascript:alert%281%29)', 'link'],
    ['[本机路径](file:///private.txt)', 'link'],
    ['<script>alert(1)</script>', 'html'],
    ['```ts filename=demo.ts\nconst x = 1;\n```', 'code'],
    ['**`组合格式`**', 'code'],
  ];
  for (const [source, reason] of samples) {
    const result = markdownDocument(source);
    assert.ok(result.preserved.includes(reason), source);
    assert.equal(
      result.document.content.every((n) => n.type === 'paragraph'),
      true,
      source,
    );
    assert.equal(text(result.document.content).replace(/\n\n/g, ''), source.replace(/\n\n/g, ''));
    assert.equal(
      flatten(result.document.content).some(
        (n) => n.type === 'image' || n.marks?.some((m) => m.type === 'link'),
      ),
      false,
    );
  }
});

test('reference links resolve to safe destinations while definitions needed by literal fallback survive', () => {
  const good = markdownDocument('[官网][site]\n\n[site]: https://example.test "官网"');
  assert.deepEqual(good.preserved, []);
  assert.deepEqual(good.document.content[0].content[0].marks, [
    { type: 'link', attrs: { href: 'https://example.test', title: '官网' } },
  ]);
  const fallback = markdownDocument(
    '[官网][site] ![图](https://example.test/x)\n\n[site]: https://example.test',
  );
  assert.ok(text(fallback.document.content).includes('[site]: https://example.test'));
  assert.equal(fallback.document.content.length, 2);
});

test('mixed tasks retain order and list items begin with paragraphs even with nested blocks', () => {
  const result = markdownDocument('- 普通\n- [x] 完成\n- 继续\n\n- > 引文');
  assert.deepEqual(
    result.document.content.map((n) => n.type),
    ['bulletList', 'taskList', 'bulletList'],
  );
  for (const item of flatten(result.document.content).filter((n) =>
    ['listItem', 'taskItem'].includes(n.type),
  ))
    assert.equal(item.content[0].type, 'paragraph');
});

test('deeply nested Markdown is preserved before exceeding the V1 document depth', () => {
  const source = '> '.repeat(50) + '原文';
  const result = markdownDocument(source);
  assert.ok(result.preserved.includes('depth'));
  const depth = (nodes) => Math.max(0, ...nodes.map((node) => 1 + depth(node.content ?? [])));
  assert.ok(depth(result.document.content) <= 32);
  assert.ok(text(result.document.content).includes('原文'));
});

test('plain-source mode keeps line breaks and all Markdown characters including blank lines', () => {
  const source = '## 原文\n\n**内容**\n<script>literal</script>\n';
  const doc = plainTextDocument(source);
  assert.equal(doc.content.map((n) => text([n])).join('\n'), source);
  assert.ok(doc.content.every((n) => n.type === 'paragraph'));
});
