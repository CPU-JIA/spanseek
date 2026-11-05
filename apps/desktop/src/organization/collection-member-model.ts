import type { CollectionMember, ResolveLink } from '@pkms/contracts';

/** A source opens the exact version shown; an annotation resolves its own frozen locator. */
export function collectionMemberTarget(member: CollectionMember): ResolveLink | null {
  if (member.object.trashed || member.annotation?.source_trashed) return null;
  if (member.object.kind === 'annotation' && member.annotation)
    return { target_id: member.object.id, target_revision_id: null, target_block_id: null };
  if (member.object.kind === 'source' && member.source)
    return {
      target_id: member.object.id,
      target_revision_id: member.source.current_version_id,
      target_block_id: null,
    };
  if (member.object.kind === 'note' && member.note)
    return { target_id: member.object.id, target_revision_id: null, target_block_id: null };
  return null;
}
