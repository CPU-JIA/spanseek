import assert from 'node:assert/strict';
import test from 'node:test';
import { parse } from 'parse5';
import { canvasSvg } from '../src/organization/canvas-export.ts';

const labels = {
  missing: '引用已失效',
  pending: '预览待更新',
  trashed: '笔记在回收站',
  partial: '部分关系未载入',
  untitled: '未命名笔记',
};
function elements(node, name) {
  return [
    ...(node.tagName === name ? [node] : []),
    ...(node.childNodes ?? []).flatMap((child) => elements(child, name)),
  ];
}
function attr(node, name) {
  return node.attrs?.find((a) => a.name === name)?.value;
}
const canvas = {
  name: '导出 <script>alert("x")</script>',
  nodes: [
    { note_id: 'a', x: -30, y: -20, width: 160, height: 96, z_index: 2 },
    { note_id: 'b', x: 330, y: 140, width: 250, height: 148, z_index: 1 },
  ],
};
const note = (title, preview, trashed = false) => ({
  note: { title, preview },
  metadata: { trashed },
});
const edges = ['block-one', 'block-two'].map((target_block_id) => ({
  source_id: 'a',
  target_id: 'b',
  target_block_id,
  kind: 'block_reference',
  context: '引用 <正文>',
  weight: 1,
}));

test('SVG preserves distinct real block links and stacking, and text cannot become active markup', () => {
  const svg = canvasSvg(
    canvas,
    new Map([
      ['a', note('<img src=x onerror=alert(1)> 中文长标题'.repeat(10), '正文预览'.repeat(100))],
      ['b', note('目标', '不可见的回收正文', true)],
    ]),
    edges,
    { labels, relationsComplete: true },
  );
  const tree = parse(svg);
  assert.equal(elements(tree, 'script').length, 0);
  assert.equal(elements(tree, 'img').length, 0);
  assert.deepEqual(
    elements(tree, 'line').map((line) => attr(line, 'data-target-block-id')),
    ['block-one', 'block-two'],
  );
  assert.deepEqual(
    elements(tree, 'g')
      .filter((g) => attr(g, 'data-note-id'))
      .map((g) => attr(g, 'data-note-id')),
    ['b', 'a'],
  );
  assert.equal(elements(tree, 'clipPath').length, 2);
  assert.ok(svg.includes('笔记在回收站') && !svg.includes('不可见的回收正文'));
  assert.ok(!svg.includes('正文预览'.repeat(100)));
  assert.ok(svg.includes('&lt;script&gt;'));
});

test('missing, pending previews and a partial relation projection are explicitly different', () => {
  const svg = canvasSvg(canvas, new Map(), [], {
    labels,
    missingIds: new Set(['a']),
    relationsComplete: false,
  });
  assert.ok(svg.includes('引用已失效'));
  assert.ok(svg.includes('预览待更新'));
  assert.ok(svg.includes('部分关系未载入'));
});
