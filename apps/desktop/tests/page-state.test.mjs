import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveListDisplayState } from '../src/layout/page-state.ts';

test('a list without a page or error is loading', () => {
  assert.equal(
    resolveListDisplayState({ hasPage: false, itemCount: 0, loading: false, error: null }),
    'loading',
  );
});

test('a first-page error is blocking instead of an empty result', () => {
  assert.equal(
    resolveListDisplayState({
      hasPage: false,
      itemCount: 0,
      loading: false,
      error: 'storage busy',
    }),
    'error',
  );
});

test('an arrived page with no rows is empty', () => {
  assert.equal(
    resolveListDisplayState({ hasPage: true, itemCount: 0, loading: false, error: null }),
    'empty',
  );
});

test('rows remain usable when a refresh fails', () => {
  assert.equal(
    resolveListDisplayState({ hasPage: true, itemCount: 3, loading: false, error: 'storage busy' }),
    'content-with-error',
  );
});

test('an empty refresh stays in the loading state', () => {
  assert.equal(
    resolveListDisplayState({ hasPage: true, itemCount: 0, loading: true, error: null }),
    'loading',
  );
});

test('rows without an error are ordinary content', () => {
  assert.equal(
    resolveListDisplayState({ hasPage: true, itemCount: 1, loading: false, error: null }),
    'content',
  );
});
