import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(new URL('../src/styles.css', import.meta.url), 'utf8');

test('core navigation uses semantic tone tokens in light and dark themes', () => {
  for (const token of [
    '--nav-knowledge',
    '--nav-inbox',
    '--nav-sources',
    '--nav-graph',
    '--nav-neutral',
  ]) {
    assert.match(css, new RegExp(`${token}:\\s*#[0-9a-f]{6}`, 'i'), token);
  }
  assert.match(css, /:root\[data-theme='dark'\]\s*\{[\s\S]*--nav-knowledge:\s*#78b7ff;/);
});

test('navigation hover, selected and keyboard focus are independent channels', () => {
  assert.match(css, /\.nav-item:hover:not\(:disabled\):not\(\.active\)\s*\{[^}]*background:/s);
  assert.match(css, /\.nav-item\.active\s*\{[^}]*background:[^}]*font-weight:\s*600;/s);
  assert.match(css, /\.nav-item\.active::before\s*\{[^}]*width:\s*2px;/s);
  assert.match(css, /\.nav-item:focus-visible[\s\S]*outline:\s*2px solid var\(--focus-ring\)/);
});

test('navigation icons no longer use hard-coded gradients', () => {
  const navRules = css.slice(css.indexOf('.nav-item {'), css.indexOf('.local-indicator {'));
  assert.doesNotMatch(navRules, /linear-gradient/);
  assert.match(navRules, /color:\s*var\(--nav-tone, var\(--nav-neutral\)\)/);
});

test('the More group is collapsed unless a secondary screen is active', () => {
  assert.match(css, /\.nav-more-items\s*\{[^}]*flex-direction:\s*column;/s);
  assert.match(
    css,
    /\.nav-more\[open\] > \.nav-more-trigger \.nav-more-chevron\s*\{[^}]*rotate\(180deg\)/s,
  );
});
