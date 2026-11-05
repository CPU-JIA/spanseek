import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { formatSourceSize } from '../src/sources/source-size.ts';

// The reader used to print `${(version.size / 1024).toFixed(1)} KB` for every
// original, so the only unit the interface ever offered was kilobytes: an 8 MiB
// PDF read as "8192.0 KB" and a 200 MiB scan as "204800.0 KB" even though the
// import dialog states its limits in MiB. These cases are what that expression
// cannot produce.
test('a document size picks the unit that keeps the number readable', () => {
  assert.equal(formatSourceSize(0), '0 B');
  assert.equal(formatSourceSize(512), '512 B');
  assert.equal(formatSourceSize(1023), '1023 B');
  assert.equal(formatSourceSize(1024), '1.0 KB');
  // The V1.7 native check read this file as "大小 1.6 KB"; the same wording stays.
  assert.equal(formatSourceSize(1638), '1.6 KB');
  assert.equal(formatSourceSize(1024 ** 2), '1.0 MB');
  assert.equal(formatSourceSize(8 * 1024 ** 2), '8.0 MB');
  assert.equal(formatSourceSize(255 * 1024 ** 2), '255.0 MB');
  assert.equal(formatSourceSize(1024 ** 3), '1.0 GB');
  assert.equal(formatSourceSize(1024 ** 3 + 512 * 1024 ** 2), '1.5 GB');
});

test('a megabyte-sized original no longer reads as a kilobyte count', () => {
  const pdf = 8 * 1024 ** 2;
  assert.notEqual(formatSourceSize(pdf), `${(pdf / 1024).toFixed(1)} KB`);
  assert.notEqual(formatSourceSize(pdf), '8192.0 KB');
  assert.ok(
    !formatSourceSize(pdf).includes('KB'),
    'a mebibyte-sized original must leave KB behind',
  );
});

test('a size the kernel could not report reads as a dash instead of NaN', () => {
  for (const value of [Number.NaN, Number.POSITIVE_INFINITY, -1]) {
    assert.equal(formatSourceSize(value), '—');
  }
});

// The unit switch only helps if the reader actually asks for it, so the inline
// expression is locked out: a future edit that divides by 1024 again would fail
// here rather than quietly returning to "8192.0 KB".
test('the reader formats the original size through the shared helper', () => {
  const source = readFileSync(new URL('../src/sources/SourceReader.tsx', import.meta.url), 'utf8');
  assert.ok(
    source.includes('formatSourceSize(version.size)'),
    'the reader must format its size through formatSourceSize',
  );
  assert.ok(
    !/\/\s*1024\)\s*\.toFixed\(/.test(source),
    'the reader must not print a raw kilobyte count',
  );
  assert.ok(!/' KB'/.test(source), 'the reader must not hard-code the KB unit');
});
