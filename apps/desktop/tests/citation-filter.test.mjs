import test from 'node:test';
import assert from 'node:assert/strict';
import { citationLabel, filterCitations } from '../src/citations/citation-filter.ts';

const entries = [
  {
    key: 'vaswani2017',
    title: 'Attention is All You Need',
    formatted: 'Vaswani, A. (2017). Attention is All You Need. [C]',
  },
  { key: 'devlin2019', title: 'BERT', formatted: 'Devlin, J. (2019). BERT. [C]' },
  { key: 'cslonly', title: 'CSL only', formatted: '' },
];

test('an empty query keeps the catalog order', () => {
  assert.deepEqual(filterCitations(entries, '   '), entries);
});

test('every whitespace-separated term must match key, title or formatted line', () => {
  assert.deepEqual(
    filterCitations(entries, 'vaswani').map((entry) => entry.key),
    ['vaswani2017'],
  );
  assert.deepEqual(
    filterCitations(entries, '2019 bert').map((entry) => entry.key),
    ['devlin2019'],
  );
  assert.deepEqual(
    filterCitations(entries, 'attention 2017').map((entry) => entry.key),
    ['vaswani2017'],
  );
  assert.deepEqual(filterCitations(entries, 'attention 2019'), []);
  assert.deepEqual(
    filterCitations(entries, 'bert').map((entry) => entry.key),
    ['devlin2019'],
  );
});

test('the label falls back from formatted line to title and then key', () => {
  assert.equal(citationLabel(entries[0]), entries[0].formatted);
  assert.equal(citationLabel(entries[2]), 'CSL only');
  assert.equal(citationLabel({ key: 'k', title: '  ', formatted: '' }), 'k');
});
