import type { DocumentNode, NoteDocument } from '@pkms/contracts';

function wikiNodes(document: NoteDocument): Map<string, DocumentNode> {
  const result = new Map<string, DocumentNode>();
  const visit = (nodes: DocumentNode[]) => {
    for (const node of nodes) {
      if ((node.type === 'wikiLink' || node.type === 'wikiEmbed') && node.attrs?.occurrence_id)
        result.set(node.attrs.occurrence_id, node);
      if (node.content) visit(node.content);
    }
  };
  visit(document.content);
  return result;
}

/** Carry a server-resolved link identity into newer drafts without replacing their edits. */
export function mergeCommittedBindings(
  current: NoteDocument,
  submitted: NoteDocument,
  committed: NoteDocument,
): NoteDocument {
  const before = wikiNodes(submitted);
  const after = wikiNodes(committed);
  let changed = false;
  const visit = (nodes: DocumentNode[]): DocumentNode[] =>
    nodes.map((node) => {
      let next = node;
      const occurrence = node.attrs?.occurrence_id;
      const original = occurrence ? before.get(occurrence) : undefined;
      const saved = occurrence ? after.get(occurrence) : undefined;
      if (
        original &&
        saved?.attrs?.target_id &&
        !original.attrs?.target_id &&
        !node.attrs?.target_id &&
        !node.attrs?.target_block_id &&
        !node.attrs?.target_revision_id &&
        original.type === node.type &&
        saved.type === node.type &&
        original.attrs?.wiki_target === node.attrs?.wiki_target &&
        saved.attrs.wiki_target === node.attrs?.wiki_target
      ) {
        changed = true;
        next = {
          ...node,
          attrs: {
            ...node.attrs,
            target_id: saved.attrs.target_id,
            ...(saved.attrs.target_block_id
              ? { target_block_id: saved.attrs.target_block_id }
              : {}),
            ...(saved.attrs.target_revision_id
              ? { target_revision_id: saved.attrs.target_revision_id }
              : {}),
          },
        };
      }
      if (node.content) {
        const content = visit(node.content);
        if (content.some((child, index) => child !== node.content![index]))
          next = { ...next, content };
      }
      return next;
    });
  const content = visit(current.content);
  return changed ? { ...current, content } : current;
}
