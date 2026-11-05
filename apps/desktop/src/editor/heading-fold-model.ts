export const HEADING_FOLD_ID = /^[A-Za-z0-9-]{1,128}$/;

export type HeadingFoldBlock = {
  type: string;
  level: number;
  id: string | null;
  from: number;
  to: number;
};

export type HeadingFoldSection = {
  headingIndex: number;
  id: string;
  level: number;
  nextIndex: number;
  foldedCount: number;
};

/**
 * Heading folding is a view projection: it never changes the document JSON.
 * Sections end at the next heading of the same or a higher level.
 */
export function headingFoldSections(blocks: readonly HeadingFoldBlock[]): HeadingFoldSection[] {
  const sections: HeadingFoldSection[] = [];
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index];
    if (block.type !== 'heading' || !block.id) continue;
    let nextIndex = blocks.length;
    for (let scan = index + 1; scan < blocks.length; scan += 1) {
      const candidate = blocks[scan];
      if (candidate.type === 'heading' && candidate.level <= block.level) {
        nextIndex = scan;
        break;
      }
    }
    sections.push({
      headingIndex: index,
      id: block.id,
      level: block.level,
      nextIndex,
      foldedCount: Math.max(0, nextIndex - index - 1),
    });
  }
  return sections;
}

/** The last top-level heading whose section contains the selection. */
export function headingFoldSectionAt(
  blocks: readonly HeadingFoldBlock[],
  position: number,
): HeadingFoldSection | null {
  if (!blocks.length) return null;
  let blockIndex = -1;
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index];
    if (position >= block.from && position <= block.to) {
      blockIndex = index;
      break;
    }
    if (block.from <= position) blockIndex = index;
  }
  if (blockIndex < 0) return null;
  const sections = headingFoldSections(blocks);
  for (let index = blockIndex; index >= 0; index -= 1) {
    if (blocks[index].type !== 'heading') continue;
    return sections.find((section) => section.headingIndex === index) ?? null;
  }
  return null;
}
export type HeadingFoldStorage = {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
};

const MAX_STORED_HEADINGS = 256;

export function headingFoldStorageKey(
  localInstanceId: string,
  workspaceId: string,
  noteId: string,
): string {
  return `pkms.heading-fold.v1/${localInstanceId}/${workspaceId}/${noteId}`;
}

export function parseHeadingFoldIds(raw: string | null): Set<string> {
  if (!raw) return new Set();
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(
      parsed
        .filter(
          (value): value is string => typeof value === 'string' && HEADING_FOLD_ID.test(value),
        )
        .slice(0, MAX_STORED_HEADINGS),
    );
  } catch {
    return new Set();
  }
}

export function storedHeadingFoldIds(
  collapsed: Iterable<string>,
  available: Iterable<string>,
): string[] {
  const allowed = new Set([...available].filter((id) => HEADING_FOLD_ID.test(id)));
  return [...new Set(collapsed)]
    .filter((id) => allowed.has(id))
    .sort()
    .slice(0, MAX_STORED_HEADINGS);
}

export function loadHeadingFoldIds(storage: HeadingFoldStorage, key: string): Set<string> {
  if (!key) return new Set();
  try {
    return parseHeadingFoldIds(storage.getItem(key));
  } catch {
    // Folding remains available for this session when optional view storage is unavailable.
    return new Set();
  }
}

export function saveHeadingFoldIds(
  storage: HeadingFoldStorage,
  key: string,
  collapsed: Iterable<string>,
  available: Iterable<string>,
): void {
  if (!key) return;
  const ids = storedHeadingFoldIds(collapsed, available);
  try {
    if (ids.length) storage.setItem(key, JSON.stringify(ids));
    else storage.removeItem(key);
  } catch {
    // Folding remains available for this session when optional view storage is unavailable.
  }
}
