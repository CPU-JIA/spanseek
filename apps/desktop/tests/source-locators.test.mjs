import test from 'node:test';
import assert from 'node:assert/strict';
import { textLocator, pdfRect } from '../src/sources/locators.ts';

test('Unicode offsets preserve Chinese, emoji and repeated quotes', () => {
  const text = '开头😀引用 再次😀引用';
  const start = text.lastIndexOf('😀');
  const result = textLocator('version', 'block', text, start, text.length);
  assert.equal(result.quote, '😀引用');
  assert.equal(result.start_codepoint, 8);
  assert.equal(result.end_codepoint, 11);
  assert.equal(result.prefix, '开头😀引用 再次');
  assert.throws(() => textLocator('v', 'b', '😀', 1, 2), /Unicode/);
});
test('PDF inverse viewport handles rotated crop origins without guessing', () => {
  assert.deepEqual(
    pdfRect([10, 20, 30, 50], [100, 200, 500, 700], (x, y) => [100 + y / 2, 700 - x / 2]),
    [110, 685, 125, 695],
  );
  assert.deepEqual(
    pdfRect([-5, -5, 20, 30], [0, 0, 100, 100], (x, y) => [x, y]),
    [0, 0, 20, 30],
  );
  assert.throws(() => pdfRect([200, 200, 220, 230], [0, 0, 100, 100], (x, y) => [x, y]));
});
