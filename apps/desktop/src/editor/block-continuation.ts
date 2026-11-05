import { Extension } from '@tiptap/core';
import { Fragment, type Node as ProseMirrorNode } from '@tiptap/pm/model';
import {
  NodeSelection,
  Selection,
  TextSelection,
  type EditorState,
  type Transaction,
} from '@tiptap/pm/state';

const containers = new Set(['blockquote', 'callout']);
type Container = { node: ProseMirrorNode; position: number; depth: number };

export function selectedContainer(state: EditorState): Container | null {
  const { selection } = state;
  if (selection instanceof NodeSelection && containers.has(selection.node.type.name))
    return { node: selection.node, position: selection.from, depth: selection.$from.depth + 1 };
  const { $from, $to } = selection;
  for (let depth = $from.depth; depth > 0; depth--) {
    if (containers.has($from.node(depth).type.name) && $to.pos <= $from.after(depth))
      return { node: $from.node(depth), position: $from.before(depth), depth };
  }
  return null;
}

/** Insert one editable paragraph beside a container, keeping its existing content and identity. */
export function continueOutsideContainer(
  state: EditorState,
  dispatch?: (transaction: Transaction) => void,
  position?: number,
  direction: -1 | 1 = 1,
): boolean {
  const selected = position === undefined ? selectedContainer(state) : null;
  const node = position === undefined ? selected?.node : state.doc.nodeAt(position);
  const start = position ?? selected?.position;
  if (!node || start === undefined || !containers.has(node.type.name)) return false;
  const at = direction === 1 ? start + node.nodeSize : start;
  const $at = state.doc.resolve(at);
  const adjacent = direction === 1 ? $at.nodeAfter : $at.nodeBefore;
  if (adjacent?.type.name === 'paragraph' && adjacent.content.size === 0) {
    const caret = direction === 1 ? at + 1 : at - adjacent.nodeSize + 1;
    dispatch?.(state.tr.setSelection(TextSelection.create(state.doc, caret)).scrollIntoView());
    return true;
  }
  const paragraph = state.schema.nodes.paragraph.create({ id: crypto.randomUUID() });
  if (!$at.parent.canReplace($at.index(), $at.index(), Fragment.from(paragraph))) return false;
  if (dispatch) {
    const transaction = state.tr.insert(at, paragraph);
    dispatch(
      transaction.setSelection(TextSelection.create(transaction.doc, at + 1)).scrollIntoView(),
    );
  }
  return true;
}

/** Empty final paragraphs exit one level; list commands keep ownership of empty list items. */
export function exitEmptyContainer(
  state: EditorState,
  dispatch?: (transaction: Transaction) => void,
): boolean {
  const { selection } = state;
  if (!selection.empty || !(selection instanceof TextSelection)) return false;
  const { $from } = selection;
  const depth = $from.depth - 1;
  if (depth < 1 || $from.parent.type.name !== 'paragraph' || $from.parent.content.size)
    return false;
  const container = $from.node(depth);
  if (!containers.has(container.type.name) || $from.index(depth) !== container.childCount - 1)
    return false;
  const after = $from.after(depth);
  const $after = state.doc.resolve(after);
  if (!$after.parent.canReplaceWith($after.index(), $after.index(), state.schema.nodes.paragraph))
    return false;
  if (!dispatch) return true;
  const transaction = state.tr;
  const paragraph = $from.parent;
  if (container.childCount > 1) {
    const emptyAt = $from.before();
    transaction.delete(emptyAt, emptyAt + paragraph.nodeSize);
    const insertAt = transaction.mapping.map(after);
    transaction.insert(insertAt, paragraph);
    transaction.setSelection(TextSelection.create(transaction.doc, insertAt + 1));
  } else if (container.type.name === 'blockquote') {
    const start = $from.before(depth);
    transaction.replaceWith(start, after, paragraph);
    transaction.setSelection(TextSelection.create(transaction.doc, start + 1));
  } else {
    // An empty callout can still carry a meaningful title/type; keep it intact.
    return continueOutsideContainer(state, dispatch, $from.before(depth));
  }
  dispatch(transaction.scrollIntoView());
  return true;
}

/** Arrow navigation crosses the nearest quote boundary only at its actual text edge. */
export function moveAcrossContainer(
  state: EditorState,
  direction: -1 | 1,
  dispatch?: (transaction: Transaction) => void,
): boolean {
  const container = selectedContainer(state);
  if (!container) return false;
  const { selection } = state;
  if (!(selection instanceof NodeSelection)) {
    if (!selection.empty || !selection.$from.parent.isTextblock) return false;
    const { $from } = selection;
    if ($from.parentOffset !== (direction === 1 ? $from.parent.content.size : 0)) return false;
    for (let depth = $from.depth - 1; depth >= container.depth; depth--) {
      if ($from.index(depth) !== (direction === 1 ? $from.node(depth).childCount - 1 : 0))
        return false;
    }
  }
  const at = container.position + (direction === 1 ? container.node.nodeSize : 0);
  const $at = state.doc.resolve(at);
  const adjacent = direction === 1 ? $at.nodeAfter : $at.nodeBefore;
  if (!adjacent) return continueOutsideContainer(state, dispatch, container.position, direction);
  dispatch?.(state.tr.setSelection(Selection.near($at, direction)).scrollIntoView());
  return true;
}

export const BlockContinuation = Extension.create({
  name: 'blockContinuation',
  priority: 1100,
  addKeyboardShortcuts() {
    const run = (
      action: (state: EditorState, dispatch: (transaction: Transaction) => void) => boolean,
    ) => {
      if (this.editor.isDestroyed || !this.editor.isEditable || this.editor.view.composing)
        return false;
      return action(this.editor.state, (transaction) => this.editor.view.dispatch(transaction));
    };
    return {
      Enter: () => run(exitEmptyContainer),
      'Mod-Enter': () => run(continueOutsideContainer),
      ArrowDown: () => run((state, dispatch) => moveAcrossContainer(state, 1, dispatch)),
      ArrowUp: () => run((state, dispatch) => moveAcrossContainer(state, -1, dispatch)),
      ArrowRight: () => run((state, dispatch) => moveAcrossContainer(state, 1, dispatch)),
      ArrowLeft: () => run((state, dispatch) => moveAcrossContainer(state, -1, dispatch)),
    };
  },
});
