import test from 'node:test';
import assert from 'node:assert/strict';
import { searchHighlightParts, searchHighlightTerms } from '../src/inference/search-highlight.ts';

test('Chinese query terms highlight the exact phrase without breaking text', () => {
  const text = '证据整理 · 研究笔记 02';
  const parts = searchHighlightParts(text, '证据整理', 'zh');
  assert.equal(parts.map((part) => part.text).join(''), text);
  assert.deepEqual(
    parts.filter((part) => part.match).map((part) => part.text),
    ['证据整理'],
  );
  assert.ok(searchHighlightTerms('证据整理', 'zh').includes('证据'));
  assert.ok(searchHighlightTerms('证据整理', 'zh').includes('整理'));
});

test('English terms are highlighted case-insensitively and adjacent matches merge', () => {
  const parts = searchHighlightParts(
    'Evidence retrieval and evidence review',
    'evidence retrieval',
    'en',
  );
  assert.equal(parts.map((part) => part.text).join(''), 'Evidence retrieval and evidence review');
  assert.deepEqual(
    parts.filter((part) => part.match).map((part) => part.text),
    ['Evidence retrieval', 'evidence'],
  );
});

test('query punctuation is treated literally rather than as a regular expression', () => {
  const text = 'a.b axb';
  const parts = searchHighlightParts(text, 'a.b', 'en');
  const matches = parts.filter((part) => part.match).map((part) => part.text);
  assert.equal(parts.map((part) => part.text).join(''), text);
  assert.ok(matches.includes('a.b'));
  assert.ok(!matches.includes('axb'));
});

test('highlighting preserves inert markup-like text', () => {
  const text = '<script>alert(1)</script>';
  const parts = searchHighlightParts(text, 'script', 'en');
  assert.equal(parts.map((part) => part.text).join(''), text);
  assert.deepEqual(
    parts.filter((part) => part.match).map((part) => part.text),
    ['script', 'script'],
  );
});

test('empty queries do not create highlight fragments', () => {
  assert.deepEqual(searchHighlightParts('plain text', '   ', 'en'), [
    { text: 'plain text', match: false },
  ]);
});
test('Latin terms do not highlight fragments of a larger word', () => {
  const text = 'earth art artist AI笔记';
  const parts = searchHighlightParts(text, 'art AI', 'en');
  assert.equal(parts.map((part) => part.text).join(''), text);
  assert.deepEqual(
    parts.filter((part) => part.match).map((part) => part.text),
    ['art', 'AI'],
  );
});
