import { useEffect, useRef, useState } from 'react';
import { Check, Clock3 } from 'lucide-react';
import type { ResolveLink, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useReviewAction } from './useReviewAction';
import {
  invalidateReviewIndex,
  reviewAddBaseVersion,
  reviewEntriesFor,
  selectReviewStatus,
  type ReviewStatus,
} from './review-status';
import { reviewAddComponentKey, reviewAddScope, reviewTargetKey } from './review-target';
import './add-to-review.css';

type Props = {
  identity: WorkspaceIdentity;
  target: ResolveLink;
  disabled?: boolean;
  /** Save current edits, then provide the exact committed revision to freeze. */
  prepareTarget?: () => Promise<ResolveLink>;
};
export function AddToReview(props: Props) {
  return <AddScope key={reviewAddComponentKey(props.identity, props.target)} {...props} />;
}
function AddScope({ identity, target, disabled, prepareTarget }: Props) {
  const { t } = useI18n();
  const actions = useReviewAction(identity, reviewAddScope(target));
  const [preparing, setPreparing] = useState(false);
  const [status, setStatus] = useState<ReviewStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [statusRead, setStatusRead] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const locked = useRef(false);
  // `target` is a fresh object on every render, so the read keys on this string.
  const targetKey = reviewTargetKey(target);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    let active = true;
    setStatus(null);
    setStatusError(null);
    reviewEntriesFor(identity, (bucket, cursor) =>
      call(
        { kind: 'review_list', payload: { bucket, collection_id: null, limit: 100, cursor } },
        'review_page',
        identity,
      ),
    )
      .then((entries) => {
        if (active) setStatus(selectReviewStatus(entries, target));
      })
      .catch((readError) => {
        if (active) setStatusError(message(readError));
      });
    return () => {
      active = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, targetKey, statusRead]);
  const state = status?.state ?? null;
  async function add() {
    if (locked.current || disabled || (actions.locked && !actions.pending)) return;
    if (statusError) {
      // The current state is unknown: re-read it instead of guessing what a click means.
      setStatusRead((value) => value + 1);
      return;
    }
    if (!status) return;
    locked.current = true;
    setPreparing(true);
    setError(null);
    try {
      const result = actions.pending
        ? await actions.submit()
        : await (async () => {
            const frozen = prepareTarget ? await prepareTarget() : structuredClone(target);
            if (!alive.current) return null;
            return actions.submit({
              kind: 'review_add',
              payload: {
                command_id: crypto.randomUUID(),
                request: {
                  target: frozen,
                  base_row_version: reviewAddBaseVersion(status, frozen),
                },
              },
            });
          })();
      if (!alive.current) return;
      if (result) {
        setStatus({ state: 'pending', entry: result.entry });
      } else {
        // A refused add can mean this button's version or bucket moved on: re-read
        // instead of repeating a request that is known to fail.
        setStatusRead((value) => value + 1);
      }
      // The shared queue read is stale for every other control either way.
      invalidateReviewIndex(identity);
    } catch (error) {
      if (alive.current) setError(message(error));
    } finally {
      locked.current = false;
      if (alive.current) setPreparing(false);
    }
  }
  const label =
    preparing || actions.busy
      ? t('正在确认…', 'Confirming…')
      : actions.pending
        ? t('确认加入', 'Confirm add')
        : statusError
          ? t('回顾状态未知，点击重试', 'Review status unknown (retry)')
          : !status
            ? t('正在读取回顾状态…', 'Checking review status…')
            : state === 'pending'
              ? t('已加入待回顾', 'Queued for review')
              : state === 'completed'
                ? t('已回顾（可再次加入）', 'Reviewed (add again)')
                : state === 'removed'
                  ? t('已移出（可重新加入）', 'Removed (add again)')
                  : t('加入回顾', 'Add to review');
  return (
    <span className="add-to-review">
      <button
        type="button"
        disabled={
          disabled ||
          preparing ||
          actions.busy ||
          (actions.locked && !actions.pending) ||
          (!status && !statusError)
        }
        onClick={() => void add()}
        title={
          actions.pending
            ? t('确认之前的加入请求', 'Confirm the previous add request')
            : (statusError ?? undefined)
        }
      >
        {state === 'pending' && !actions.pending ? (
          <Check size={14} aria-hidden="true" />
        ) : (
          <Clock3 size={14} aria-hidden="true" />
        )}
        {label}
      </button>
      {(error || actions.error || statusError) && (
        <span role="alert" className="add-to-review-error">
          {error || actions.error || statusError}
        </span>
      )}
    </span>
  );
}
