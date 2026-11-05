import test from 'node:test';
import assert from 'node:assert/strict';
import { readSelectedSourceTags } from '../src/sources/source-filter-tags.ts';

const tag = (id, display_name = id) => ({ id, display_name, normalized_name: display_name });

test('restored source filters recover selected tags across pages before adding an AND condition', async () => {
  const first = tag('a', '原有标签');
  const second = tag('b', '新增标签');
  const cursors = [];
  const selected = await readSelectedSourceTags(
    ['a'],
    async (cursor) => {
      cursors.push(cursor);
      return cursor === null
        ? { items: [tag('other')], next_cursor: 'page-2' }
        : { items: [first], next_cursor: 'unused-page-3' };
    },
    () => true,
  );
  assert.deepEqual(cursors, [null, 'page-2']);
  assert.deepEqual(selected, [first]);
  assert.deepEqual(
    [...selected, second].map((value) => value.id),
    ['a', 'b'],
  );
});

test('recovery returns exactly the current selected IDs in their requested order', async () => {
  assert.deepEqual(
    await readSelectedSourceTags(
      ['b', 'a'],
      async () => ({ items: [tag('a'), tag('unselected'), tag('b')], next_cursor: null }),
      () => true,
    ),
    [tag('b'), tag('a')],
  );
  let calls = 0;
  assert.deepEqual(
    await readSelectedSourceTags(
      [],
      async () => {
        calls++;
        throw Error('unexpected read');
      },
      () => true,
    ),
    [],
  );
  assert.equal(calls, 0);
});

test('an obsolete selection or workspace cannot publish a delayed page or request the next one', async () => {
  let current = true;
  let calls = 0;
  const result = await readSelectedSourceTags(
    ['a'],
    async () => {
      calls++;
      current = false;
      return { items: [tag('a')], next_cursor: 'page-2' };
    },
    () => current,
  );
  assert.equal(result, null);
  assert.equal(calls, 1);
});

test('failed or incomplete recovery never publishes a partial filter selection', async () => {
  await assert.rejects(
    readSelectedSourceTags(
      ['a', 'b'],
      async () => ({ items: [tag('a')], next_cursor: null }),
      () => true,
    ),
    /部分已选标签暂不可用/,
  );
  const failure = new Error('read failed');
  await assert.rejects(
    readSelectedSourceTags(
      ['a'],
      async () => {
        throw failure;
      },
      () => true,
    ),
    (error) => error === failure,
  );
});
