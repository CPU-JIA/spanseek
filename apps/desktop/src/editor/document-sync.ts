import type { NoteDocument } from '@pkms/contracts';

/**
 * Whether a pane has to load the record document into its editor.
 *
 * A self-produced edit hands the editor back the exact object it just produced,
 * so reference identity settles it without serializing the whole document twice.
 * Every other change (mirrored peer edit, adopted revision, restored history)
 * still compares the full documents so a real difference is never missed.
 */
export function needsDocumentSync(
  recordDocument: NoteDocument,
  applied: NoteDocument | null,
  readEditorDocument: () => NoteDocument,
): boolean {
  if (applied === recordDocument) return false;
  return JSON.stringify(readEditorDocument()) !== JSON.stringify(recordDocument);
}
