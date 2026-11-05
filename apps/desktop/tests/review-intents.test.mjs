import test from 'node:test';
import assert from 'node:assert/strict';
import {
  confirmReviewIntent,
  snoozeTimestamp,
  storeReviewIntent,
  validateReviewIntent,
} from '../src/review/review-intents.ts';
const identity = { local_instance_id: 'local-a', workspace_id: 'workspace' };
const request = () => ({
  identity,
  command: {
    kind: 'review_snooze',
    payload: {
      command_id: 'fixed-command',
      request: { entry_id: 'entry', base_row_version: 4, due_at: '2030-01-02T10:00:00Z' },
    },
  },
});
function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}
test('unknown outcome retains exact review version, date and command across reload', () => {
  const disk = storage(),
    intent = request();
  storeReviewIntent(disk, 'pending', intent);
  intent.command.payload.request.base_row_version = 9;
  const restored = JSON.parse(disk.getItem('pending'));
  validateReviewIntent(restored, identity);
  assert.equal(restored.command.payload.request.base_row_version, 4);
  assert.equal(restored.command.payload.command_id, 'fixed-command');
  assert.equal(restored.command.payload.request.due_at, '2030-01-02T10:00:00Z');
  assert.throws(() => storeReviewIntent(disk, 'pending', request()));
  assert.throws(() => confirmReviewIntent(disk, 'pending', intent));
  assert.ok(disk.getItem('pending'));
  confirmReviewIntent(disk, 'pending', restored);
  assert.equal(disk.getItem('pending'), null);
});
test('a pending review cannot cross physical instance identities', () => {
  assert.throws(() =>
    validateReviewIntent(request(), { ...identity, local_instance_id: 'local-b' }),
  );
  assert.throws(() =>
    validateReviewIntent({ ...request(), command: { kind: 'notes_save' } }, identity),
  );
});

test('adding a review retains the exact committed content revision while later edits continue', () => {
  const disk = storage();
  const value = {
    identity,
    command: {
      kind: 'review_add',
      payload: {
        command_id: 'fixed-add',
        request: {
          target: {
            target_id: 'note',
            target_revision_id: 'committed-revision',
            target_block_id: null,
          },
          base_row_version: null,
        },
      },
    },
  };
  validateReviewIntent(value, identity);
  storeReviewIntent(disk, 'add', value);
  value.command.payload.request.target.target_revision_id = 'newer-revision';
  const recovered = JSON.parse(disk.getItem('add'));
  validateReviewIntent(recovered, identity);
  assert.equal(recovered.command.payload.request.target.target_revision_id, 'committed-revision');
  assert.equal(recovered.command.payload.request.base_row_version, null);
  assert.throws(() => confirmReviewIntent(disk, 'add', value));
  confirmReviewIntent(disk, 'add', recovered);
});
test('snooze uses local calendar time and rejects past or normalized invalid dates', () => {
  const now = new Date(2029, 0, 1).getTime();
  assert.equal(snoozeTimestamp('2030-01-02T10:00', now), new Date(2030, 0, 2, 10, 0).toISOString());
  assert.throws(() => snoozeTimestamp('2030-02-30T10:00', now));
  assert.throws(() => snoozeTimestamp('2028-01-02T10:00', now));
  assert.throws(() => snoozeTimestamp('2030-01-02', now));
});
