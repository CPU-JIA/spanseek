import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import {
  MAX_DOCX_PREVIEW_BYTES,
  SOURCE_RESOURCE_CHUNK_BYTES,
  isSafeDocxHref,
  sourceByteRanges,
} from '../src/sources/source-resource-bytes.ts';

test('large originals are assembled from bounded inclusive ranges', () => {
  assert.deepEqual(sourceByteRanges(10, 4), [
    { start: 0, end: 3 },
    { start: 4, end: 7 },
    { start: 8, end: 9 },
  ]);
  assert.deepEqual(sourceByteRanges(0), []);
  assert.equal(sourceByteRanges(MAX_DOCX_PREVIEW_BYTES).length, 8);
  assert.equal(SOURCE_RESOURCE_CHUNK_BYTES, 8 * 1024 * 1024);
  assert.throws(() => sourceByteRanges(-1), /source_resource_size/);
  assert.throws(() => sourceByteRanges(1, 0), /source_resource_chunk/);
});

test('only web, email, and internal Word links survive preview cleanup', () => {
  for (const href of [
    'https://example.com/a',
    'http://example.com',
    'mailto:a@example.com',
    '#bookmark',
  ])
    assert.equal(isSafeDocxHref(href), true, href);
  for (const href of [
    '',
    'javascript:alert(1)',
    'data:text/html,x',
    'file:///C:/Windows',
    'cmd:/c',
  ])
    assert.equal(isSafeDocxHref(href), false, href);
});

test('the Word view keeps layout parts but never renders executable altChunks', async () => {
  const view = await readFile(new URL('../src/sources/DocxReadView.tsx', import.meta.url), 'utf8');
  for (const option of [
    'breakPages: true',
    'ignoreWidth: false',
    'ignoreHeight: false',
    'ignoreFonts: false',
    'renderHeaders: true',
    'renderFooters: true',
    'renderFootnotes: true',
    'renderEndnotes: true',
    'renderComments: false',
    'experimental: false',
  ])
    assert.match(view, new RegExp(option.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), option);
  assert.match(view, /renderAltChunks: false/);
  assert.match(view, /sanitizeDocxPreview\(body\.current\)/);
  assert.match(view, /URL\.revokeObjectURL\(url\)/);
  assert.match(view, /ignoreLastRenderedPageBreak: false/);
});

test('DOCX opens in the rendered view and keeps the exact source view for excerpts', async () => {
  const reader = await readFile(
    new URL('../src/sources/SourceReader.tsx', import.meta.url),
    'utf8',
  );
  assert.match(reader, /version\.format === 'markdown'/);
  assert.match(reader, /version\.format === 'docx'/);
  assert.match(reader, /version\.format === 'doc'/);
  assert.match(reader, /<DocxReadView/);
  assert.match(reader, /t\('版式阅读', 'Word layout'\)/);
  assert.match(reader, /原文（可摘录）/);
  assert.match(reader, /onFallback=\{\(\) => setMode\('source'\)\}/);
});
