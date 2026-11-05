import test from 'node:test';
import assert from 'node:assert/strict';
import { knowledgeSearchNotices } from '../src/organization/knowledge-search-status.ts';

const zh = (zhText) => zhText;

test('semantic unavailability keeps its actionable reason beside the lexical fallback', () => {
  const notices = knowledgeSearchNotices(
    {
      lexicalAvailable: true,
      semanticAvailable: false,
      unavailableReason: 'embedding_unconfigured',
      idCount: 18,
    },
    zh,
  );
  assert.equal(notices.length, 1);
  assert.match(notices[0].text, /尚未配置知识库向量检索/);
  assert.match(notices[0].text, /当前显示关键词命中的对象/);
});

test('lexical failure is an alert and not presented as a normal fallback', () => {
  const notices = knowledgeSearchNotices(
    {
      lexicalAvailable: false,
      semanticAvailable: false,
      unavailableReason: 'index_unavailable',
      idCount: 0,
    },
    zh,
  );
  assert.deepEqual(
    notices.map((notice) => notice.tone),
    ['status', 'alert'],
  );
  assert.match(notices[1].text, /关键词检索也不可用/);
});

test('the hundred-object cap is stated without pretending the result set is complete', () => {
  const notices = knowledgeSearchNotices(
    {
      lexicalAvailable: true,
      semanticAvailable: true,
      unavailableReason: null,
      idCount: 100,
    },
    zh,
  );
  assert.equal(notices.length, 1);
  assert.match(notices[0].text, /最多返回前 100 个对象/);
});
