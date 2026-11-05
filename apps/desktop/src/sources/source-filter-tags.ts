import type { Tag, TagPage } from '@pkms/contracts';

/** Recover every selected tag before allowing a picker to replace the saved IDs. */
export async function readSelectedSourceTags(
  ids: readonly string[],
  readPage: (cursor: string | null) => Promise<TagPage>,
  isCurrent: () => boolean,
): Promise<Tag[] | null> {
  const wanted = new Set(ids);
  const found = new Map<string, Tag>();
  let cursor: string | null = null;
  if (!wanted.size) return [];
  do {
    if (!isCurrent()) return null;
    const page = await readPage(cursor);
    if (!isCurrent()) return null;
    for (const tag of page.items) {
      if (wanted.has(tag.id)) found.set(tag.id, tag);
    }
    if (found.size === wanted.size) return ids.map((id) => found.get(id)!);
    cursor = page.next_cursor;
  } while (cursor);
  throw new Error('Some saved tags are unavailable / 部分已选标签暂不可用，请重试或清除标签筛选');
}
