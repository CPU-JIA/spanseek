export const SYNC_STATUS_CHANGED_EVENT = 'spanseek:sync-status-changed';
export const SYNC_PULLED_EVENT = 'spanseek:sync-pulled';

export function announceSyncStatusChanged(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(SYNC_STATUS_CHANGED_EVENT));
}

export type SyncPulledDetail = { identity: WorkspaceIdentity; applied: string[] };

export function announceSyncPulled(applied: string[], identity: WorkspaceIdentity): void {
  if (typeof window !== 'undefined')
    window.dispatchEvent(
      new CustomEvent<SyncPulledDetail>(SYNC_PULLED_EVENT, {
        detail: { identity: { ...identity }, applied: [...new Set(applied)] },
      }),
    );
}
import type { WorkspaceIdentity } from '@pkms/contracts';
