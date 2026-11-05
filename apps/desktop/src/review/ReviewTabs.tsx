import type { ReviewBucket } from '@pkms/contracts';
import { useI18n } from '../i18n';
import type { ReviewTabCounts } from './review-tab-counts';
import { REVIEW_STATUS_BUCKETS } from './review-status';

/** The tab's copy; the badge repeats it, so both live in one place. */
export function reviewBucketLabel(
  bucket: ReviewBucket,
  choose: (zh: string, en: string) => string,
): string {
  if (bucket === 'due') return choose('待回顾', 'Due');
  if (bucket === 'later') return choose('稍后', 'Later');
  if (bucket === 'completed') return choose('已回顾', 'Completed');
  return choose('已移出', 'Removed');
}

export type ReviewTabsProps = {
  bucket: ReviewBucket;
  /** `null` until the queue read answers: a badge never shows a guessed 0. */
  counts: ReviewTabCounts | null;
  /** Why a count is still unknown (reading, or the read failed); the badge tooltip. */
  countNote: string | null;
  onSelect: (bucket: ReviewBucket) => void;
};

export function ReviewTabs({ bucket, counts, countNote, onSelect }: ReviewTabsProps) {
  const { t } = useI18n();
  return (
    <div className="review-tabs" role="group" aria-label={t('回顾状态', 'Review status')}>
      {REVIEW_STATUS_BUCKETS.map((value) => {
        const count = counts?.[value] ?? null;
        const label = reviewBucketLabel(value, t);
        return (
          <button
            key={value}
            type="button"
            aria-pressed={bucket === value}
            onClick={() => onSelect(value)}
            title={count === null ? (countNote ?? undefined) : undefined}
            aria-label={
              count === null ? t(`${label}，数量未知`, `${label}, count unknown`) : undefined
            }
          >
            {label}
            <span className="review-tab-count" data-known={count === null ? 'no' : 'yes'}>
              {count ?? '—'}
            </span>
          </button>
        );
      })}
    </div>
  );
}
