import type { CollectionMembersPage } from '@pkms/contracts';

/** Reordering is a permutation of every member, including individually trashed ones. */
export async function reorderedMembers(
  read: (cursor: string | null) => Promise<CollectionMembersPage>,
  expectedVersion: number,
  memberId: string,
  direction: -1 | 1,
): Promise<string[]> {
  let cursor: string | null = null;
  const ids: string[] = [];
  const cursors = new Set<string>();
  do {
    const page = await read(cursor);
    if (page.collection_row_version !== expectedVersion)
      throw new Error(
        'Collection changed; reload before reordering / 集合已有更新，请重新读取后排序',
      );
    ids.push(...page.items.map((member) => member.object.id));
    cursor = page.next_cursor;
    if (ids.length > 10000 || (cursor && cursors.has(cursor)))
      throw new Error('Invalid collection pagination / 集合分页异常');
    if (cursor) cursors.add(cursor);
  } while (cursor);
  if (new Set(ids).size !== ids.length)
    throw new Error('Collection members changed; reload / 集合成员已变化，请重新读取');
  const index = ids.indexOf(memberId);
  const other = index + direction;
  if (index < 0 || other < 0 || other >= ids.length)
    throw new Error('Member position changed; reload / 成员位置已变化，请重新读取');
  [ids[index], ids[other]] = [ids[other], ids[index]];
  return ids;
}
