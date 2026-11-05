import test from 'node:test';
import assert from 'node:assert/strict';
import { proposalBlockChanges } from '../src/inference/proposal-changes.ts';

function paragraph(id, text) {
  return { type: 'paragraph', attrs: { id }, content: [{ type: 'text', text }] };
}

function heading(id, level, text) {
  return { type: 'heading', attrs: { id, level }, content: [{ type: 'text', text }] };
}

function document(...content) {
  return { type: 'doc', content };
}

test('proposal change summary separates added, changed and removed blocks', () => {
  const base = document(
    paragraph('keep', '保留'),
    paragraph('edit', '旧正文'),
    paragraph('remove', '移除'),
  );
  const proposed = document(
    paragraph('keep', '保留'),
    paragraph('edit', '新正文'),
    paragraph('add', '新增段落'),
  );
  const changes = proposalBlockChanges(base, proposed);
  assert.equal(changes.added, 1);
  assert.equal(changes.changed, 1);
  assert.equal(changes.removed, 1);
  assert.equal(changes.total, 3);
  assert.deepEqual(
    changes.changes.map((item) => [item.kind, item.id, item.preview]),
    [
      ['changed', 'edit', '新正文'],
      ['added', 'add', '新增段落'],
      ['removed', 'remove', '移除'],
    ],
  );
});

test('proposal change summary is bounded and reports hidden changes', () => {
  const base = document(paragraph('old-1', '旧一'), paragraph('old-2', '旧二'));
  const proposed = document(
    paragraph('new-1', '新一'),
    paragraph('new-2', '新二'),
    paragraph('new-3', '新三'),
  );
  const changes = proposalBlockChanges(base, proposed, 2);
  assert.equal(changes.total, 5);
  assert.equal(changes.changes.length, 2);
  assert.equal(changes.hidden, 3);
});

test('a new note is represented as added blocks, not removed base content', () => {
  const proposed = document(paragraph('new', '第一篇 AI 草稿'));
  const changes = proposalBlockChanges(null, proposed);
  assert.equal(changes.added, 1);
  assert.equal(changes.changed, 0);
  assert.equal(changes.removed, 0);
  assert.equal(changes.changes[0].type, 'paragraph');
});
test('an unchanged block with a regenerated id is not reported as add plus remove', () => {
  const base = document(paragraph('old-id', '同一段文字'));
  const proposed = document(paragraph('new-id', '同一段文字'));
  const changes = proposalBlockChanges(base, proposed);
  assert.equal(changes.added, 0);
  assert.equal(changes.changed, 0);
  assert.equal(changes.removed, 0);
  assert.equal(changes.total, 0);
});

test('a regenerated id with an attribute change is still reported as changed', () => {
  const base = document(heading('old-id', 2, '同一段文字'));
  const proposed = document(heading('new-id', 3, '同一段文字'));
  const changes = proposalBlockChanges(base, proposed);
  assert.equal(changes.added, 0);
  assert.equal(changes.changed, 1);
  assert.equal(changes.removed, 0);
  assert.deepEqual(
    changes.changes.map((item) => [item.kind, item.id, item.type]),
    [['changed', 'new-id', 'heading']],
  );
});
