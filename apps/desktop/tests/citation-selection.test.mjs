import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CITATION_LOCATOR_OPTIONS,
  citationMarker,
  formatCitationLocator,
} from '../src/citations/citation-selection.ts';

test('an empty selection has no insertable marker', () => {
  assert.equal(citationMarker([]), '');
  assert.equal(citationMarker([{ key: '   ', locatorKind: 'none', locatorValue: '' }]), '');
});

test('page ranges and lists use pp. while a single page uses p.', () => {
  assert.equal(formatCitationLocator('page', '9'), 'p. 9');
  assert.equal(formatCitationLocator('page', '3-4'), 'pp. 3-4');
  assert.equal(formatCitationLocator('page', '3–4'), 'pp. 3–4');
  assert.equal(formatCitationLocator('page', '3, 9'), 'pp. 3, 9');
  assert.equal(formatCitationLocator('page', '3 & 9'), 'pp. 3 & 9');
});

test('typed locator labels become canonical CSL prefixes', () => {
  assert.equal(formatCitationLocator('chapter', '2'), 'ch. 2');
  assert.equal(formatCitationLocator('section', '4.1'), 'sec. 4.1');
  assert.equal(formatCitationLocator('figure', '3'), 'fig. 3');
  assert.equal(formatCitationLocator('sub-verbo', 'habeas corpus'), 's.v. habeas corpus');
});

test('custom and Chinese locator values remain verbatim', () => {
  assert.equal(formatCitationLocator('custom', '图 3'), '图 3');
  assert.equal(
    citationMarker([{ key: 'raw', locatorKind: 'custom', locatorValue: '图 3' }]),
    '[@raw, 图 3]',
  );
});

test('multiple selections form one cluster without duplicate keys', () => {
  assert.equal(
    citationMarker([
      { key: 'vaswani2017', locatorKind: 'page', locatorValue: '3-4' },
      { key: 'vaswani2017', locatorKind: 'page', locatorValue: '9' },
      { key: 'devlin2019', locatorKind: 'chapter', locatorValue: ' 2 ' },
      { key: 'raw', locatorKind: 'custom', locatorValue: '' },
    ]),
    '[@vaswani2017, pp. 3-4; @devlin2019, ch. 2; @raw]',
  );
});

test('every selectable locator except none/custom carries a kernel prefix', () => {
  for (const option of CITATION_LOCATOR_OPTIONS) {
    if (option.value === 'none' || option.value === 'custom')
      assert.equal(option.prefix, undefined);
    else assert.ok(option.prefix, `${option.value} has no prefix`);
  }
});
