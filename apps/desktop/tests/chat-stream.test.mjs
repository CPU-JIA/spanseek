import test from 'node:test';
import assert from 'node:assert/strict';
import { appendChatEvents, emptyChatStream } from '../src/inference/chat-stream.ts';

test('durable chat event replay preserves Markdown and Unicode without duplicating overlapping pages', () => {
  const events = [
    { seq: 1, kind: 'status', text_delta: null, status: 'queued' },
    { seq: 2, kind: 'status', text_delta: null, status: 'running' },
    { seq: 3, kind: 'text_delta', text_delta: '**你好**\n', status: 'running' },
    { seq: 4, kind: 'text_delta', text_delta: '🌏\n\n```ts\n', status: 'running' },
    { seq: 5, kind: 'status', text_delta: null, status: 'canceled' },
  ];
  let state = appendChatEvents(emptyChatStream(), events.slice(0, 3));
  state = appendChatEvents(state, events.slice(2));
  assert.equal(state.text, '**你好**\n🌏\n\n```ts\n');
  assert.equal(state.status, 'canceled');
  assert.equal(appendChatEvents(state, events), state);
});

test('a missing event is rejected atomically so retry cannot silently lose text', () => {
  const state = emptyChatStream();
  assert.throws(
    () =>
      appendChatEvents(state, [
        { seq: 1, kind: 'status', text_delta: null, status: 'running' },
        { seq: 3, kind: 'text_delta', text_delta: 'lost prefix', status: 'running' },
      ]),
    /gap/,
  );
  assert.equal(state.seq, 0);
  assert.equal(state.text, '');
});

test('a canceled stream cannot be revived by a late response', () => {
  const state = { seq: 5, text: 'partial', status: 'canceled' };
  assert.throws(
    () =>
      appendChatEvents(state, [
        { seq: 6, kind: 'text_delta', text_delta: 'late', status: 'running' },
      ]),
    /terminal/,
  );
  assert.equal(state.text, 'partial');
});
