import type { Editor, EditorEvents } from '@tiptap/core';
import type { NoteDocument } from '@pkms/contracts';
import type { DraftController } from './draft-controller';
import { mirrorTransactions, MIRROR } from './mirror-transactions.ts';

interface EditorHandlers {
  fail: (error: unknown) => void;
  /** Record the document object this pane's editor now holds. */
  applied: (document: NoteDocument) => void;
}

const groups = new WeakMap<DraftController, Map<Editor, EditorHandlers>>();

/** Dispatch synchronously; no stale full-document effect can replace another pane's edit. */
export function joinEditors(controller: DraftController, editor: Editor, handlers: EditorHandlers) {
  const group = groups.get(controller) ?? new Map<Editor, EditorHandlers>();
  groups.set(controller, group);
  group.set(editor, handlers);
  const sync = ({ transaction, appendedTransactions }: EditorEvents['transaction']) => {
    if (!group.has(editor) || !editor.isEditable) return;
    if (transaction.getMeta(MIRROR)) return;
    const transactions = [transaction, ...appendedTransactions];
    if (!transactions.some((item) => item.docChanged)) return;
    for (const [peer, peerHandlers] of group) {
      if (peer === editor || peer.isDestroyed) continue;
      try {
        peer.view.dispatch(mirrorTransactions(peer.state, transactions));
      } catch (error) {
        // Keep the source update/save alive and prevent a stale peer from submitting.
        peer.setEditable(false, false);
        group.delete(peer);
        peerHandlers.fail(error);
      }
    }
  };
  editor.on('transaction', sync);
  return () => {
    editor.off('transaction', sync);
    group.delete(editor);
    if (!group.size) groups.delete(controller);
  };
}

/**
 * Records that every joined pane now holds the document object another pane just
 * produced. Mirroring suppresses the peers' own `onUpdate`, so without this they
 * would fall back to comparing both full documents on every edit.
 */
export function shareEditedDocument(
  controller: DraftController,
  editor: Editor,
  document: NoteDocument,
): void {
  const group = groups.get(controller);
  if (!group) return;
  for (const [peer, handlers] of group)
    if (peer !== editor && !peer.isDestroyed) handlers.applied(document);
}
