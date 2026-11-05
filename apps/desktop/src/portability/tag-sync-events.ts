import type { WorkspaceIdentity } from '@pkms/contracts';

export const TAG_LOCAL_CHANGED_EVENT = 'spanseek:tag-local-changed';
export const TAG_METADATA_CHANGED_EVENT = 'spanseek:tag-metadata-changed';

export function sameTagSyncScope(a: WorkspaceIdentity | null, b: WorkspaceIdentity | null) {
  return (
    !!a && !!b && a.local_instance_id === b.local_instance_id && a.workspace_id === b.workspace_id
  );
}

export function announceTagChange(event: string, identity: WorkspaceIdentity) {
  if (typeof window !== 'undefined')
    window.dispatchEvent(new CustomEvent<WorkspaceIdentity>(event, { detail: { ...identity } }));
}
