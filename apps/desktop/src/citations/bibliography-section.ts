/** Locate the generated bibliography block inside a note body so re-inserting replaces it in place. */

export const BIBLIOGRAPHY_HEADINGS = ['参考文献', 'References'] as const;
export const MISSING_TARGET_PREFIXES = ['缺少引用目标：', 'Missing targets: '] as const;

export type DocNodeLike = {
  type: { name: string };
  textContent: string;
  nodeSize: number;
  attrs?: Record<string, unknown>;
};

export type DocLike = {
  forEach: (callback: (node: DocNodeLike, offset: number) => void) => void;
};

export type TopLevelNode = { node: DocNodeLike; offset: number };

export type BibliographySection = {
  from: number;
  to: number;
  styleId: string | null;
  fingerprint: string | null;
  /** null when an older bibliography has no fingerprint to compare. */
  modified: boolean | null;
};

type FingerprintPart = { type: string; text: string };
type JsonNodeLike = {
  type?: string;
  text?: string;
  content?: readonly JsonNodeLike[];
};

export function topLevelNodes(doc: DocLike): TopLevelNode[] {
  const nodes: TopLevelNode[] = [];
  doc.forEach((node, offset) => {
    nodes.push({ node, offset });
  });
  return nodes;
}

/** Stable, compact fingerprint used to detect manual edits to a generated section. */
export function bibliographyFingerprint(parts: readonly FingerprintPart[]): string {
  const source = parts.map((part) => `${part.type}\u001f${part.text}`).join('\u001e');
  let hash = 0xcbf29ce484222325n;
  for (const character of source) {
    hash ^= BigInt(character.codePointAt(0) ?? 0);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return hash.toString(16).padStart(16, '0');
}

function jsonNodeText(node: JsonNodeLike): string {
  if (typeof node.text === 'string') return node.text;
  return (node.content ?? []).map(jsonNodeText).join('');
}

/** Fingerprint the JSON passed to `insertContent` before it is placed in the document. */
export function bibliographyFingerprintFromJson(nodes: readonly JsonNodeLike[]): string {
  return bibliographyFingerprint(
    nodes.map((node) => ({ type: node.type ?? '', text: jsonNodeText(node) })),
  );
}

export function bibliographyFingerprintFromNodes(nodes: readonly DocNodeLike[]): string {
  return bibliographyFingerprint(
    nodes.map((node) => ({ type: node.type.name, text: node.textContent })),
  );
}

/**
 * Range of the last bibliography section: its heading, the entry list and the
 * missing-target note, but never body text the writer added afterwards.
 * Returns null when the note has no bibliography heading yet.
 */
export function bibliographySection(
  doc: DocLike,
  headings: readonly string[] = BIBLIOGRAPHY_HEADINGS,
  missingPrefixes: readonly string[] = MISSING_TARGET_PREFIXES,
): BibliographySection | null {
  const nodes = topLevelNodes(doc);
  let index = -1;
  for (let cursor = nodes.length - 1; cursor >= 0; cursor -= 1) {
    const candidate = nodes[cursor];
    if (
      candidate.node.type.name === 'heading' &&
      headings.includes(candidate.node.textContent.trim())
    ) {
      index = cursor;
      break;
    }
  }
  if (index < 0) return null;
  const start = nodes[index];
  let end = start.offset + start.node.nodeSize;
  let endIndex = index;
  const list = nodes[index + 1];
  if (list && list.node.type.name === 'bulletList') {
    end = list.offset + list.node.nodeSize;
    endIndex = index + 1;
    const note = nodes[index + 2];
    if (
      note &&
      note.node.type.name === 'paragraph' &&
      missingPrefixes.some((prefix) => note.node.textContent.startsWith(prefix))
    ) {
      end = note.offset + note.node.nodeSize;
      endIndex = index + 2;
    }
  }
  const storedFingerprint = start.node.attrs?.bibliography_fingerprint;
  const fingerprint = typeof storedFingerprint === 'string' ? storedFingerprint : null;
  const storedStyleId = start.node.attrs?.bibliography_style_id;
  const styleId = typeof storedStyleId === 'string' ? storedStyleId : null;
  const currentFingerprint = bibliographyFingerprintFromNodes(
    nodes.slice(index, endIndex + 1).map(({ node }) => node),
  );
  return {
    from: start.offset,
    to: end,
    styleId,
    fingerprint,
    modified: fingerprint ? fingerprint !== currentFingerprint : null,
  };
}

/** The replaceable range of the final generated bibliography section. */
export function bibliographyRange(
  doc: DocLike,
  headings: readonly string[] = BIBLIOGRAPHY_HEADINGS,
  missingPrefixes: readonly string[] = MISSING_TARGET_PREFIXES,
): { from: number; to: number } | null {
  const section = bibliographySection(doc, headings, missingPrefixes);
  return section ? { from: section.from, to: section.to } : null;
}
