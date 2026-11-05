export const TAG_TONE_COUNT = 12;

/**
 * Derive a stable visual tone from the immutable tag id.
 *
 * Names are deliberately excluded: renaming a tag must not move it to a new
 * colour, and the same id must render identically in every surface.
 */
export function tagTone(id: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < id.length; index += 1) {
    hash ^= id.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0) % TAG_TONE_COUNT;
}

export function tagToneClass(id: string): string {
  return `tag-tone tag-tone-${tagTone(id)}`;
}
