import type { WorkspaceIdentity } from '@pkms/contracts';

export type ReferencePanelTab = 'backlinks' | 'outgoing' | 'relation';

/**
 * What page 1 means for the reference panel.
 *
 * Every input here changes the rows, so each one belongs in the key: another
 * note, another saved revision, another tab or a rebuilt index has to start from
 * its own first page instead of replaying the previous cursor stack. A collapsed
 * panel reads nothing at all, which is what a `null` scope means to `usePagedList`.
 */
export function referencePanelScope(
  identity: WorkspaceIdentity,
  open: boolean,
  noteId: string,
  revisionId: string | null,
  tab: ReferencePanelTab,
  generation: number,
): string | null {
  if (!open) return null;
  return JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    noteId,
    // A property relation belongs to the note, not to one saved revision, so
    // browsing another revision must not restart that list.
    tab === 'relation' ? null : revisionId,
    tab,
    generation,
  ]);
}
