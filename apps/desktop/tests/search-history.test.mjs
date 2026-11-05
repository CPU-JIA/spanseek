import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SEARCH_HISTORY_LIMIT,
  parseSearchHistory,
  recordSearchHistory,
  searchHistoryEntry,
  searchHistoryStorageKey,
  searchQueryFromHistory,
  serializeSearchHistory,
} from '../src/inference/search-history.ts';

const scope = {
  object_ids: [],
  kinds: [],
  tag_ids: [],
  collection_id: null,
  inbox_state: null,
  reading_status: null,
  source_format: null,
  archived: null,
};

function query(text, overrides = {}) {
  return { query: text, mode: 'hybrid', scope, limit: 8, minimum_cosine: null, ...overrides };
}

function entry(id, text, resultCount = 3, overrides = {}) {
  return searchHistoryEntry({
    id,
    query: query(text, overrides),
    order: 'fusion',
    resultCount,
    searchedAt: `2026-10-21T00:00:${String(Number(id) || 0).padStart(2, '0')}.000Z`,
  });
}

test('history entries round-trip without storing result bodies', () => {
  const value = entry('01', '证据整理');
  const parsed = parseSearchHistory(serializeSearchHistory([value]));
  assert.deepEqual(parsed, [value]);
  assert.equal('items' in parsed[0], false);
});

test('recording the same query and scope moves the newest entry to the top', () => {
  const first = entry('01', '证据整理', 5);
  const second = entry('02', '研究方法', 2);
  const repeated = entry('03', '证据整理', 4);
  const history = recordSearchHistory(recordSearchHistory([first], second), repeated);
  assert.deepEqual(
    history.map((value) => value.id),
    ['03', '02'],
  );
  assert.equal(history[0].result_count, 4);
});

test('history is capped and malformed local storage cannot inject entries', () => {
  let history = [];
  for (let index = 0; index < SEARCH_HISTORY_LIMIT + 5; index += 1) {
    history = recordSearchHistory(history, entry(String(index + 1), `query ${index}`));
  }
  assert.equal(history.length, SEARCH_HISTORY_LIMIT);
  assert.equal(history[0].query, `query ${SEARCH_HISTORY_LIMIT + 4}`);
  assert.deepEqual(parseSearchHistory('{broken'), []);
  assert.deepEqual(
    parseSearchHistory(
      JSON.stringify({
        version: 1,
        entries: [
          { ...entry('01', 'valid'), mode: 'future-mode' },
          { ...entry('02', 'valid'), query: '' },
        ],
      }),
    ),
    [],
  );
});

test('history is scoped to one physical workspace', () => {
  const left = searchHistoryStorageKey({
    local_instance_id: 'instance-a',
    workspace_id: 'workspace-a',
  });
  const right = searchHistoryStorageKey({
    local_instance_id: 'instance-a',
    workspace_id: 'workspace-b',
  });
  assert.notEqual(left, right);
  assert.match(left, /search-history/);
});

test('recalling a query restores its scope without sharing the stored object', () => {
  const value = entry('01', '研究方法', 2, {
    mode: 'lexical',
    limit: 12,
    minimum_cosine: 0.35,
    scope: { ...scope, tag_ids: ['tag-1'], kinds: ['note'] },
  });
  const recalled = searchQueryFromHistory(value);
  assert.equal(recalled.mode, 'lexical');
  assert.equal(recalled.limit, 12);
  assert.equal(recalled.minimum_cosine, 0.35);
  assert.deepEqual(recalled.scope.tag_ids, ['tag-1']);
  recalled.scope.tag_ids.push('tag-2');
  assert.deepEqual(value.scope.tag_ids, ['tag-1']);
});

test('every supported source format survives history scope validation', () => {
  for (const format of [
    'pdf',
    'html',
    'text',
    'markdown',
    'docx',
    'doc',
    'epub',
    'mobi',
    'csv',
    'xlsx',
    'xls',
    'pptx',
  ]) {
    const value = entry('01', `query ${format}`, 1, {
      scope: { ...scope, source_format: format },
    });
    const [parsed] = parseSearchHistory(serializeSearchHistory([value]));
    assert.equal(parsed.scope.source_format, format);
  }
});
