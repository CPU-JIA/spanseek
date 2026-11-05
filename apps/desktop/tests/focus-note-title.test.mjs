import test from 'node:test';
import assert from 'node:assert/strict';
import { focusActiveNoteTitle } from '../src/editor/focus-note-title.ts';

test('focuses the active pane title and selects only when requested', () => {
  const calls = [];
  const title = {
    focus: (options) => calls.push(['focus', options]),
    select: () => calls.push(['select']),
  };
  globalThis.document = {
    querySelector: (selector) => {
      calls.push(['query', selector]);
      return title;
    },
  };
  focusActiveNoteTitle({ select: true });
  assert.deepEqual(calls, [
    ['query', '.content-pane.active-pane input.note-title'],
    ['focus', { preventScroll: true }],
    ['select'],
  ]);
  calls.length = 0;
  focusActiveNoteTitle();
  assert.equal(
    calls.some((call) => call[0] === 'select'),
    false,
  );
});

test('retries until the editor title mounts', () => {
  let tries = 0;
  const calls = [];
  const title = {
    focus: () => calls.push('focus'),
    select: () => calls.push('select'),
  };
  globalThis.document = {
    querySelector: () => (++tries === 3 ? title : null),
  };
  globalThis.window = {
    setTimeout: (callback) => {
      callback();
      return 0;
    },
  };
  focusActiveNoteTitle({ select: true });
  assert.equal(tries, 3);
  assert.deepEqual(calls, ['focus', 'select']);
});
