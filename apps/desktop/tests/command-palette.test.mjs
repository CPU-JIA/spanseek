import test from 'node:test';
import assert from 'node:assert/strict';
import {
  clampCommandPaletteIndex,
  filterCommandPalette,
  moveCommandPaletteIndex,
  shouldIgnoreCommandPaletteKey,
} from '../src/layout/command-palette.ts';

const items = [
  { id: 'new', label: '新建笔记', keywords: ['create', 'note'], run: () => {} },
  { id: 'auto', label: '自动化', description: '整理提案', keywords: ['automation'], run: () => {} },
  { id: 'settings', label: 'Settings', keywords: ['preferences'], run: () => {} },
];

test('empty query preserves command order', () => {
  assert.deepEqual(
    filterCommandPalette(items, '  ').map((item) => item.id),
    ['new', 'auto', 'settings'],
  );
});

test('filters Chinese labels, descriptions and English keywords case-insensitively', () => {
  assert.deepEqual(
    filterCommandPalette(items, '自动').map((item) => item.id),
    ['auto'],
  );
  assert.deepEqual(
    filterCommandPalette(items, '整理').map((item) => item.id),
    ['auto'],
  );
  assert.deepEqual(
    filterCommandPalette(items, 'NOTE').map((item) => item.id),
    ['new'],
  );
  assert.deepEqual(
    filterCommandPalette(items, 'PREF').map((item) => item.id),
    ['settings'],
  );
});

test('returns an empty list when nothing matches', () => {
  assert.deepEqual(filterCommandPalette(items, '没有这个命令'), []);
});

test('arrow navigation wraps and handles an empty result', () => {
  assert.equal(moveCommandPaletteIndex(0, 1, 3), 1);
  assert.equal(moveCommandPaletteIndex(2, 1, 3), 0);
  assert.equal(moveCommandPaletteIndex(0, -1, 3), 2);
  assert.equal(moveCommandPaletteIndex(-1, 1, 3), 0);
  assert.equal(moveCommandPaletteIndex(-1, -1, 3), 2);
  assert.equal(moveCommandPaletteIndex(0, 1, 0), -1);
});
test('IME composition never executes or closes the palette', () => {
  assert.equal(shouldIgnoreCommandPaletteKey({ isComposing: true }), true);
  assert.equal(shouldIgnoreCommandPaletteKey({ keyCode: 229 }), true);
  assert.equal(shouldIgnoreCommandPaletteKey({}, true), true);
  assert.equal(shouldIgnoreCommandPaletteKey({ isComposing: false, keyCode: 13 }), false);
});

test('the selected row stays inside a shrinking result set during the same render', () => {
  assert.equal(clampCommandPaletteIndex(4, 2), 1);
  assert.equal(clampCommandPaletteIndex(-1, 2), 0);
  assert.equal(clampCommandPaletteIndex(0, 0), -1);
  assert.equal(clampCommandPaletteIndex(2, 5), 2);
});
