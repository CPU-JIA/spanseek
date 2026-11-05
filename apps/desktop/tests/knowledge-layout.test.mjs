import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const css = await readFile(
  new URL('../src/organization/knowledge-index.css', import.meta.url),
  'utf8',
);

test('short viewports keep the batch bar above the pinned pager', () => {
  assert.match(
    css,
    /\.knowledge-list-panel:has\(\.knowledge-batch\)\s*\{[^}]*overflow:\s*visible/s,
  );
  assert.match(
    css,
    /\.knowledge-list-panel:has\(\.knowledge-batch\)\s*>\s*\.knowledge-batch\s*\{[^}]*position:\s*absolute;[^}]*bottom:\s*0/s,
  );
});
