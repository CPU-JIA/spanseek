import type { CollectionMembersPage } from '@pkms/contracts';

/**
 * The member picker has two sources of rows: content that can still be added and
 * content that already belongs to the collection. The collection's own member
 * list is what decides the second source, and the picker only ever shows one
 * member page at a time, so asking the current page is not the same question:
 * a note that is a member on page 2 looks addable on page 1, the command adds
 * nothing (the core drops ids that are already members), and the UI reports a
 * success that changed nothing.
 *
 * This walks every member page - including individually trashed members, which
 * still count as members - so "already in this collection" is answered by the
 * whole collection.
 */
export async function collectionMemberIds(
  read: (cursor: string | null) => Promise<CollectionMembersPage>,
  expectedVersion: number,
): Promise<ReadonlySet<string>> {
  const ids = new Set<string>();
  const cursors = new Set<string>();
  let cursor: string | null = null;
  do {
    const page = await read(cursor);
    if (page.collection_row_version !== expectedVersion)
      throw new Error(
        'Collection changed; reload before adding members / 集合已有更新，请重新读取后再添加',
      );
    for (const member of page.items) {
      if (ids.has(member.object.id))
        throw new Error('Collection members changed; reload / 集合成员已变化，请重新读取');
      ids.add(member.object.id);
    }
    cursor = page.next_cursor;
    if (ids.size > 10000 || (cursor && cursors.has(cursor)))
      throw new Error('Invalid collection pagination / 集合分页异常');
    if (cursor) cursors.add(cursor);
  } while (cursor);
  return ids;
}

const isMemberList = (value: ReadonlySet<string> | readonly string[]): value is readonly string[] =>
  Array.isArray(value);
export type MemberCandidateRow<T> = { item: T; inCollection: boolean };

/**
 * One page of candidates. A row that is already a member keeps its place and is
 * marked instead of being filtered out of the page, so the page holds the page
 * size it promised and page numbers stay meaningful.
 */
export function memberCandidateRows<T extends { id: string }>(
  items: readonly T[],
  members: ReadonlySet<string> | readonly string[],
): MemberCandidateRow<T>[] {
  const known = isMemberList(members) ? new Set(members) : members;
  return items.map((item) => ({ item, inCollection: known.has(item.id) }));
}

/** Rows a user may still pick on this page. */
export function addableCount<T>(rows: readonly MemberCandidateRow<T>[]): number {
  return rows.reduce((total, row) => (row.inCollection ? total : total + 1), 0);
}
