import {
  SourceCollectionPicker,
  type SourceCollectionChoice,
} from '../sources/SourceCollectionPicker';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowLeft, ArrowUpRight, Check, Clock3, RefreshCw, X } from 'lucide-react';
import type {
  ResolveLink,
  ReviewBucket,
  ReviewCounts,
  ReviewEvent,
  ReviewItem,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { ListPagination } from '../layout/Pagination.tsx';
import { usePageNavigationPaging } from '../layout/usePageNavigationPaging';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { PageFrame } from '../layout/PageFrame';
import { PageState } from '../layout/PageState';
import { resolveListDisplayState } from '../layout/page-state';
import { snoozeTimestamp, type ReviewCommand } from './review-intents';
import { reviewTabCounts } from './review-tab-counts';
import { SnoozeForm } from './SnoozeForm';
import { ReviewTabs } from './ReviewTabs';
import {
  reviewSnoozeView,
  snoozeMinimum,
  toggleSnoozePanel,
  withSnoozeDate,
  type SnoozePanel,
} from './snooze-panel';
import { useReviewAction } from './useReviewAction';
import './review.css';

export type ReviewPageProps = {
  identity: WorkspaceIdentity;
  onOpen: (target: ResolveLink) => void;
};
export function ReviewPage(props: ReviewPageProps) {
  return (
    <ReviewScope
      key={`${props.identity.local_instance_id}/${props.identity.workspace_id}`}
      {...props}
    />
  );
}
function ReviewScope({ identity, onOpen }: ReviewPageProps) {
  const { t, language } = useI18n();
  const [bucket, setBucket] = usePageNavigationState<ReviewBucket>('review.bucket', 'due');
  const [collection, setCollection] = usePageNavigationState('review.collection', '');
  const [collectionChoice, setCollectionChoice] =
    usePageNavigationState<SourceCollectionChoice | null>('review.collectionChoice', null);
  const navigationKey = `review.${bucket}.${collection || 'all'}`;
  const [selected, setSelected] = usePageNavigationState<string | null>(
    `${navigationKey}.selected`,
    null,
  );
  const [detailOpen, setDetailOpen] = usePageNavigationState(`${navigationKey}.detail`, false);
  const [detailScroll, setDetailScroll] = usePageNavigationState(
    `${navigationKey}.detailScroll`,
    0,
  );
  const detailElement = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [completedRefresh, setCompletedRefresh] = useState(0);
  // The 稍后回顾 panel is bound to the entry its date was typed for, so one item's
  // date can never be edited or submitted for another one.
  const [snooze, setSnooze] = usePageNavigationState<SnoozePanel | null>(
    `${navigationKey}.snooze`,
    null,
  );
  const identityKey = `${identity.local_instance_id}/${identity.workspace_id}`;
  // One paged list per identity/bucket/collection: the pager owns page 1, so a new
  // scope starts clean and the first row of the arriving page becomes the selection.
  // The page size picker is the list's own, so the old hard-coded limits are gone.
  const paging = usePageNavigationPaging<ReviewItem>(
    `${identityKey}/${bucket}/${collection || 'all'}`,
    navigationKey,
    (cursor, size) =>
      call(
        {
          kind: 'review_list',
          payload: { bucket, collection_id: collection || null, limit: size, cursor },
        },
        'review_page',
        identity,
      ),
    true,
    !detailOpen,
  );
  const list = paging.pager;
  // Tab badges count the queue this page is filtered to. The number is a real read
  // rather than a guess, so it is taken again whenever the page re-reads, and the
  // badge stays 「—」 until a count arrives instead of claiming 0.
  const [counts, setCounts] = useState<ReviewCounts | null>(null);
  const [countNote, setCountNote] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    setCounts(null);
    setCountNote(t('正在统计回顾数量…', 'Counting review items…'));
    // One aggregate read with the kernel's own `as_of` boundary. The badge used
    // to page the whole queue (bounded at 10,000 rows, then 「—」).
    void call(
      { kind: 'review_counts', payload: { collection_id: collection || null } },
      'review_counts',
      identity,
    )
      .then((next) => {
        if (!active) return;
        setCounts(next);
        setCountNote(null);
      })
      .catch((failure) => {
        if (!active) return;
        setCounts(null);
        setCountNote(message(failure));
      });
    return () => {
      active = false;
    };
  }, [identityKey, collection, reload]);
  const page = list.page;
  const busy = list.loading;
  const displayState = resolveListDisplayState({
    hasPage: Boolean(page),
    itemCount: page?.items.length ?? 0,
    loading: list.loading,
    error: list.error,
  });
  const listNotice = displayState === 'content-with-error' ? list.error : null;
  const actions = useReviewAction(identity);
  const alive = useRef(true);
  const requestSequence = useRef(0);
  const refreshButton = useRef<HTMLButtonElement>(null);
  const snoozeButton = useRef<HTMLButtonElement>(null);
  const restoreRefreshFocus = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      ++requestSequence.current;
    };
  }, []);
  useEffect(() => {
    if (!collection) {
      setCollectionChoice(null);
      return;
    }
    let live = true;
    void call(
      { kind: 'collections_get', payload: { collection_id: collection } },
      'collection',
      identity,
    )
      .then((item) => {
        if (live) setCollectionChoice({ id: item.id, name: item.name });
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [collection, identityKey, reload, setCollectionChoice]);
  const previousScope = useRef(navigationKey);
  useEffect(() => {
    if (previousScope.current !== navigationKey) {
      previousScope.current = navigationKey;
      setSnooze(null);
      setError(null);
    }
  }, [navigationKey, setSnooze]);
  useEffect(() => {
    if (!paging.ready) return;
    const items = list.page?.items;
    if (!items) return;
    setSelected((old) =>
      items.some((item) => item.entry.id === old) ? old : (items[0]?.entry.id ?? null),
    );
  }, [paging.ready, list.page, setSelected]);
  const previousSelection = useRef(selected);
  useEffect(() => {
    if (previousSelection.current !== selected) {
      previousSelection.current = selected;
      setSnooze(null);
    }
  }, [selected, setSnooze]);
  useLayoutEffect(() => {
    if (detailOpen && paging.ready && detailElement.current)
      detailElement.current.scrollTop = detailScroll;
  }, [detailOpen, paging.ready, selected]);
  // A refused action bumps `reload`, and so does the refresh button: page 1 is
  // re-read from current data, which is what the append list did. The refresh
  // button restores its focus once that read lands.
  const listReload = useRef(reload);
  useEffect(() => {
    if (listReload.current === reload) return;
    listReload.current = reload;
    const sequence = ++requestSequence.current;
    const restoreFocus = restoreRefreshFocus.current;
    restoreRefreshFocus.current = false;
    setSnooze(null);
    setError(null);
    void list.refresh().then(() => {
      if (!alive.current || sequence !== requestSequence.current) return;
      if (restoreFocus) setCompletedRefresh(sequence);
    });
  }, [reload, list]);
  // The refresh button owns focus while its read lands: the row that had focus may
  // disappear with the re-read page, so the button takes focus back once the list
  // is current again. Any other focus choice made in the meantime wins.
  useLayoutEffect(() => {
    const button = refreshButton.current;
    if (
      !completedRefresh ||
      completedRefresh !== requestSequence.current ||
      !alive.current ||
      !button?.isConnected ||
      button.disabled ||
      button.closest('[inert]') ||
      (document.activeElement !== document.body && document.activeElement !== button)
    )
      return;
    button.focus({ preventScroll: true });
  }, [completedRefresh]);
  async function commit(command?: ReviewCommand) {
    const result = await actions.submit(command);
    if (!alive.current) return;
    if (result) setSnooze(null);
    // A refused action is usually stale page data: a row version another change
    // moved on, a bucket this entry left, or an entry removed elsewhere. Reload so
    // the next attempt starts from current data instead of repeating the conflict.
    setReload((value) => value + 1);
  }
  const current = page?.items.find((item) => item.entry.id === selected);
  const entryId = current?.entry.id ?? null;
  const panel = reviewSnoozeView(snooze, entryId);
  function mutate(kind: 'review_complete' | 'review_remove' | 'review_snooze') {
    if (!current) return;
    const request = { entry_id: current.entry.id, base_row_version: current.entry.row_version };
    try {
      const command: ReviewCommand =
        kind === 'review_snooze'
          ? {
              kind,
              payload: {
                command_id: crypto.randomUUID(),
                // The field only submits while the panel is open for this entry, and an
                // empty value stays a refused request instead of an invented date.
                request: { ...request, due_at: snoozeTimestamp(panel.date) },
              },
            }
          : { kind, payload: { command_id: crypto.randomUUID(), request } };
      void commit(command);
    } catch (error) {
      setError(message(error));
    }
  }
  return (
    <PageFrame
      className="review-page"
      title={t('回顾', 'Review')}
      description={t('重新遇见值得留下的想法。', 'Return to ideas worth keeping.')}
      actions={
        <button
          ref={refreshButton}
          disabled={busy}
          onClick={(event) => {
            restoreRefreshFocus.current = document.activeElement === event.currentTarget;
            setReload((value) => value + 1);
          }}
          aria-label={t('刷新回顾', 'Refresh reviews')}
        >
          <RefreshCw size={15} aria-hidden="true" />
        </button>
      }
      toolbar={
        <div className="review-toolbar">
          {detailOpen ? (
            <button className="text-button" onClick={() => setDetailOpen(false)}>
              <ArrowLeft size={15} />
              {t('返回回顾列表', 'Back to review list')}
            </button>
          ) : (
            <>
              <ReviewTabs
                bucket={bucket}
                counts={reviewTabCounts(counts)}
                countNote={countNote}
                onSelect={setBucket}
              />
              <SourceCollectionPicker
                identity={identity}
                label={t('主题', 'Collection')}
                emptyLabel={t('全部主题', 'All collections')}
                disabled={busy}
                value={
                  collection
                    ? collectionChoice?.id === collection
                      ? collectionChoice
                      : { id: collection, name: t('当前主题', 'Current collection') }
                    : null
                }
                onChange={(choice) => {
                  setCollectionChoice(choice);
                  setCollection(choice?.id ?? '');
                }}
                generation={reload}
              />
            </>
          )}
        </div>
      }
      footer={detailOpen ? undefined : <ListPagination list={list} />}
    >
      {(error || actions.error || listNotice) && (
        <p role="alert" className="review-notice">
          {error || actions.error || listNotice}
        </p>
      )}
      {actions.pending && (
        <div role="status" className="review-notice">
          {t(
            '上次操作的结果尚待确认，确认后再继续。',
            'The previous operation needs confirmation before continuing.',
          )}{' '}
          <button disabled={actions.busy} onClick={() => void commit()}>
            {actions.busy ? t('正在确认…', 'Confirming…') : t('确认操作结果', 'Confirm result')}
          </button>
        </div>
      )}
      <section
        className="review-content"
        aria-busy={busy}
        aria-label={t('回顾内容', 'Review items')}
      >
        {/* A new scope clears the page before the pager starts reading, so "no page
            and no failure yet" is a first page on its way. */}
        {displayState === 'error' ? (
          <PageState
            className="review-empty"
            kind="error"
            title={t('无法读取回顾', 'Could not load reviews')}
            detail={list.error}
            retryLabel={t('重新读取回顾', 'Reload reviews')}
            onRetry={() => void list.refresh()}
          />
        ) : displayState === 'loading' ? (
          <PageState
            className="review-empty"
            kind="loading"
            title={t('正在读取回顾…', 'Loading reviews…')}
          />
        ) : displayState === 'empty' ? (
          <div className="review-empty">
            <Check size={32} strokeWidth={1.5} aria-hidden="true" />
            <p>
              {bucket === 'due'
                ? t('当前没有待回顾的内容。', 'Nothing is due right now.')
                : t('这里还没有内容。', 'There are no items here yet.')}
            </p>
            <p className="review-muted">
              {t(
                '在笔记、资料或摘录的操作菜单里选择「加入回顾」，就会出现在这里。',
                'Use "Add to review" in the menu of a note, source, or highlight to queue it here.',
              )}
            </p>
          </div>
        ) : (
          page && (
            <div className="review-workspace">
              <div
                className="review-queue-scroll"
                ref={paging.element}
                onScroll={paging.rememberScroll}
                hidden={detailOpen}
              >
                <nav className="review-list" aria-label={t('回顾列表', 'Review list')}>
                  {page.items.map((item) => (
                    <button
                      key={item.entry.id}
                      aria-current={selected === item.entry.id ? 'true' : undefined}
                      onClick={() => {
                        setSelected(item.entry.id);
                        setDetailOpen(true);
                      }}
                    >
                      <strong>{item.resolved.title || t('未命名内容', 'Untitled')}</strong>
                      <span>
                        {item.entry.kind === 'note'
                          ? t('笔记', 'Note')
                          : item.entry.kind === 'annotation'
                            ? t('摘录', 'Annotation')
                            : t('资料', 'Source')}
                        {' · '}
                        {displayDate(
                          bucket === 'completed' ? item.entry.updated_at : item.entry.due_at,
                          language,
                        )}
                      </span>
                    </button>
                  ))}
                </nav>
              </div>
              {detailOpen && current && (
                <div
                  className="review-detail-scroll"
                  ref={detailElement}
                  onScroll={() => {
                    if (paging.ready && detailElement.current)
                      setDetailScroll(detailElement.current.scrollTop);
                  }}
                >
                  <article className="review-detail">
                    <span className="review-muted">
                      {current.entry.kind === 'note'
                        ? t('笔记', 'Note')
                        : current.entry.kind === 'annotation'
                          ? t('摘录', 'Annotation')
                          : t('资料', 'Source')}
                    </span>
                    <h2>{current.resolved.title || t('未命名内容', 'Untitled')}</h2>
                    {current.resolved.state === 'available' ? (
                      <>
                        <p className="review-excerpt">
                          {current.resolved.block_text ||
                            t('打开内容，继续阅读和思考。', 'Open this item to read and reflect.')}
                        </p>
                        <button onClick={() => onOpen(current.entry.target)}>
                          {t('查看内容', 'Open content')}{' '}
                          <ArrowUpRight size={14} aria-hidden="true" />
                        </button>
                        {current.entry.kind !== 'annotation' &&
                          current.resolved.current_revision_id !==
                            current.resolved.resolved_revision_id && (
                            <p className="review-muted">
                              {t(
                                '内容已有更新，这条回顾保留加入时的版本。',
                                'This item has changed. This review preserves the version originally added.',
                              )}{' '}
                              <button
                                onClick={() =>
                                  onOpen({
                                    target_id: current.entry.target.target_id,
                                    target_revision_id: null,
                                    target_block_id: null,
                                  })
                                }
                              >
                                {t('查看当前内容', 'Open current version')}
                              </button>
                            </p>
                          )}
                      </>
                    ) : (
                      <p className="review-notice">
                        {current.resolved.state === 'target_trashed'
                          ? t(
                              '内容已在回收站，可以移出回顾。',
                              'This item is in the trash. You can remove it from reviews.',
                            )
                          : t(
                              '原内容或定位已不可用，可以移出回顾。',
                              'The original content or location is unavailable. You can remove it from reviews.',
                            )}
                      </p>
                    )}
                    <div className="review-actions">
                      {current.entry.state === 'removed' && (
                        <button
                          disabled={actions.locked || current.resolved.state !== 'available'}
                          onClick={() =>
                            void commit({
                              kind: 'review_add',
                              payload: {
                                command_id: crypto.randomUUID(),
                                request: {
                                  target: current.entry.target,
                                  base_row_version: current.entry.row_version,
                                },
                              },
                            })
                          }
                        >
                          {t('重新加入回顾', 'Add to review again')}
                        </button>
                      )}
                      {current.entry.state === 'pending' && (
                        <>
                          <button
                            className="primary"
                            disabled={actions.locked || current.resolved.state !== 'available'}
                            onClick={() => mutate('review_complete')}
                          >
                            <Check size={15} aria-hidden="true" />
                            {t('完成回顾', 'Mark reviewed')}
                          </button>
                          <button
                            ref={snoozeButton}
                            disabled={actions.locked}
                            aria-expanded={panel.open}
                            onClick={() => {
                              if (entryId) setSnooze((old) => toggleSnoozePanel(old, entryId));
                            }}
                          >
                            <Clock3 size={15} aria-hidden="true" />
                            {t('稍后回顾', 'Review later')}
                          </button>
                        </>
                      )}
                      {current.entry.state !== 'removed' && (
                        <button disabled={actions.locked} onClick={() => mutate('review_remove')}>
                          <X size={15} aria-hidden="true" />
                          {t('移出回顾', 'Remove from review')}
                        </button>
                      )}
                    </div>
                    {panel.open && entryId && (
                      <SnoozeForm
                        date={panel.date}
                        min={snoozeMinimum(new Date())}
                        disabled={actions.locked}
                        onDate={(value) => setSnooze((old) => withSnoozeDate(old, entryId, value))}
                        onSubmit={() => mutate('review_snooze')}
                        onCancel={() => {
                          setSnooze(null);
                          snoozeButton.current?.focus({ preventScroll: true });
                        }}
                      />
                    )}
                    <ReviewHistory
                      key={`${current.entry.id}/${current.entry.row_version}`}
                      identity={identity}
                      entryId={current.entry.id}
                    />
                  </article>
                </div>
              )}
            </div>
          )
        )}
      </section>
    </PageFrame>
  );
}

function ReviewHistory({ identity, entryId }: { identity: WorkspaceIdentity; entryId: string }) {
  const { t, language } = useI18n();
  const [expanded, setExpanded] = usePageNavigationState(
    `review.history.${entryId}.expanded`,
    false,
  );
  const summary = useRef<HTMLElement>(null);
  // A collapsed section reads nothing, so the paged list is keyed on the open entry.
  const historyPaging = usePageNavigationPaging<ReviewEvent>(
    expanded ? `${identity.local_instance_id}/${identity.workspace_id}/${entryId}` : null,
    `review.history.${entryId}`,
    async (cursor, size) => {
      const result = await call(
        { kind: 'review_history', payload: { entry_id: entryId, limit: size, cursor } },
        'review_event_page',
        identity,
      );
      if (result.items.some((item) => item.entry_id !== entryId))
        throw new Error(t('回顾历史响应不匹配。', 'Review history response does not match.'));
      return result;
    },
  );
  const list = historyPaging.pager;
  const page = list.page;
  return (
    <details
      open={expanded}
      className="review-history"
      onToggle={(event) => setExpanded(event.currentTarget.open)}
    >
      <summary ref={summary}>{t('操作历史', 'Review history')}</summary>
      {expanded && (
        <div aria-busy={list.loading}>
          {list.error && (
            <p role="alert" className="review-notice">
              {list.error}{' '}
              <button
                disabled={list.loading}
                onClick={() => {
                  summary.current?.focus();
                  void list.loadFirst();
                }}
              >
                {t('重新读取', 'Reload')}
              </button>
            </p>
          )}
          {list.loading && <p role="status">{t('正在读取历史…', 'Loading history…')}</p>}
          {page?.items.length === 0 && (
            <p className="review-muted">
              {t('还没有操作记录。', 'No review actions recorded yet.')}
            </p>
          )}
          {!!page?.items.length && (
            <ol>
              {page.items.map((item) => (
                <li key={item.seq}>
                  <span>
                    {item.action === 'added'
                      ? t('加入回顾', 'Added to review')
                      : item.action === 'completed'
                        ? t('完成回顾', 'Marked reviewed')
                        : item.action === 'removed'
                          ? t('移出回顾', 'Removed from review')
                          : t('稍后回顾', 'Snoozed')}
                  </span>
                  <time dateTime={item.occurred_at}>{displayDate(item.occurred_at, language)}</time>
                  {item.action === 'snoozed' && (
                    <small>
                      {t('下次回顾：', 'Review after: ')}
                      {displayDate(item.due_at, language)}
                    </small>
                  )}
                </li>
              ))}
            </ol>
          )}
          <ListPagination list={list} />
        </div>
      )}
    </details>
  );
}
