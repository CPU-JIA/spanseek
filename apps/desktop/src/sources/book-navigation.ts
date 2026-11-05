import type { SourceTocItem } from '@pkms/contracts';

/** The last contents entry at or before the reader's current block. */
export function activeBookChapterIndex(toc: SourceTocItem[], position: number): number {
  let active = -1;
  for (const [index, item] of toc.entries()) {
    if (item.block_position !== null && item.block_position <= position) active = index;
  }
  return active;
}

/** Keyboard- and button-friendly bounded chapter movement. Null means no move. */
export function adjacentBookChapterIndex(
  toc: SourceTocItem[],
  current: number,
  direction: -1 | 1,
): number | null {
  if (!toc.length) return null;
  let next = current < 0 ? (direction > 0 ? 0 : -1) : current + direction;
  while (next >= 0 && next < toc.length) {
    if (toc[next].block_position !== null || toc[next].block_id !== null) return next;
    next += direction;
  }
  return null;
}

/** A small lead-in keeps the chapter heading visible after a page fetch. */
export function bookChapterTarget(
  toc: SourceTocItem[],
  index: number,
  leadIn = 8,
): { blockId: string | null; position: number; offset: number } | null {
  const item = toc[index];
  if (!item || (item.block_position === null && item.block_id === null)) return null;
  const position = Math.max(0, item.block_position ?? 0);
  return {
    blockId: item.block_id,
    position,
    offset: Math.max(0, position - Math.max(0, leadIn)),
  };
}
/** Keys that move the reader without a wheel/pointer event. */
export function isBookScrollIntentKey(key: string): boolean {
  return (
    key === 'ArrowUp' ||
    key === 'ArrowDown' ||
    key === 'PageUp' ||
    key === 'PageDown' ||
    key === 'Home' ||
    key === 'End' ||
    key === ' '
  );
}
