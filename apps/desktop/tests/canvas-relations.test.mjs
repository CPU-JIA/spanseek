import test from 'node:test';
import assert from 'node:assert/strict';
import {
  canvasRelationLines,
  canvasNoteEdges,
  canvasRelationKey,
  relationKindLabel,
  unplacedRelationTargets,
} from '../src/organization/canvas-relations.ts';

const nodes = [
  { note_id: 'rtos-note', x: 100, y: 80, width: 250, height: 148, z_index: 0 },
  { note_id: 'spi-source-note', x: 460, y: 220, width: 250, height: 148, z_index: 1 },
];

const edge = (source_id, target_id, kind = 'reference') => ({
  source_id,
  target_id,
  kind,
  weight: 1,
  context: '中断向量表 → SPI 时序',
  target_block_id: null,
});

const graphNode = (id, title, kind) => ({
  id,
  title,
  kind,
  tag_ids: [],
  degree: 1,
  object_id: kind === 'block' ? 'rtos-note' : id,
  block_id: kind === 'block' ? id.replace('block:', '') : null,
});

test('block references connect owner notes while preserving distinct block locations', () => {
  const note = graphNode('spi-source-note', 'SPI timing', 'note');
  const block = (id) => ({ ...graphNode(`block:${id}`, id, 'block'), object_id: note.id });
  const edges = ['timing', 'limits'].map((id) => ({
    ...edge('rtos-note', `block:${id}`, 'block_reference'),
    target_block_id: id,
  }));
  const projected = canvasNoteEdges(edges, [
    graphNode('rtos-note', 'RTOS', 'note'),
    note,
    block('timing'),
    block('limits'),
  ]);
  assert.equal(projected.length, 2);
  assert.ok(projected.every((item) => item.target_id === note.id));
  assert.deepEqual(
    projected.map((item) => item.target_block_id),
    ['timing', 'limits'],
  );
  assert.equal(new Set(projected.map(canvasRelationKey)).size, 2);
  assert.equal(canvasRelationLines(nodes, projected).length, 2);
  assert.deepEqual(unplacedRelationTargets(nodes.slice(0, 1), projected, [note]), [
    { id: note.id, title: note.title },
  ]);
  assert.deepEqual(
    canvasNoteEdges(edges, [block('timing')]),
    [],
    'an absent owner is not invented',
  );
  assert.deepEqual(
    edges.map((item) => item.target_id),
    ['block:timing', 'block:limits'],
    'the graph source is not rewritten',
  );
});

test('canvas shows only established relations whose both objects are on this canvas', () => {
  const lines = canvasRelationLines(nodes, [
    edge('rtos-note', 'spi-source-note'),
    edge('rtos-note', 'not-on-canvas'),
    edge('rtos-note', 'spi-source-note', 'canvas'),
  ]);
  assert.equal(lines.length, 1);
  assert.deepEqual(
    { x1: lines[0].x1, y1: lines[0].y1, x2: lines[0].x2, y2: lines[0].y2 },
    { x1: 225, y1: 154, x2: 585, y2: 294 },
  );
});

test('relation types stay explainable rather than becoming unlabeled lines', () => {
  assert.equal(relationKindLabel('reference'), 'body reference');
  assert.equal(relationKindLabel('relation'), 'property relation');
  assert.equal(relationKindLabel('block_reference'), 'block reference');
  assert.equal(relationKindLabel('canvas'), 'canvas membership');
});

test('unplaced relation targets are deduplicated, note-only, and explainable', () => {
  const graphNodes = [
    graphNode('rtos-note', '中断向量表与 RTOS 任务调度', 'note'),
    graphNode('spi-source-note', 'SPI 时序复盘', 'note'),
    graphNode('third-note', 'CAN 采样点复盘', 'note'),
    graphNode('source-1', 'CAN 总线故障排查手册', 'source'),
    graphNode('block:abc', '中断向量表块', 'block'),
  ];
  const targets = unplacedRelationTargets(
    nodes,
    [
      edge('rtos-note', 'third-note', 'relation'),
      edge('third-note', 'rtos-note', 'reference'),
      edge('rtos-note', 'block:abc', 'block_reference'),
      edge('rtos-note', 'source-1', 'reference'),
      edge('not-on-canvas', 'third-note', 'reference'),
      edge('rtos-note', 'spi-source-note', 'reference'),
      edge('rtos-note', 'third-note', 'canvas'),
    ],
    graphNodes,
  );
  assert.deepEqual(targets, [{ id: 'third-note', title: 'CAN 采样点复盘' }]);
});
