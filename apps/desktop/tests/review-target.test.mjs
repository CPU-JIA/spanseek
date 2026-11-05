import test from 'node:test';
import assert from 'node:assert/strict';
import {
  reviewAddComponentKey,
  reviewAddScope,
  reviewTargetKey,
} from '../src/review/review-target.ts';

test('UI target and Rust reply compare equal regardless of property insertion order', () => {
  const ui = { target_id: 'note', target_revision_id: 'saved-revision', target_block_id: null };
  const rust = { target_id: 'note', target_block_id: null, target_revision_id: 'saved-revision' };
  assert.equal(reviewTargetKey(ui), reviewTargetKey(rust));
  assert.notEqual(
    reviewTargetKey(ui),
    reviewTargetKey({ ...rust, target_revision_id: 'newer-revision' }),
  );
  assert.notEqual(
    reviewTargetKey(ui),
    reviewTargetKey({ ...rust, target_block_id: 'another-block' }),
  );
  assert.notEqual(reviewTargetKey(ui), reviewTargetKey({ ...rust, target_id: 'annotation-id' }));
  assert.notEqual(reviewTargetKey(ui), reviewTargetKey({ ...rust, target_revision_id: null }));
});

test('save-induced head changes retain the pending-add scope while confirmed targets stay version-specific', () => {
  const identity = { local_instance_id: 'instance', workspace_id: 'workspace' };
  const beforeSave = { target_id: 'note', target_block_id: null, target_revision_id: 'old' };
  const saved = { target_revision_id: 'saved', target_block_id: null, target_id: 'note' };
  assert.equal(reviewAddScope(beforeSave), reviewAddScope(saved));
  assert.equal(
    reviewAddComponentKey(identity, beforeSave),
    reviewAddComponentKey({ workspace_id: 'workspace', local_instance_id: 'instance' }, saved),
  );
  assert.notEqual(reviewTargetKey(beforeSave), reviewTargetKey(saved));
  assert.notEqual(
    reviewAddComponentKey(identity, saved),
    reviewAddComponentKey({ ...identity, local_instance_id: 'other' }, saved),
  );
  assert.notEqual(
    reviewAddComponentKey(identity, saved),
    reviewAddComponentKey({ ...identity, workspace_id: 'other' }, saved),
  );
});
