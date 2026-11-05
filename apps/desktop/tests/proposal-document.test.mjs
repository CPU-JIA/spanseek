import test from 'node:test';
import assert from 'node:assert/strict';
import { appendAnswerProposal } from '../src/inference/proposal-document.ts';
test('proposal preserves original rich document and appends immutable note/source reference targets', () => {
  const base = {
    type: 'doc',
    content: [
      {
        type: 'paragraph',
        attrs: { id: 'old-block' },
        content: [{ type: 'text', text: 'original', marks: [{ type: 'bold' }] }],
      },
      { type: 'image', attrs: { id: 'image-id', blob_hash: 'hash' } },
    ],
  };
  const original = structuredClone(base);
  const evidence = ['note', 'source'].map((kind, index) => ({
    citation_id: `E${index + 1}`,
    evidence: {
      title: '中文来源',
      target: {
        kind,
        object_id: `object-${index}`,
        content_revision: `version-${index}`,
        block_id: `block-${index}`,
        parse_id: kind === 'source' ? 'parse' : null,
      },
    },
  }));
  const answer = {
    request: { search: { query: '问题' } },
    text: '第一行 [E1]\n\n第二行 [E2]',
    evidence,
  };
  let count = 0;
  const proposed = appendAnswerProposal(base, answer, () => `new-${++count}`);
  assert.deepEqual(base, original);
  assert.deepEqual(proposed.content.slice(0, 2), original.content);
  assert.equal(proposed.content[3].content[0].text, '第一行 [E1]');
  assert.equal(proposed.content[4].content[0].text, '第二行 [E2]');
  const refs = proposed.content
    .flatMap((node) => node.content ?? [])
    .filter((node) => node.type === 'noteReference');
  assert.equal(refs.length, 2);
  for (const [index, node] of refs.entries()) {
    assert.equal(node.attrs.target_id, `object-${index}`);
    assert.equal(node.attrs.target_revision_id, `version-${index}`);
    assert.equal(node.attrs.target_block_id, `block-${index}`);
  }
  assert.notEqual(refs[0].attrs.occurrence_id, refs[1].attrs.occurrence_id);
});

test('annotation evidence retains its grant and immutable version without replacing PDF geometry by a parent block', () => {
  const answer = {
    request: { search: { query: '摘录依据' } },
    text: '答案 [E1] [E2]',
    evidence: ['text', 'pdf'].map((format, index) => ({
      citation_id: `E${index + 1}`,
      evidence: {
        title: '摘录',
        target: {
          kind: 'source',
          object_id: 'parent',
          content_revision: `frozen-${format}`,
          block_id: `parent-block-${format}`,
          page_index: format === 'pdf' ? 3 : null,
        },
        annotation: {
          annotation_id: `grant-${format}`,
          annotation_revision: 2,
          locator_hash: 'fixed',
        },
      },
    })),
  };
  let count = 0;
  const proposed = appendAnswerProposal(
    { type: 'doc', content: [] },
    answer,
    () => `new-${++count}`,
  );
  const refs = proposed.content
    .flatMap((node) => node.content ?? [])
    .filter((node) => node.type === 'noteReference');
  assert.equal(refs.length, 2);
  for (const [index, format] of ['text', 'pdf'].entries()) {
    assert.equal(refs[index].attrs.target_id, `grant-${format}`);
    assert.equal(refs[index].attrs.target_revision_id, `frozen-${format}`);
    assert.equal(Object.hasOwn(refs[index].attrs, 'target_block_id'), false);
  }
});
