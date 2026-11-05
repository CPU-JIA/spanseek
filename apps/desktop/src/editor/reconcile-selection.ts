import type { Node as ProseMirrorNode } from '@tiptap/pm/model';
import { NodeSelection, Selection } from '@tiptap/pm/state';
import { findNodePosition } from './rich-format-model.ts';

/** Restore selection after a snapshot replacement without treating node boundaries as text. */
export function reconcileSelection(previous: Selection, document: ProseMirrorNode): Selection {
  const nearest = (position: number) =>
    Selection.near(document.resolve(Math.max(0, Math.min(position, document.content.size))));

  if (previous instanceof NodeSelection) {
    const id = previous.node.attrs.id;
    const occurrence = previous.node.attrs.occurrence_id;
    let position: number | undefined = previous.from;
    if (id) position = findNodePosition(document, id);
    else if (occurrence) {
      position = undefined;
      document.descendants((node, at) => {
        if (node.attrs.occurrence_id === occurrence) position = at;
      });
    }
    const node =
      position !== undefined && position < document.content.size ? document.nodeAt(position) : null;
    if (node && NodeSelection.isSelectable(node) && (id || occurrence || node.eq(previous.node)))
      return NodeSelection.create(document, position!);
    return nearest(previous.from);
  }

  try {
    // PM bookmarks retain text-selection direction, all-selection and table selections.
    return previous.getBookmark().resolve(document);
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    return nearest(previous.head);
  }
}
