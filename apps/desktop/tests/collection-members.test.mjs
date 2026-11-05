import test from 'node:test';
import assert from 'node:assert/strict';
import { collectionMemberTarget } from '../src/organization/collection-member-model.ts';
import { reorderedMembers } from '../src/organization/collection-order.ts';

const member = (id, kind, extra = {}) => ({
  position: 0,
  object: { id, kind, trashed: false, search_enabled: true },
  note: null,
  source: null,
  annotation: null,
  ...extra,
});

test('source members open the displayed source version and do not fall back to the current version', () => {
  const source = member('source-1', 'source', { source: { current_version_id: 'version-shown' } });
  assert.deepEqual(collectionMemberTarget(source), {
    target_id: 'source-1',
    target_revision_id: 'version-shown',
    target_block_id: null,
  });
  assert.equal(collectionMemberTarget(member('source-1', 'source')), null);
});

test('excerpt members resolve their own locator rather than opening the whole parent source', () => {
  const excerpt = member('excerpt-1', 'annotation', {
    annotation: {
      source_id: 'source-1',
      source_version_id: 'historical-version',
      source_trashed: false,
    },
  });
  assert.deepEqual(collectionMemberTarget(excerpt), {
    target_id: 'excerpt-1',
    target_revision_id: null,
    target_block_id: null,
  });
  assert.equal(
    collectionMemberTarget({
      ...excerpt,
      annotation: { ...excerpt.annotation, source_trashed: true },
    }),
    null,
  );
});

test('search-disabled content stays readable while trashed members cannot open', () => {
  const note = member('note-1', 'note', { note: { title: 'Note' } });
  note.object.search_enabled = false;
  assert.equal(collectionMemberTarget(note).target_id, 'note-1');
  note.object.trashed = true;
  assert.equal(collectionMemberTarget(note), null);
});

test('reordering walks all pages and preserves source, excerpt and trashed members', async () => {
  const first = [member('note-1', 'note'), member('source-1', 'source')];
  const last = [
    member('excerpt-1', 'annotation'),
    member('trash-1', 'note', { object: { id: 'trash-1', kind: 'note', trashed: true } }),
  ];
  const cursors = [];
  const ids = await reorderedMembers(
    async (cursor) => {
      cursors.push(cursor);
      return {
        collection_row_version: 12,
        items: cursor ? last : first,
        next_cursor: cursor ? null : 'next-page',
      };
    },
    12,
    'excerpt-1',
    -1,
  );
  assert.deepEqual(cursors, [null, 'next-page']);
  assert.deepEqual(ids, ['note-1', 'excerpt-1', 'source-1', 'trash-1']);
});
