import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeCommittedBindings } from '../src/editor/committed-bindings.ts';

const wrap = (nodes) => ({
  type: 'doc',
  content: [{ type: 'paragraph', attrs: { id: 'paragraph' }, content: nodes }],
});
const link = (extra = {}) => ({
  type: 'wikiLink',
  attrs: { wiki_target: '阅读#结构', occurrence_id: 'occurrence', label: '原标签', ...extra },
});

test('committed wiki identities join newer edits without replacing their text or formatting', () => {
  const submitted = wrap([link()]);
  const committed = wrap([link({ target_id: 'resolved-note', target_block_id: 'resolved-block' })]);
  const current = wrap([
    { type: 'text', text: '之后输入', marks: [{ type: 'underline' }] },
    link({ label: '之后改的别名' }),
  ]);
  const before = structuredClone(current);
  const merged = mergeCommittedBindings(current, submitted, committed);
  assert.deepEqual(current, before);
  assert.deepEqual(merged.content[0].content[0], current.content[0].content[0]);
  assert.deepEqual(merged.content[0].content[1].attrs, {
    ...current.content[0].content[1].attrs,
    target_id: 'resolved-note',
    target_block_id: 'resolved-block',
  });
});

test('changed targets, new occurrences and explicit bindings never inherit a stale receipt target', () => {
  const submitted = wrap([link()]);
  const committed = wrap([link({ target_id: 'old-target' })]);
  for (const attrs of [
    { wiki_target: '另一本笔记' },
    { occurrence_id: 'another-occurrence' },
    { target_id: 'chosen-target' },
    { target_block_id: 'chosen-block' },
    { target_revision_id: 'chosen-revision' },
  ]) {
    const current = wrap([link(attrs)]);
    assert.equal(mergeCommittedBindings(current, submitted, committed), current);
  }
});

test('an unrelated committed node cannot bind a current wiki link, and ordinary documents keep identity', () => {
  const submitted = wrap([link()]);
  for (const saved of [
    link({ wiki_target: 'different-target', target_id: 'resolved' }),
    { type: 'wikiEmbed', attrs: { ...link().attrs, target_id: 'resolved' } },
  ]) {
    const current = wrap([link()]);
    assert.equal(mergeCommittedBindings(current, submitted, wrap([saved])), current);
  }
  const plain = wrap([{ type: 'text', text: '不变' }]);
  assert.equal(mergeCommittedBindings(plain, plain, plain), plain);
});
