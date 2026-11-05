import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../src/styles.css', import.meta.url), 'utf8');

test('generic hover stays low-specificity and excludes primary and secondary actions', () => {
  const hover = css.match(/button:hover:where\([\s\S]*?\{[^}]+\}/)?.[0];
  assert.ok(hover, 'neutral hover must not outrank component hover states');
  assert.match(hover, /\.primary-button/);
  assert.match(hover, /\.secondary-button/);
  assert.match(hover, /:disabled/);
});

test('primary actions define visible hover and pressed text without relying on inheritance', () => {
  assert.match(css, /\.primary-button:hover:not\(:disabled\)\s*\{[^}]*color:\s*var\(--surface\);/s);
  assert.match(
    css,
    /\.primary-button:active:not\(:disabled\)\s*\{[^}]*color:\s*var\(--surface\);/s,
  );
  assert.match(css, /\.secondary-button:hover:not\(:disabled\)\s*\{[^}]*color:\s*var\(--text\);/s);
});
