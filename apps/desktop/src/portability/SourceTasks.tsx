import { PageFooter } from '../layout/PageFrame';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { FileText, RefreshCw, X } from 'lucide-react';
import type { SourceJob, SourceJobEvent, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { sourceImportFailure } from '../sources/source-import-failure';
import { useSourceActions } from '../sources/useSourceActions';
import { ListPagination } from '../layout/Pagination.tsx';
import { usePageNavigationState } from '../layout/page-navigation-state';
import type { TaskDestination } from './task-navigation';
import { usePageNavigationPaging } from '../layout/usePageNavigationPaging';
import { latestSourceJob } from './source-task-state';
import { nextSourcePollDelay, sourcePollOnVisibilityChange } from './poll-schedule';

const active = (job: SourceJob) => job.status === 'queued' || job.status === 'running';

export function SourceTasks({
  identity,
  refresh,
  targetId,
  onOpenSource,
}: {
  identity: WorkspaceIdentity;
  /** Manual refresh counter of the task center; reloads the list without remounting. */
  refresh: number;
  targetId?: string;
  onOpenSource: (id: string, versionId?: string) => void;
}) {
  const { t, language } = useI18n();
  const actions = useSourceActions(identity, 'task-center');
  const [error, setError] = useState<string | null>(null);
  const [canceling, setCanceling] = useState<string | null>(null);
  const [selected, setSelected] = usePageNavigationState<string | null>(
    'tasks.sources.selected',
    null,
  );
  const [scroll] = usePageNavigationState('tasks.sources.scroll', 0);
  const [located, setLocated] = useState<SourceJob | null>(null);
  const [locateError, setLocateError] = useState<string | null>(null);
  const [locateRevision, setLocateRevision] = useState(0);
  const [, setTarget] = usePageNavigationState<TaskDestination | null>('tasks.target', null);
  const lastAcceptedJob = useRef(actions.lastJob?.jobId);
  const sectionElement = useRef<HTMLElement>(null);
  const lastTarget = useRef('');
  const alive = useRef(true);
  const currentJobs = useRef<SourceJob[]>([]);
  const jobsList = usePageNavigationPaging<SourceJob>(
    JSON.stringify([
      identity.local_instance_id,
      identity.workspace_id,
      refresh,
      actions.generation,
    ]),
    'tasks.sources',
    async (cursor, size) => {
      const page = await call(
        { kind: 'source_jobs_page', payload: { limit: size, cursor } },
        'source_jobs_page',
        identity,
      );
      return {
        items: page.items.map((item) =>
          latestSourceJob(
            currentJobs.current.find((previous) => previous.id === item.id),
            item,
          ),
        ),
        next_cursor: page.next_cursor,
      };
    },
  ).pager;
  const pageJobs = jobsList.page?.items ?? [];
  const matched = located?.id === targetId ? located : null;
  const jobs = pageJobs.map((job) =>
    matched?.id === job.id ? latestSourceJob(job, matched) : job,
  );
  if (matched && !jobs.some((job) => job.id === matched.id)) jobs.unshift(matched);
  currentJobs.current = jobs;
  const loaded = jobsList.page !== null;
  const busy = jobsList.loading || !!canceling;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    let live = true;
    setLocated(null);
    setLocateError(null);
    if (targetId)
      void call(
        { kind: 'source_job_get', payload: { job_id: targetId } },
        'source_job',
        identity,
      ).then(
        (job) => {
          if (live) {
            if (job.id === targetId) setLocated(job);
            else
              setLocateError(
                t(
                  '任务响应与所选任务不一致，请重新读取。',
                  'The response does not match this task. Reload it.',
                ),
              );
          }
        },
        (failure) => {
          if (live) setLocateError(message(failure));
        },
      );
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, targetId, refresh, locateRevision]);
  useEffect(() => {
    const id = actions.lastJob?.jobId;
    if (id && id !== lastAcceptedJob.current) setTarget({ section: 'sources', taskId: id });
    lastAcceptedJob.current = id;
  }, [actions.lastJob?.jobId]);
  useLayoutEffect(() => {
    if (!targetId) {
      lastTarget.current = '';
      return;
    }
    if (lastTarget.current === targetId) return;
    const row = sectionElement.current?.querySelector<HTMLElement>(
      `[data-task-id="${CSS.escape(targetId)}"]`,
    );
    if (row) {
      row.scrollIntoView({ block: 'nearest' });
      lastTarget.current = targetId;
    }
  }, [targetId, located, jobsList.page]);
  useLayoutEffect(() => {
    const body = sectionElement.current?.closest<HTMLElement>('.page-frame-body');
    if (body && jobsList.page && !jobsList.loading && !targetId) body.scrollTop = scroll;
  }, [jobsList.page, jobsList.loading, targetId]);
  /**
   * Only jobs already known to be active are fetched, so the cadence follows the id
   * set: re-arming on it drops to the idle interval when nothing is running and
   * starts the fast cadence the moment an import begins.
   */
  const activeKey = jobs
    .filter(active)
    .map((job) => job.id)
    .join('|');
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    let visible = document.visibilityState !== 'hidden';
    let failures = 0;
    const activeIds = () => currentJobs.current.filter(active).map((job) => job.id);
    const schedule = () => {
      clearTimeout(timer);
      timer = setTimeout(
        () => void poll(),
        nextSourcePollDelay({ visible, activeJobs: activeIds().length, failures }),
      );
    };
    async function poll() {
      const ids = activeIds();
      const results = await Promise.allSettled(
        ids.map((job_id) =>
          call({ kind: 'source_job_get', payload: { job_id } }, 'source_job', identity),
        ),
      );
      if (!live) return;
      const updates = results.flatMap((result) =>
        result.status === 'fulfilled' ? [result.value] : [],
      );
      if (updates.length) {
        setLocated((old) =>
          old ? latestSourceJob(old, updates.find((job) => job.id === old.id) ?? old) : old,
        );
        jobsList.updateItems((old) =>
          old.map((job) =>
            latestSourceJob(job, updates.find((update) => update.id === job.id) ?? job),
          ),
        );
      }
      failures = results.some((result) => result.status === 'rejected') ? failures + 1 : 0;
      const failure = results.find((result) => result.status === 'rejected');
      if (failure?.status === 'rejected') setError(message(failure.reason));
      schedule();
    }
    const onVisibilityChange = () => {
      const next = document.visibilityState !== 'hidden';
      if (next === visible) return;
      visible = next;
      clearTimeout(timer);
      const transition = sourcePollOnVisibilityChange({
        visible,
        activeJobs: activeIds().length,
        failures,
      });
      // Hidden work keeps only the heartbeat; a window coming back sweeps at once.
      if (transition.pollNow) void poll();
      else timer = setTimeout(() => void poll(), transition.delay);
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    schedule();
    return () => {
      live = false;
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [identity.local_instance_id, identity.workspace_id, activeKey, jobsList.scope]);
  async function cancel(job: SourceJob) {
    if (canceling) return;
    setCanceling(job.id);
    setError(null);
    try {
      const updated = await call(
        { kind: 'source_job_cancel', payload: { job_id: job.id } },
        'source_job',
        identity,
      );
      if (alive.current) {
        setLocated((old) => (old?.id === updated.id ? latestSourceJob(old, updated) : old));
        jobsList.updateItems((old) =>
          old.map((value) => (value.id === updated.id ? latestSourceJob(value, updated) : value)),
        );
      }
    } catch (e) {
      if (alive.current) setError(message(e));
    } finally {
      if (alive.current) setCanceling(null);
    }
  }
  return (
    <section
      ref={sectionElement}
      className="task-section"
      aria-label={t('资料处理任务', 'Document processing tasks')}
    >
      <h2>
        <FileText size={17} />
        {t('资料处理', 'Document processing')}
      </h2>
      <p className="muted">
        {t(
          '包含所有资料导入与网页采集任务。重试会新建任务，原记录保留。',
          'All document imports and web captures. Retrying creates a new task and preserves the original record.',
        )}
      </p>
      {(error || actions.error || jobsList.error || locateError) && (
        <div className="task-error" role="alert">
          <div>
            {error && <p>{error}</p>}
            {locateError && (
              <p>
                {t('无法定位资料任务：', 'Could not locate document task: ')}
                {locateError}{' '}
                <button
                  className="text-button"
                  onClick={() => setLocateRevision((value) => value + 1)}
                >
                  {t('重试定位', 'Retry locating')}
                </button>
              </p>
            )}
            {actions.error && <p>{actions.error}</p>}
            {jobsList.error && <p>{jobsList.error}</p>}
          </div>
        </div>
      )}
      {(!actions.ready || actions.pending) && (
        <div className="task-report">
          <p>
            {actions.pending
              ? t(
                  '上次重试的受理结果待确认；将使用原请求重新确认。',
                  'The last retry needs confirmation; its original request will be replayed.',
                )
              : t(
                  '正在读取本机待确认请求。读取失败时可重新读取。',
                  'Reading local pending requests. Reload if reading failed.',
                )}
          </p>
          <button
            className="secondary-button"
            disabled={actions.busy}
            onClick={() => void actions.retry()}
          >
            {t('重新确认 / 读取', 'Confirm / reload')}
          </button>
        </div>
      )}
      {jobsList.loading && !error && (
        <p className="task-empty">{t('正在读取资料任务…', 'Loading document tasks…')}</p>
      )}
      {loaded && !jobs.length && (
        <p className="task-empty">
          {t('还没有资料处理任务。', 'No document processing tasks yet.')}
        </p>
      )}
      {jobs.map((job) => (
        <article
          className={`task-row${targetId === job.id ? ' is-target' : ''}`}
          data-task-id={job.id}
          key={job.id}
        >
          <FileText size={19} />
          <div className="task-body">
            {targetId === job.id && (
              <span className="task-located-label">{t('定位的任务', 'Located task')}</span>
            )}
            <strong>{job.input.original_name || t('网页采集', 'Web capture')}</strong>
            <small>
              {job.attempt === 0
                ? t('尚未开始执行', 'Execution has not started')
                : t(`第 ${job.attempt} 次尝试`, `Attempt ${job.attempt}`)}{' '}
              · {displayDate(job.updated_at, language)}
            </small>
            <p className={`task-status task-status-${job.status}`}>
              <SourceStage
                status={job.status}
                stage={job.stage}
                cancelRequested={job.cancel_requested}
              />
            </p>
            {job.error_code && job.error_code !== 'canceled' && (
              <div className="task-failure">
                <p>
                  {t('最近一次处理信息：', 'Latest processing detail: ')}
                  {/* The sentence is what the user acts on; the machine code stays
                      in the tooltip, exactly as the import dialog shows it. */}
                  <span title={job.error_code}>{sourceImportFailure(job.error_code, t)}</span>
                </p>
              </div>
            )}
            {job.source_id && job.source_version_id && (
              <button
                className="text-button"
                onClick={() => onOpenSource(job.source_id!, job.source_version_id!)}
              >
                {job.status === 'succeeded'
                  ? t('打开处理结果', 'Open result')
                  : t('打开已保存的资料', 'Open saved document')}
              </button>
            )}
            <button
              className="text-button"
              aria-expanded={selected === job.id}
              onClick={() => setSelected(selected === job.id ? null : job.id)}
            >
              {t('查看处理记录', 'View processing history')}
            </button>
            {selected === job.id && <SourceEvents key={job.id} identity={identity} job={job} />}
          </div>
          {active(job) ? (
            <button
              className="secondary-button"
              disabled={job.cancel_requested || !!canceling}
              onClick={() => void cancel(job)}
            >
              <X size={14} />
              {t('取消', 'Cancel')}
            </button>
          ) : (
            (job.status === 'failed' || job.status === 'canceled') && (
              <button
                className="secondary-button"
                disabled={actions.locked}
                onClick={() =>
                  void actions.execute(
                    {
                      kind: 'source_job_retry',
                      payload: { command_id: crypto.randomUUID(), job_id: job.id },
                    },
                    'job_accepted',
                  )
                }
              >
                <RefreshCw size={14} />
                {t('重试', 'Retry')}
              </button>
            )
          )}
        </article>
      ))}
      <div className="source-task-controls">
        <button
          className="secondary-button"
          disabled={busy}
          onClick={() => void jobsList.refresh()}
        >
          {t('重新读取列表', 'Reload list')}
        </button>
      </div>
      <PageFooter>
        <ListPagination list={jobsList} disabled={!!canceling} />
      </PageFooter>
    </section>
  );
}

function SourceStage({
  status,
  stage,
  cancelRequested = false,
}: {
  status: string;
  stage: string;
  cancelRequested?: boolean;
}) {
  const { t } = useI18n();
  const stages: Record<string, string> = {
    queued: t('等待处理', 'Queued'),
    capture_pending: t('等待获取原件', 'Waiting to capture original'),
    resume_pending: t('执行已中断，等待再次处理', 'Interrupted; waiting to resume'),
    capturing: t('正在获取原件', 'Capturing original'),
    original_saved: t('原件已保存', 'Original saved'),
    parse_pending: t('等待解析', 'Waiting to parse'),
    parsing: t('正在解析正文', 'Parsing text'),
    indexing: t('正在建立检索索引', 'Indexing'),
    lexical_ready: t('已完成，可检索', 'Complete and searchable'),
    needs_ocr: t(
      '原件已保存；存在没有可提取文字的页面',
      'Original saved; includes pages without extractable text',
    ),
  };
  const terminal =
    status === 'failed'
      ? t('未完成', 'Failed')
      : status === 'canceled'
        ? t('已取消', 'Canceled')
        : status === 'succeeded'
          ? t('已完成', 'Completed')
          : null;
  if (terminal)
    return (
      <>
        {terminal}
        {/* A finished job stores the status word in its stage column, so only a stage
            this interface can name is appended; the event list keeps showing the real
            stage of each attempt. */}
        {stages[stage] ? ` · ${stages[stage]}` : null}
      </>
    );
  if (cancelRequested)
    return <>{t('已请求取消，等待实际终态', 'Cancellation requested; awaiting final status')}</>;
  return <>{stages[stage] ?? `${t('正在处理', 'Processing')} · ${stage}`}</>;
}

function SourceEvents({ identity, job }: { identity: WorkspaceIdentity; job: SourceJob }) {
  const { t, language } = useI18n();
  const [events, setEvents] = useState<SourceJobEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [more, setMore] = useState(true);
  const [refresh, setRefresh] = useState(0);
  const sequence = useRef(0);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      setBusy(true);
      try {
        const items = await call(
          {
            kind: 'source_job_events',
            payload: { job_id: job.id, after_seq: sequence.current, limit: 50 },
          },
          'source_job_events',
          identity,
        );
        if (!live) return;
        setEvents((old) => [...old, ...items]);
        if (items.length) sequence.current = items[items.length - 1].seq;
        setMore(items.length === 50);
        setError(null);
        if (items.length < 50 && active(job)) timer = setTimeout(() => void read(), 1800);
      } catch (e) {
        if (live) setError(message(e));
      } finally {
        if (live) setBusy(false);
      }
    }
    void read();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [job.id, job.status, job.attempt, refresh, identity]);
  return (
    <div className="task-report">
      {error && (
        <p role="alert" className="task-failure">
          {error}
        </p>
      )}
      {!events.length && !error && (
        <p>
          {busy ? t('读取事件…', 'Loading events…') : t('没有事件记录。', 'No recorded events.')}
        </p>
      )}
      <ol className="source-task-events">
        {events.map((event) => (
          <li key={event.seq}>
            <strong>
              {event.attempt !== null
                ? event.attempt === 0
                  ? t('执行前', 'Before execution')
                  : t(`第 ${event.attempt} 次尝试`, `Attempt ${event.attempt}`)
                : t('尝试编号未记录', 'Attempt not recorded')}
            </strong>{' '}
            · <small>{displayDate(event.created_at, language)}</small> ·{' '}
            <SourceStage status={event.status} stage={event.stage} />
          </li>
        ))}
      </ol>
      {(more || error) && (
        <button
          className="text-button"
          disabled={busy}
          onClick={() => setRefresh((value) => value + 1)}
        >
          {error
            ? t('重新读取事件', 'Retry reading events')
            : t('加载更多事件', 'Load more events')}
        </button>
      )}
    </div>
  );
}
