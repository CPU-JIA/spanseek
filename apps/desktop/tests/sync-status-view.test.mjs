import test from 'node:test';
import assert from 'node:assert/strict';
import { SYNC_STATUS_UI_TIMEOUT_MS, syncStatusTone } from '../src/portability/sync-status-view.ts';

const status = {
  configured: true,
  endpoint: 'http://127.0.0.1:8787',
  device_id: '11111111-1111-1111-1111-111111111111',
  workspace_id: 'workspace-a',
  healthy: true,
  revoked: false,
  schema_version: 27,
  protocol_version: 1,
};

test('sync badge distinguishes loading, unconfigured and unknown states', () => {
  assert.ok(SYNC_STATUS_UI_TIMEOUT_MS <= 4_000);
  assert.equal(syncStatusTone(null, 'workspace-a', null), 'loading');
  assert.equal(syncStatusTone(null, 'workspace-a', 'storage_busy'), 'unknown');
  assert.equal(
    syncStatusTone({ ...status, configured: false }, 'workspace-a', null),
    'unconfigured',
  );
});

test('sync badge distinguishes offline, unattached and online libraries', () => {
  assert.equal(syncStatusTone({ ...status, healthy: false }, 'workspace-a', null), 'offline');
  assert.equal(syncStatusTone({ ...status, revoked: true }, 'workspace-a', null), 'revoked');
  assert.equal(syncStatusTone(status, 'workspace-b', null), 'unattached');
  assert.equal(syncStatusTone(status, 'workspace-a', null), 'online');
});

test('an unconfigured profile wins over a stale workspace or health flag', () => {
  assert.equal(
    syncStatusTone(
      { ...status, configured: false, healthy: false, workspace_id: 'workspace-b' },
      'workspace-a',
      null,
    ),
    'unconfigured',
  );
});
