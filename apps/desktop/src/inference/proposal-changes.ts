import type { DocumentNode, NoteDocument } from '@pkms/contracts';

export type ProposalBlockChangeKind = 'added' | 'changed' | 'removed';

export type ProposalBlockChange = {
  id: string;
  kind: ProposalBlockChangeKind;
  type: string;
  preview: string;
};

export type ProposalChangeSet = {
  added: number;
  changed: number;
  removed: number;
  total: number;
  changes: ProposalBlockChange[];
  hidden: number;
};

const MAX_PREVIEW = 180;

function blockText(node: DocumentNode): string {
  const parts: string[] = [];
  if (node.text) parts.push(node.text);
  for (const child of node.content ?? []) parts.push(blockText(child));
  return parts.join(' ').replace(/\s+/g, ' ').trim();
}

function blockKey(node: DocumentNode, index: number): string {
  return node.attrs?.id ?? `index:${index}`;
}

function preview(node: DocumentNode): string {
  const text = blockText(node);
  if (text.length <= MAX_PREVIEW) return text;
  return `${text.slice(0, MAX_PREVIEW - 1)}…`;
}

function comparableBlock(node: DocumentNode): string {
  const attrs = node.attrs ? { ...node.attrs } : null;
  if (attrs) delete attrs.id;
  return JSON.stringify({ ...node, attrs: attrs && Object.keys(attrs).length ? attrs : undefined });
}

function change(
  node: DocumentNode,
  index: number,
  kind: ProposalBlockChangeKind,
): ProposalBlockChange {
  return {
    id: blockKey(node, index),
    kind,
    type: node.type,
    preview: preview(node),
  };
}

/**
 * Produce a review-oriented summary of top-level block changes. It deliberately
 * compares the same stable node ids used by the proposal preview; it does not
 * claim a text diff or silently drop nodes that have no id.
 */
export function proposalBlockChanges(
  base: NoteDocument | null,
  proposed: NoteDocument,
  limit = 8,
): ProposalChangeSet {
  const baseNodes = base?.content ?? [];
  const proposedNodes = proposed.content;
  const before = new Map<string, DocumentNode>();
  for (let index = 0; index < baseNodes.length; index += 1) {
    before.set(blockKey(baseNodes[index], index), baseNodes[index]);
  }
  const after = new Map<string, DocumentNode>();
  for (let index = 0; index < proposedNodes.length; index += 1) {
    after.set(blockKey(proposedNodes[index], index), proposedNodes[index]);
  }

  const usedBase = new Set<string>();
  for (let index = 0; index < proposedNodes.length; index += 1) {
    const key = blockKey(proposedNodes[index], index);
    if (before.has(key)) usedBase.add(key);
  }
  // A converter may legitimately create a new id for an unchanged block. Match
  // such leftovers by type and visible text before calling them add/remove.
  const contentMatched = new Set<string>();
  const contentChanged = new Set<string>();
  for (let index = 0; index < proposedNodes.length; index += 1) {
    const node = proposedNodes[index];
    const key = blockKey(node, index);
    if (before.has(key)) continue;
    const text = blockText(node);
    for (let baseIndex = 0; baseIndex < baseNodes.length; baseIndex += 1) {
      const baseNode = baseNodes[baseIndex];
      const baseKey = blockKey(baseNode, baseIndex);
      if (usedBase.has(baseKey)) continue;
      if (baseNode.type !== node.type || blockText(baseNode) !== text) continue;
      usedBase.add(baseKey);
      contentMatched.add(key);
      if (comparableBlock(baseNode) !== comparableBlock(node)) contentChanged.add(key);
      break;
    }
  }

  const changes: ProposalBlockChange[] = [];
  let added = 0;
  let changed = 0;
  let removed = 0;
  for (let index = 0; index < proposedNodes.length; index += 1) {
    const node = proposedNodes[index];
    const key = blockKey(node, index);
    if (before.has(key)) {
      if (JSON.stringify(before.get(key)) !== JSON.stringify(node)) {
        changed += 1;
        changes.push(change(node, index, 'changed'));
      }
    } else if (contentChanged.has(key)) {
      changed += 1;
      changes.push(change(node, index, 'changed'));
    } else if (!contentMatched.has(key)) {
      added += 1;
      changes.push(change(node, index, 'added'));
    }
  }
  for (let index = 0; index < baseNodes.length; index += 1) {
    const node = baseNodes[index];
    if (!usedBase.has(blockKey(node, index))) {
      removed += 1;
      changes.push(change(node, index, 'removed'));
    }
  }

  const total = changes.length;
  const safeLimit = Math.max(0, Math.floor(limit));
  return {
    added,
    changed,
    removed,
    total,
    changes: changes.slice(0, safeLimit),
    hidden: Math.max(0, total - safeLimit),
  };
}
