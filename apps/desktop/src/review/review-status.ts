import type { ReviewEntry, ReviewPage, ResolveLink, WorkspaceIdentity } from '@pkms/contracts';

/** Every bucket that can hold an entry: a pending entry is due or later. */
export const REVIEW_STATUS_BUCKETS = ['due', 'later', 'completed', 'removed'] as const;
export type ReviewStatusBucket = (typeof REVIEW_STATUS_BUCKETS)[number];

export type ReviewPageReader = (
  bucket: ReviewStatusBucket,
  cursor: string | null,
) => Promise<ReviewPage>;

export type ReviewStatus = {
  /** `none` means the object is not in the queue at all. */
  state: 'none' | ReviewEntry['state'];
  /**
   * The entry an add would reuse, so its row_version is the version the command
   * must carry. It is absent when only another revision of the same object is
   * queued: that add creates a new entry and must omit the version.
   */
  entry: ReviewEntry | null;
};

/** Object identity of a target: the block belongs to it, the revision does not. */
export function reviewTargetIdentity(
  target: Pick<ResolveLink, 'target_id' | 'target_block_id'>,
): string {
  return JSON.stringify([target.target_id, target.target_block_id]);
}

/**
 * Read the whole queue once. `review_list` filters by bucket and collection, not
 * by object, so the existing command that answers "which state is this object in"
 * is the queue itself: page every bucket and match locally.
 */
export async function readReviewEntries(read: ReviewPageReader): Promise<ReviewEntry[]> {
  const entries: ReviewEntry[] = [];
  for (const bucket of REVIEW_STATUS_BUCKETS) {
    let cursor: string | null = null;
    const cursors = new Set<string>();
    do {
      const page = await read(bucket, cursor);
      entries.push(...page.items.map((item) => item.entry));
      cursor = page.next_cursor;
      if (entries.length > 10000 || (cursor !== null && cursors.has(cursor)))
        throw new Error('回顾队列分页异常 / Invalid review pagination');
      if (cursor) cursors.add(cursor);
    } while (cursor);
  }
  return entries;
}

const statePriority = { pending: 0, completed: 1, removed: 2 } as const;

/**
 * The state a manual "add to review" control must show. The revision-exact entry
 * wins because it is the one the next add reuses; without one the object still
 * reports its queue state, but no version is safe to send.
 */
export function selectReviewStatus(
  entries: readonly ReviewEntry[],
  target: ResolveLink,
): ReviewStatus {
  const identity = reviewTargetIdentity(target);
  const matches = entries.filter((entry) => reviewTargetIdentity(entry.target) === identity);
  if (!matches.length) return { state: 'none', entry: null };
  const exact = matches.filter(
    (entry) => entry.target.target_revision_id === target.target_revision_id,
  );
  const pool = exact.length ? exact : matches;
  const best = pool.reduce((left, right) =>
    statePriority[left.state] < statePriority[right.state] ||
    (statePriority[left.state] === statePriority[right.state] && left.updated_at > right.updated_at)
      ? left
      : right,
  );
  return { state: best.state, entry: exact.length ? best : null };
}

/** The version to send with an add, or null when the add creates a new entry. */
export function reviewAddBaseVersion(
  status: ReviewStatus | null,
  target: ResolveLink,
): number | null {
  return status?.entry && status.entry.target.target_revision_id === target.target_revision_id
    ? status.entry.row_version
    : null;
}

/**
 * One queue read per workspace: duplicate panes and every mounted button share the
 * same scan instead of walking the queue once per object.
 */
const queues = new Map<string, Promise<ReviewEntry[]>>();

function workspaceKey(identity: WorkspaceIdentity): string {
  return JSON.stringify([identity.local_instance_id, identity.workspace_id]);
}

export function reviewEntriesFor(
  identity: WorkspaceIdentity,
  read: ReviewPageReader,
): Promise<ReviewEntry[]> {
  const key = workspaceKey(identity);
  const cached = queues.get(key);
  if (cached) return cached;
  const pending = readReviewEntries(read);
  queues.set(key, pending);
  // A failed scan is not an answer: the next mount must be able to try again.
  pending.catch(() => {
    if (queues.get(key) === pending) queues.delete(key);
  });
  return pending;
}

/** Drop the shared queue read after a review mutation or a refused action. */
export function invalidateReviewIndex(identity: WorkspaceIdentity) {
  queues.delete(workspaceKey(identity));
}
