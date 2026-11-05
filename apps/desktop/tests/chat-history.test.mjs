import test from 'node:test';
import assert from 'node:assert/strict';
import {
  chatPageSeed,
  canSaveChatReply,
  prependChatMessages,
  appendChatConversations,
} from '../src/inference/chat-history.ts';
import { appendChatEvents } from '../src/inference/chat-stream.ts';
const item = (ordinal, conversation_id = 'c') => ({
  id: `message-${ordinal}`,
  conversation_id,
  ordinal,
  role: ordinal % 2 ? 'user' : 'assistant',
  content: `**${ordinal}**`,
  run_id: 'r',
  run_status: 'running',
  run_error_code: null,
});
const page = (status = 'running') => ({
  conversation: { id: 'c' },
  items: [item(99), { ...item(100), run_status: status }],
  latest_run: {
    id: 'r',
    assistant_message_id: 'message-100',
    conversation_id: 'c',
    last_event_seq: 600,
    status,
  },
});

test('atomic latest page seeds exact Markdown then replays only subsequent events', () => {
  const seed = chatPageSeed(page());
  const stream = appendChatEvents(seed.stream, [
    { seq: 600, kind: 'text_delta', text_delta: 'overlap', status: 'running' },
    { seq: 601, kind: 'text_delta', text_delta: '\n尾段', status: 'running' },
    { seq: 602, kind: 'status', text_delta: null, status: 'completed' },
  ]);
  assert.deepEqual(stream, { seq: 602, status: 'completed', text: '**100**\n尾段' });
  assert.equal(chatPageSeed(page('completed')).stream.status, 'completed');
  assert.equal(chatPageSeed({ ...page(), latest_run: null }), null);
  assert.throws(() => chatPageSeed({ ...page(), items: [item(102)] }), /snapshot/);
  assert.throws(() => chatPageSeed({ ...page(), conversation: { id: 'other' } }), /snapshot/);
});

test('older pages preserve latest message identity and reject overlapping or foreign history', () => {
  const current = [item(5), item(6)];
  const older = [item(3), item(4)];
  const combined = prependChatMessages(current, older);
  assert.deepEqual(
    combined.map((m) => m.ordinal),
    [3, 4, 5, 6],
  );
  assert.equal(combined.at(-1), current.at(-1));
  assert.equal(prependChatMessages(current, []), current);
  assert.throws(() => prependChatMessages(current, [item(5)]), /overlaps/);
  assert.throws(() => prependChatMessages(current, [item(2, 'other')]), /another/);
  assert.deepEqual(
    current.map((m) => m.ordinal),
    [5, 6],
  );
});

test('conversation paging never duplicates an existing identity', () => {
  const current = [
    { id: 'new', title: 'Latest' },
    { id: 'middle', title: 'Selected' },
  ];
  assert.deepEqual(
    appendChatConversations(current, [
      { id: 'middle', title: 'Old copy' },
      { id: 'old', title: 'Older' },
    ]),
    [...current, { id: 'old', title: 'Older' }],
  );
});

test('older partial replies stay disabled after a newer completion; live state must match its run', () => {
  const live = { id: 'new-run' };
  const completed = { seq: 10, text: 'done', status: 'completed' };
  for (const run_status of [
    'canceled',
    'interrupted',
    'timed_out',
    'failed',
    'truncated',
    'refused',
    'model_unconfigured',
    'running',
    'queued',
  ]) {
    assert.equal(canSaveChatReply({ ...item(2), run_status }, 'message-4', live, completed), false);
  }
  assert.equal(
    canSaveChatReply({ ...item(2), run_status: 'completed' }, 'message-4', live, completed),
    true,
  );
  assert.equal(canSaveChatReply(item(4), 'message-4', live, completed), false);
  assert.equal(
    canSaveChatReply(item(4), 'message-4', { id: 'r' }, { ...completed, status: 'running' }),
    false,
  );
  assert.equal(canSaveChatReply(item(4), 'message-4', { id: 'r' }, completed), true);
  assert.throws(
    () => chatPageSeed({ ...page(), items: [item(99), { ...item(100), run_status: 'completed' }] }),
    /snapshot/,
  );
});
