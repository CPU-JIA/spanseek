import type { EditorState, Transaction } from '@tiptap/pm/state';
import { Step } from '@tiptap/pm/transform';

export const MIRROR = 'pkmsMirror';

/** Apply the same edit, mapping this pane's cursor and undo history through it. */
export function mirrorTransactions(state: EditorState, transactions: Transaction[]): Transaction {
  const before = transactions[0]?.before;
  if (!before || !state.doc.eq(state.schema.nodeFromJSON(before.toJSON())))
    throw new Error('双面板正文不同步，当前内容已保留，请重新打开此页签。');
  const mirrored = state.tr;
  for (const transaction of transactions) {
    for (const step of transaction.steps) {
      // Each Tiptap instance owns a schema; don't insert nodes from another schema.
      mirrored.step(Step.fromJSON(state.schema, step.toJSON()));
    }
  }
  return mirrored
    .setMeta(MIRROR, true)
    .setMeta('preventUpdate', true)
    .setMeta('addToHistory', false);
}
