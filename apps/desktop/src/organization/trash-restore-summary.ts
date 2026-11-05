import type { ObjectKind, TrashItem } from '@pkms/contracts';

export type TrashSelectionSummary = {
  total: number;
  /** Non-zero kinds only, in a stable order, so the line never jumps around. */
  counts: { kind: ObjectKind; count: number }[];
  /** Selected excerpts whose source is in Trash and was not selected with them. */
  excerptsWithoutSource: number;
};

const KIND_ORDER: readonly ObjectKind[] = ['note', 'source', 'annotation', 'collection'];

/**
 * The restore batch reports one IPC round trip per item, so a large selection is
 * a long operation. Knowing what is actually in the batch (how many notes, how
 * many sources, which excerpts are still waiting for their source) is the
 * estimate a user needs before starting it.
 */
export function summarizeTrashSelection(items: readonly TrashItem[]): TrashSelectionSummary {
  const selected = new Set(items.map((item) => item.object_id));
  const counts = new Map<ObjectKind, number>();
  let excerptsWithoutSource = 0;
  for (const item of items) {
    counts.set(item.kind, (counts.get(item.kind) ?? 0) + 1);
    if (
      item.kind === 'annotation' &&
      item.source_trashed &&
      (!item.parent_source_id || !selected.has(item.parent_source_id))
    )
      ++excerptsWithoutSource;
  }
  return {
    total: items.length,
    counts: KIND_ORDER.filter((kind) => counts.has(kind)).map((kind) => ({
      kind,
      count: counts.get(kind) ?? 0,
    })),
    excerptsWithoutSource,
  };
}

export type TrashFailureInput = {
  object_id: string;
  title: string;
  code: string | null;
  error: string | null;
};
export type TrashFailureGroup = {
  key: string;
  code: string | null;
  message: string;
  count: number;
  /** A few names as evidence; the rest stay behind `remaining`. */
  titles: string[];
  remaining: number;
};

/**
 * A failed batch used to render one row per failure, so 300 failures sharing two
 * reasons produced 300 identical rows. Group by reason instead: the list stays
 * readable and the count carries the size.
 */
export function groupTrashFailures(
  items: readonly TrashFailureInput[],
  titleLimit = 3,
): TrashFailureGroup[] {
  const groups = new Map<string, TrashFailureGroup>();
  for (const item of items) {
    const message = item.error ?? item.code ?? '';
    const key = item.code ?? `message:${message}`;
    const group = groups.get(key) ?? {
      key,
      code: item.code,
      message,
      count: 0,
      titles: [],
      remaining: 0,
    };
    ++group.count;
    if (group.titles.length < Math.max(0, titleLimit)) group.titles.push(item.title);
    groups.set(key, group);
  }
  return [...groups.values()]
    .map((group) => ({ ...group, remaining: group.count - group.titles.length }))
    .sort((left, right) => right.count - left.count || left.key.localeCompare(right.key));
}
