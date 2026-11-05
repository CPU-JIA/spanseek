import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_NEW_NOTE_SHORTCUT,
  DEFAULT_SEARCH_SHORTCUT,
  SHORTCUT_OPTIONS,
  matchShortcut,
  normalizedShortcut,
  shortcutParts,
} from '../src/layout/shortcuts.ts';

function press(key, modifiers = {}) {
  return {
    key,
    keyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    isComposing: false,
    ...modifiers,
  };
}

test('a shortcut matches only its own exact key combination', () => {
  assert.equal(matchShortcut('Ctrl+N', press('n', { ctrlKey: true })), true);
  assert.equal(matchShortcut('Ctrl+N', press('N', { ctrlKey: true, shiftKey: true })), false);
  assert.equal(matchShortcut('Ctrl+Shift+N', press('N', { ctrlKey: true, shiftKey: true })), true);
  assert.equal(matchShortcut('Ctrl+N', press('n')), false);
  assert.equal(matchShortcut('Ctrl+N', press('n', { ctrlKey: true, altKey: true })), false);
  assert.equal(matchShortcut('Alt+N', press('n', { altKey: true })), true);
  assert.equal(matchShortcut('Alt+N', press('n', { ctrlKey: true, altKey: true })), false);
  // Meta stands in for Ctrl so a mac keyboard behaves the same way.
  assert.equal(matchShortcut('Ctrl+K', press('k', { metaKey: true })), true);
});

test('a composing input method never triggers a shortcut', () => {
  assert.equal(matchShortcut('Ctrl+N', press('n', { ctrlKey: true, isComposing: true })), false);
  assert.equal(matchShortcut('Ctrl+N', press('n', { ctrlKey: true, keyCode: 229 })), false);
});

test('every offered shortcut is well formed and distinct', () => {
  assert.equal(new Set(SHORTCUT_OPTIONS).size, SHORTCUT_OPTIONS.length);
  for (const option of SHORTCUT_OPTIONS) {
    const parts = shortcutParts(option);
    assert.ok(parts.length >= 2, option);
    assert.equal(parts.at(-1).length, 1, option);
    // Each option must fire for its own combination and nothing else.
    const modifiers = {
      ctrlKey: parts.includes('Ctrl'),
      shiftKey: parts.includes('Shift'),
      altKey: parts.includes('Alt'),
    };
    assert.equal(matchShortcut(option, press(parts.at(-1).toLowerCase(), modifiers)), true, option);
  }
});

test('an unknown stored value falls back to the default instead of matching nothing', () => {
  assert.equal(normalizedShortcut('Ctrl+N', DEFAULT_SEARCH_SHORTCUT), 'Ctrl+N');
  assert.equal(normalizedShortcut(undefined, DEFAULT_NEW_NOTE_SHORTCUT), 'Ctrl+N');
  assert.equal(normalizedShortcut('Ctrl+Q', DEFAULT_SEARCH_SHORTCUT), DEFAULT_SEARCH_SHORTCUT);
});
