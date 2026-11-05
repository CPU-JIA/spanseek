import assert from 'node:assert/strict';
import test from 'node:test';
import { documentTextStatistics } from '../src/editor/text-statistics.ts';

const paragraph = (...text) => ({
  type: 'paragraph',
  attrs: { id: crypto.randomUUID() },
  content: text.map((value) => ({ type: 'text', text: value })),
});

test('counts English words and non-whitespace characters', () => {
  assert.deepEqual(documentTextStatistics([paragraph('Hello, world!')], 'en'), {
    words: 2,
    characters: 12,
  });
});

test('joins marked inline text without merging block boundaries', () => {
  const content = [paragraph('Hello ', 'world'), paragraph('Second paragraph')];
  assert.deepEqual(documentTextStatistics(content, 'en'), {
    words: 4,
    characters: 25,
  });
});

test('segments Chinese text instead of treating the whole sentence as one word', () => {
  const stats = documentTextStatistics([paragraph('跨索让知识随时可寻。')], 'zh-CN');
  assert.ok(stats.words >= 4, `expected Chinese word segmentation, got ${stats.words}`);
  assert.ok(stats.words < stats.characters);
  assert.equal(stats.characters, 10);
});

test('ignores whitespace for character counts and handles hard breaks', () => {
  const content = [
    {
      type: 'paragraph',
      attrs: { id: crypto.randomUUID() },
      content: [
        { type: 'text', text: 'one' },
        { type: 'hardBreak' },
        { type: 'text', text: 'two' },
      ],
    },
  ];
  assert.deepEqual(documentTextStatistics(content, 'en'), { words: 2, characters: 6 });
});
