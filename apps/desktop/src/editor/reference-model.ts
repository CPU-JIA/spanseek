import type { DocumentNode, NoteDocument, ResolveLink } from '@pkms/contracts';

export type OpenReference = (target: ResolveLink, occurrenceId?: string) => Promise<void>;
export type ReferenceLocation = { requestId: string; target: ResolveLink; occurrenceId?: string };
export type ReferenceBlock = { id: string; text: string; kind: string };
export type ReferenceInteraction = 'edit' | 'read';

/** A pointer click in an editable atom selects it; keyboard activation still opens it. */
export function opensReference(
  interaction: ReferenceInteraction,
  event: { ctrlKey: boolean; metaKey: boolean; detail: number },
): boolean {
  return interaction === 'read' || event.ctrlKey || event.metaKey || event.detail === 0;
}

export function referenceText(node: DocumentNode): string {
  if (node.type === 'comment' || node.type === 'commentBlock') return '';
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'noteReference' || node.type === 'wikiLink')
    return node.attrs?.label ?? node.attrs?.wiki_target ?? '';
  if (node.type === 'inlineMath') return node.attrs?.source ?? '';
  if (node.type === 'hardBreak') return '\n';
  return (node.content ?? []).map(referenceText).join('');
}

/** Only text blocks are selectable; IDs are never reconstructed from labels. */
export function referenceBlocks(document: NoteDocument): ReferenceBlock[] {
  const blocks: ReferenceBlock[] = [];
  const visit = (nodes: DocumentNode[]) => {
    for (const node of nodes) {
      if (node.type === 'comment' || node.type === 'commentBlock') continue;
      if (node.attrs?.id && ['paragraph', 'heading', 'codeBlock'].includes(node.type)) {
        const text = referenceText(node).trim();
        if (text) blocks.push({ id: node.attrs.id, text, kind: node.type });
      }
      if (node.content) visit(node.content);
    }
  };
  visit(document.content);
  return blocks;
}

export function referenceNode(
  target: ResolveLink,
  label: string,
  alias?: string,
  embedded = false,
): DocumentNode {
  const displayAlias = alias?.trim();
  if (embedded)
    return {
      type: 'wikiEmbed',
      attrs: {
        id: crypto.randomUUID(),
        wiki_target: target.target_id,
        target_id: target.target_id,
        target_block_id: target.target_block_id,
        ...(target.target_revision_id ? { target_revision_id: target.target_revision_id } : {}),
        occurrence_id: crypto.randomUUID(),
        label: displayAlias || label,
      },
    };
  return {
    // Wiki labels are explicit aliases; legacy noteReference labels remain title snapshots.
    // Reuse that existing distinction so renamed legacy references still follow their target.
    type: displayAlias ? 'wikiLink' : 'noteReference',
    attrs: {
      ...(displayAlias ? { wiki_target: target.target_id } : {}),
      target_id: target.target_id,
      target_block_id: target.target_block_id,
      ...(target.target_revision_id ? { target_revision_id: target.target_revision_id } : {}),
      occurrence_id: crypto.randomUUID(),
      label: displayAlias || label,
    },
  };
}

/** Scope anchors to the receiving pane; never match the other pane or a same-name note. */
export function referenceAnchor(
  root: ParentNode,
  blockId: string | null,
  occurrenceId?: string,
): HTMLElement | null {
  const attribute = occurrenceId ? 'data-reference-occurrence' : 'data-id';
  const id = occurrenceId ?? blockId;
  if (!id) return null;
  return (
    Array.from(root.querySelectorAll<HTMLElement>(`[${attribute}]`)).find(
      (element) => element.getAttribute(attribute) === id,
    ) ?? null
  );
}

export function revealReference(element: HTMLElement): () => void {
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (parent instanceof HTMLDetailsElement) parent.open = true;
  }
  element.scrollIntoView({ block: 'center', behavior: 'instant' });
  element.classList.add('reference-highlight');
  const timer = setTimeout(() => element.classList.remove('reference-highlight'), 2200);
  return () => {
    clearTimeout(timer);
    element.classList.remove('reference-highlight');
  };
}
