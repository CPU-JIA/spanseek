import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldSendChat } from '../src/inference/chat-keyboard.ts';

const enter = {
  key: 'Enter',
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  altKey: false,
  isComposing: false,
  keyCode: 13,
};
test('composer keeps Enter for multiline text and sends only the documented modifier shortcut', () => {
  assert.equal(shouldSendChat(enter), false);
  assert.equal(shouldSendChat({ ...enter, shiftKey: true }), false);
  assert.equal(shouldSendChat({ ...enter, ctrlKey: true }), true);
  assert.equal(shouldSendChat({ ...enter, metaKey: true }), true);
  assert.equal(shouldSendChat({ ...enter, ctrlKey: true, altKey: true }), false);
});
test('IME composition and legacy 229 confirmation cannot send a draft', () => {
  assert.equal(shouldSendChat({ ...enter, ctrlKey: true, isComposing: true }), false);
  assert.equal(shouldSendChat({ ...enter, ctrlKey: true, keyCode: 229 }), false);
});
