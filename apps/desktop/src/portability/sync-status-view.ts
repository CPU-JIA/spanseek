import type { SyncStatus } from '@pkms/contracts';

/** UI ceiling for a sync probe; the native health request has its own 1.5 s limit. */
export const SYNC_STATUS_UI_TIMEOUT_MS = 4_000;

export type SyncStatusTone =
  'loading' | 'unknown' | 'unconfigured' | 'offline' | 'revoked' | 'unattached' | 'online';

export function syncStatusTone(
  status: SyncStatus | null,
  workspaceId: string,
  error: string | null,
): SyncStatusTone {
  if (error) return 'unknown';
  if (!status) return 'loading';
  if (!status.configured) return 'unconfigured';
  if (status.revoked) return 'revoked';
  if (!status.healthy) return 'offline';
  if (status.workspace_id !== workspaceId) return 'unattached';
  return 'online';
}
