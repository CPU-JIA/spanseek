export async function readBoundedCursorPage<T>(
  cursor: string | null,
  size: number,
  limit: number,
  read: (
    cursor: string | null,
    size: number,
  ) => Promise<{ items: T[]; next_cursor: string | null }>,
) {
  if (!Number.isInteger(size) || size < 1 || size > 100 || !Number.isInteger(limit) || limit < 1)
    throw new Error('Invalid page size');
  const items: T[] = [];
  const seen = new Set([cursor]);
  do {
    const requested = Math.min(limit, size - items.length);
    const page = await read(cursor, requested);
    if (page.items.length > requested || (page.next_cursor !== null && seen.has(page.next_cursor)))
      throw new Error('Invalid page continuation');
    items.push(...page.items);
    cursor = page.next_cursor;
    seen.add(cursor);
  } while (cursor !== null && items.length < size);
  return { items, next_cursor: cursor };
}
