/**
 * The clipboard contract between the reading view and the editor.
 *
 * The editor parses internal links, references and images out of `text/html`
 * using these element/attribute names. The reading view has to emit exactly the
 * same markers, otherwise copying from reading mode and pasting into the editor
 * degrades links to plain text and drops images. Both sides read this module, and
 * `rich-paste-markers.test.mjs` checks it against the real editor schema.
 */
export const PASTE_MARKERS = {
  wikiLink: {
    tag: 'span[data-wiki-link]',
    attributes: ['data-wiki-target', 'label'],
  },
  wikiEmbed: {
    tag: 'div[data-wiki-embed]',
    attributes: ['data-wiki-target', 'label'],
  },
  noteReference: {
    tag: 'span[data-note-reference]',
    attributes: ['target_id', 'target_block_id', 'target_revision_id'],
  },
  image: {
    tag: 'figure[data-pkms-image]',
    attributes: ['blob_hash', 'original_name', 'alt', 'title'],
  },
} as const;

/**
 * Only defined, non-empty values are emitted: an empty attribute would otherwise
 * overwrite a real one on paste. Spread the result, because JSX rejects unknown
 * attribute names.
 */
export function pasteAttributes(
  attrs: Record<string, unknown> | undefined,
  names: readonly string[],
): Record<string, string> {
  const result: Record<string, string> = {};
  for (const name of names) {
    const value = attrs?.[name];
    if (typeof value === 'string' && value !== '') result[name] = value;
  }
  return result;
}
