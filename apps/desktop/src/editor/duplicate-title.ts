/**
 * Note titles are validated by the core as 1-300 characters after trimming.
 */
export const NOTE_TITLE_LIMIT = 300;

/**
 * Builds the title of a copied note. The suffix is always kept so the copy stays
 * recognisable, and the source title is truncated to stay inside the core limit.
 */
export function duplicateTitle(title: string, suffix: string): string {
  const tail = suffix.trim();
  const head = title.trim();
  if (!tail) return Array.from(head).slice(0, NOTE_TITLE_LIMIT).join('');
  const room = Math.max(NOTE_TITLE_LIMIT - (Array.from(tail).length + 1), 0);
  const kept = Array.from(head).slice(0, room).join('').trimEnd();
  return (kept ? kept + ' ' + tail : tail).trim();
}
