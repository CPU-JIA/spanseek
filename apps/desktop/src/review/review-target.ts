import type { ResolveLink, WorkspaceIdentity } from '@pkms/contracts';

/** ResolveLink has exactly these three identity fields; object key order is irrelevant. */
export function reviewTargetKey(target: ResolveLink): string {
  return JSON.stringify([target.target_id, target.target_block_id, target.target_revision_id]);
}

/**
 * One pending add per object/block, across head revisions. The stored command
 * retains its exact frozen revision. Including revision here would unmount the
 * button during prepareTarget's save and strand an unknown earlier receipt.
 */
export function reviewAddScope(target: ResolveLink): string {
  return JSON.stringify(['add', target.target_id, target.target_block_id]);
}

export function reviewAddComponentKey(identity: WorkspaceIdentity, target: ResolveLink): string {
  return JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    reviewAddScope(target),
  ]);
}
