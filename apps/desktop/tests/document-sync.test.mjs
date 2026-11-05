import assert from 'node:assert/strict';
import test from 'node:test';
import { needsDocumentSync } from '../src/editor/document-sync.ts';

// A fixed block id keeps two separately built documents byte-identical.
function document(text) {
  return {
    type: 'doc',
    content: [{ type: 'paragraph', attrs: { id: 'block-1' }, content: [{ type: 'text', text }] }],
  };
}

test('a self-produced document settles by identity without serializing it', () => {
  const produced = document('刚输入的正文');
  let reads = 0;
  assert.equal(
    needsDocumentSync(produced, produced, () => {
      reads += 1;
      return document('刚输入的正文');
    }),
    false,
  );
  assert.equal(reads, 0, 'the same document object must never be serialized for comparison');
});

test('an external change is detected while an equal document is left alone', () => {
  const record = document('外部采用的版本');
  const stale = document('本地旧内容');
  assert.equal(
    needsDocumentSync(record, stale, () => document('本地旧内容')),
    true,
  );
  assert.equal(
    needsDocumentSync(record, stale, () => document('外部采用的版本')),
    false,
  );
  assert.equal(
    needsDocumentSync(record, null, () => document('外部采用的版本')),
    false,
  );
});
