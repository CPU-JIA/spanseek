import type { ReviewBucket, ReviewCounts } from '@pkms/contracts';

/** What each tab badge shows; `null` keeps the 「—」 state until a count arrives. */
export type ReviewTabCounts = Record<ReviewBucket, number>;

/**
 * The four tab badges, straight from the kernel's own buckets. The kernel
 * computes them in one aggregate read with the same `as_of` boundary the lists
 * filter with, so a badge can no longer disagree with its list at the edge — or
 * cost a full queue scan to produce a number.
 */
export function reviewTabCounts(counts: ReviewCounts | null): ReviewTabCounts | null {
  if (!counts) return null;
  return {
    due: counts.due,
    later: counts.later,
    completed: counts.completed,
    removed: counts.removed,
  };
}
