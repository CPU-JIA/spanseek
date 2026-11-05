import { useEffect, useRef } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import type { SyncTagPushBatch, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { announceSyncStatusChanged } from './sync-status-events';
import {
  announceTagChange,
  sameTagSyncScope,
  TAG_LOCAL_CHANGED_EVENT,
  TAG_METADATA_CHANGED_EVENT,
} from './tag-sync-events';

/** One request at a time; later edits and reconnection schedule another pass. */
export function useTagAutoSync(
  identity: WorkspaceIdentity | null,
  onResult: (result: SyncTagPushBatch) => void,
  onFailure: (failure: unknown) => void,
) {
  const callbacks = useRef({ onResult, onFailure, identity });
  callbacks.current = { onResult, onFailure, identity };
  const instance = identity?.local_instance_id;
  const workspace = identity?.workspace_id;
  useEffect(() => {
    if (!isTauri() || !instance || !workspace) return;
    const scope = { local_instance_id: instance, workspace_id: workspace };
    let closed = false;
    let running = false;
    let queued = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = (delay: number) => {
      if (closed) return;
      if (running) {
        queued = true;
        return;
      }
      clearTimeout(timer);
      timer = setTimeout(() => void run(), delay);
    };
    const run = async () => {
      if (closed || running) return;
      if (document.visibilityState === 'hidden' || !navigator.onLine) {
        schedule(30_000);
        return;
      }
      running = true;
      queued = false;
      let nextDelay = 30_000;
      try {
        // No UI timeout: an unacknowledged native operation keeps this slot.
        const result = await call({ kind: 'sync_auto_push_tags' }, 'sync_tag_push', scope);
        if (closed || !sameTagSyncScope(callbacks.current.identity, scope)) return;
        if (result.configured) {
          if (result.changed_objects.length || result.received > 0)
            announceTagChange(TAG_METADATA_CHANGED_EVENT, scope);
          if (result.pending || result.dirty) announceSyncStatusChanged();
        }
        callbacks.current.onResult(result);
      } catch (failure) {
        if (closed || !sameTagSyncScope(callbacks.current.identity, scope)) return;
        nextDelay = 15_000;
        announceSyncStatusChanged();
        callbacks.current.onFailure(failure);
      } finally {
        running = false;
        if (!closed) schedule(queued ? 300 : nextDelay);
      }
    };
    const changed = (event: Event) => {
      if (sameTagSyncScope((event as CustomEvent<WorkspaceIdentity>).detail, scope)) schedule(500);
    };
    const resume = () => schedule(300);
    window.addEventListener(TAG_LOCAL_CHANGED_EVENT, changed);
    window.addEventListener('online', resume);
    document.addEventListener('visibilitychange', resume);
    schedule(1_500);
    return () => {
      closed = true;
      clearTimeout(timer);
      window.removeEventListener(TAG_LOCAL_CHANGED_EVENT, changed);
      window.removeEventListener('online', resume);
      document.removeEventListener('visibilitychange', resume);
    };
  }, [instance, workspace]);
}
