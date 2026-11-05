import test from 'node:test';
import assert from 'node:assert/strict';
import { duplicateTitle, NOTE_TITLE_LIMIT } from '../src/editor/duplicate-title.ts';

test('a copy keeps the source title and appends the suffix', () => {
  assert.equal(duplicateTitle('会议记录', '副本'), '会议记录 副本');
});

test('the copy title stays inside the core limit and keeps the suffix', () => {
  const title = '标'.repeat(NOTE_TITLE_LIMIT);
  const copy = duplicateTitle(title, '副本');
  assert.equal(Array.from(copy).length, NOTE_TITLE_LIMIT);
  assert.ok(copy.endsWith(' 副本'));
  assert.equal(copy.slice(0, -3), '标'.repeat(NOTE_TITLE_LIMIT - 3));
});

test('blank input never produces an empty or padded title', () => {
  assert.equal(duplicateTitle('  笔记  ', '副本'), '笔记 副本');
  assert.equal(duplicateTitle('笔记', '   '), '笔记');
  assert.equal(duplicateTitle('', '副本'), '副本');
});
