import { Fragment, type Node as ProseMirrorNode } from '@tiptap/pm/model';
import { Selection, type EditorState, type Transaction } from '@tiptap/pm/state';

export type BlockLocation = {
  node: ProseMirrorNode;
  parent: ProseMirrorNode;
  index: number;
  depth: number;
  from: number;
};

/** Move the outermost document block containing the selection, not a nested row/cell. */
export function outermostBlock(state: EditorState): BlockLocation | null {
  const { $from } = state.selection;
  for (let depth = 1; depth <= $from.depth; depth += 1) {
    const node = $from.node(depth);
    if (!node.isBlock) continue;
    return {
      node,
      parent: $from.node(depth - 1),
      index: $from.index(depth - 1),
      depth,
      from: $from.before(depth),
    };
  }
  return null;
}

export function canMoveBlock(state: EditorState, direction: -1 | 1): boolean {
  const block = outermostBlock(state);
  if (!block) return false;
  return direction < 0 ? block.index > 0 : block.index + 1 < block.parent.childCount;
}

/** A transaction that swaps the selected block with its adjacent sibling. */
export function moveBlockTransaction(state: EditorState, direction: -1 | 1): Transaction | null {
  const block = outermostBlock(state);
  if (!block) return null;
  const siblingIndex = block.index + direction;
  if (siblingIndex < 0 || siblingIndex >= block.parent.childCount) return null;
  const sibling = block.parent.child(siblingIndex);
  const start = direction < 0 ? block.from - sibling.nodeSize : block.from;
  const end =
    direction < 0
      ? block.from + block.node.nodeSize
      : block.from + block.node.nodeSize + sibling.nodeSize;
  const replacement =
    direction < 0
      ? Fragment.fromArray([block.node, sibling])
      : Fragment.fromArray([sibling, block.node]);
  const transaction = state.tr.replaceWith(start, end, replacement);
  const target = Math.min(start + 1, transaction.doc.content.size);
  transaction.setSelection(Selection.near(transaction.doc.resolve(target)));
  return transaction.scrollIntoView();
}

export function selectedBlockId(state: EditorState): string | null {
  return outermostBlock(state)?.node.attrs.id || null;
}

function freshBlockCopy(node: ProseMirrorNode): ProseMirrorNode {
  const attrs: Record<string, unknown> = { ...node.attrs };
  if (node.isBlock && 'id' in attrs) attrs.id = crypto.randomUUID();
  if ('block_id' in attrs) attrs.block_id = null;
  if ('occurrence_id' in attrs && attrs.occurrence_id != null)
    attrs.occurrence_id = crypto.randomUUID();
  if (node.isText) return node.type.schema.text(node.text ?? '', node.marks);
  if (!node.content.size) return node.type.create(attrs, null, node.marks);
  const children: ProseMirrorNode[] = [];
  node.content.forEach((child) => children.push(freshBlockCopy(child)));
  return node.type.create(attrs, Fragment.fromArray(children), node.marks);
}

/** Duplicate the whole block after itself with fresh identity and no reused alias. */
export function duplicateBlockTransaction(state: EditorState): Transaction | null {
  const block = outermostBlock(state);
  if (!block) return null;
  const copy = freshBlockCopy(block.node);
  const insertAt = block.from + block.node.nodeSize;
  const transaction = state.tr.insert(insertAt, copy);
  const target = Math.min(insertAt + 1, transaction.doc.content.size);
  transaction.setSelection(Selection.near(transaction.doc.resolve(target)));
  return transaction.scrollIntoView();
}
