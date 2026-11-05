import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  Activity,
  Archive,
  Check,
  CircleAlert,
  Clock3,
  Download,
  FolderOpen,
  FlaskConical,
  LoaderCircle,
  RefreshCw,
  ShieldCheck,
  X,
} from 'lucide-react';
import type {
  AppOperation,
  BackupReport,
  OperationKind,
  PortableJob,
  ResearchTaskSummary,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message, revealOutputJob } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { operationErrorMessage } from '../errors';
import { portableActions, usePortableActions } from './actions';
import { actionErrorText } from './pending-actions';
import './task-center.css';
import { SourceTasks } from './SourceTasks';
import { nextPollDelay, pollOnVisibilityChange } from './poll-schedule';
import { ListPagination } from '../layout/Pagination.tsx';
import { usePageNavigationPaging } from '../layout/usePageNavigationPaging';
import { readResearchPage } from '../automation/automation-pages';
import { acceptedTaskDestination, type TaskDestination, type TaskSection } from './task-navigation';
import { PageState } from '../layout/PageState';
import { PageFrame } from '../layout/PageFrame';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { resolveListDisplayState } from '../layout/page-state';

const scopeKey = (identity: WorkspaceIdentity | null) =>
  identity ? `${identity.local_instance_id}/${identity.workspace_id}` : '';
const running = (status: string) => status === 'queued' || status === 'running';
const EXPORT_FORMAT_LABELS = {
  markdown: 'Markdown',
  html: 'HTML',
  docx: 'DOCX',
  pdf: 'PDF',
} as const;

export function TaskCenter({
  identity,
  onOpenWorkspace,
  onOpenSource,
  onOpenResearch,
}: {
  identity: WorkspaceIdentity | null;
  onOpenWorkspace: (identity: WorkspaceIdentity) => void;
  onOpenSource: (id: string, versionId?: string) => void;
  onOpenResearch: (taskId: string) => void;
}) {
  const { t, language } = useI18n();
  const requests = usePortableActions();
  const [section, setSection] = usePageNavigationState<TaskSection>(
    'tasks.section',
    identity ? 'sources' : 'operations',
  );
  const [target, setTarget] = usePageNavigationState<TaskDestination | null>('tasks.target', null);
  const [scroll, setScroll] = usePageNavigationState(`tasks.${section}.scroll`, 0);
  const content = useRef<HTMLDivElement>(null);
  const lastTarget = useRef('');
  function chooseSection(next: TaskSection) {
    setTarget(null);
    setSection(next);
  }
  /**
   * Operations are app-level: they exist before a workspace is open, so this
   * list never depends on the active library. Both lists page through the
   * backend cursor, which is what gives the task centre real page numbers.
   */
  const operations = usePageNavigationPaging<AppOperation>(
    'app',
    'tasks.operations',
    (cursor, size) =>
      call({ kind: 'operations_page', payload: { limit: size, cursor } }, 'operations_page'),
  ).pager;
  const jobs = usePageNavigationPaging<PortableJob>(
    identity ? scopeKey(identity) : null,
    'tasks.exports',
    (cursor, size) =>
      call({ kind: 'jobs_page', payload: { limit: size, cursor } }, 'jobs_page', identity!),
  ).pager;
  const research = usePageNavigationPaging<ResearchTaskSummary>(
    identity ? scopeKey(identity) : null,
    'tasks.research',
    (cursor, size) => readResearchPage(identity!, cursor, size),
  ).pager;
  type Located =
    | { section: 'exports'; item: PortableJob }
    | { section: 'operations'; item: AppOperation }
    | { section: 'research'; item: ResearchTaskSummary };
  const [located, setLocated] = useState<Located | null>(null);
  const [targetError, setTargetError] = useState<string | null>(null);
  const [targetBusy, setTargetBusy] = useState(false);
  const targetSequence = useRef(0);
  async function loadTarget() {
    const sequence = ++targetSequence.current;
    if (!target?.taskId || target.section !== section || section === 'sources') {
      setLocated(null);
      setTargetError(null);
      setTargetBusy(false);
      return true;
    }
    setTargetBusy(true);
    try {
      let value: Located;
      if (section === 'operations')
        value = {
          section,
          item: await call(
            { kind: 'operation_get', payload: { operation_id: target.taskId } },
            'operation',
          ),
        };
      else if (!identity)
        throw new Error(t('请先打开任务所属知识库。', 'Open the task’s workspace first.'));
      else if (section === 'exports')
        value = {
          section,
          item: await call(
            { kind: 'job_get', payload: { job_id: target.taskId } },
            'job',
            identity,
          ),
        };
      else
        value = {
          section,
          item: await call(
            { kind: 'research_get', payload: { task_id: target.taskId } },
            'research_task',
            identity,
          ),
        };
      if (value.item.id !== target.taskId)
        throw new Error(
          t(
            '任务响应与所选任务不一致，请重新读取。',
            'The response does not match this task. Reload it.',
          ),
        );
      if (sequence !== targetSequence.current) return false;
      setLocated(value);
      setTargetError(null);
      return true;
    } catch (error) {
      if (sequence === targetSequence.current) {
        setLocated(null);
        setTargetError(message(error));
      }
      return false;
    } finally {
      if (sequence === targetSequence.current) setTargetBusy(false);
    }
  }
  const loadTargetRef = useRef(loadTarget);
  loadTargetRef.current = loadTarget;
  const [actionError, setActionError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [canceling, setCanceling] = useState<string[]>([]);
  const [revealing, setRevealing] = useState<string | null>(null);
  const [revealErrors, setRevealErrors] = useState<Record<string, string>>({});
  const key = scopeKey(identity);
  const jobsItems = jobs.page?.items ?? [];
  const operationsItems = operations.page?.items ?? [];
  const researchItems = research.page?.items ?? [];
  const jobsState = identity
    ? resolveListDisplayState({
        hasPage: Boolean(jobs.page),
        itemCount: jobsItems.length,
        loading: jobs.loading,
        error: jobs.error,
      })
    : null;
  const operationsState = resolveListDisplayState({
    hasPage: Boolean(operations.page),
    itemCount: operationsItems.length,
    loading: operations.loading,
    error: operations.error,
  });
  const researchState = identity
    ? resolveListDisplayState({
        hasPage: Boolean(research.page),
        itemCount: researchItems.length,
        loading: research.loading,
        error: research.error,
      })
    : null;
  const focused =
    located && target?.taskId === located.item.id && section === located.section ? located : null;
  const targetKey =
    target?.taskId && target.section === section ? `${key}/${section}/${target.taskId}` : '';
  useEffect(() => {
    setLocated(null);
    setTargetError(null);
    void loadTargetRef.current();
    return () => {
      ++targetSequence.current;
    };
  }, [key, section, target?.section, target?.taskId, refresh]);
  useLayoutEffect(() => {
    const body = content.current?.parentElement;
    if (!body) return;
    body.scrollTop = scroll;
    const remember = () => setScroll(body.scrollTop);
    body.addEventListener('scroll', remember, { passive: true });
    return () => body.removeEventListener('scroll', remember);
  }, [key, section]);
  useLayoutEffect(() => {
    if (!targetKey) {
      lastTarget.current = '';
      return;
    }
    if (lastTarget.current === targetKey || targetBusy) return;
    const row = content.current?.querySelector<HTMLElement>(
      `[data-task-id="${CSS.escape(target!.taskId!)}"]`,
    );
    if (!row) return;
    row.scrollIntoView({ block: 'nearest' });
    lastTarget.current = targetKey;
  }, [targetKey, targetBusy, focused, jobs.page, operations.page, research.page]);
  useLayoutEffect(() => {
    const body = content.current?.parentElement;
    const active =
      section === 'exports'
        ? jobs
        : section === 'operations'
          ? operations
          : section === 'research'
            ? research
            : null;
    if (body && active?.page && !active.loading && !targetKey) body.scrollTop = scroll;
  }, [
    section,
    jobs.page,
    jobs.loading,
    operations.page,
    operations.loading,
    research.page,
    research.loading,
    targetKey,
  ]);
  const taskErrors = [
    ...new Set(
      [actionError, operations.error, jobs.error, research.error, requests.error].filter(
        (item): item is string => Boolean(item),
      ),
    ),
  ];
  /**
   * The poll re-reads the page on screen, so it must not depend on the
   * controller object: `usePagedList` hands back a new one every render.
   */
  const operationsRef = useRef(operations);
  operationsRef.current = operations;
  const jobsRef = useRef(jobs);
  jobsRef.current = jobs;
  const researchRef = useRef(research);
  researchRef.current = research;
  const focusedRef = useRef(focused);
  focusedRef.current = focused;
  const failures = useRef(0);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    let visible = document.visibilityState !== 'hidden';
    failures.current = 0;
    const counts = () => ({
      pendingOperations: (operationsRef.current.page?.items ?? []).filter((operation) =>
        running(operation.status),
      ).length,
      activeJobs:
        (jobsRef.current.page?.items ?? []).filter((job) => running(job.status)).length +
        (researchRef.current.page?.items ?? []).filter((task) => task.status === 'running').length +
        (focusedRef.current && running(focusedRef.current.item.status) ? 1 : 0),
    });
    const schedule = () => {
      clearTimeout(timer);
      timer = setTimeout(
        () => void poll(),
        nextPollDelay({ visible, ...counts(), failures: failures.current }),
      );
    };
    async function poll() {
      // Refreshing keeps the page number and the cursor stack: a reader on
      // page three stays on page three while the rows are re-read.
      await Promise.allSettled([
        operationsRef.current.refresh(),
        identity ? jobsRef.current.refresh() : Promise.resolve(true),
        identity ? researchRef.current.refresh() : Promise.resolve(true),
        loadTargetRef.current(),
      ]);
      if (!live) return;
      // The two sources fail and recover on their own: one failing list only
      // lengthens the next wait, and the banner shows that list's own error.
      const failed = Boolean(
        operationsRef.current.error ||
        (identity && (jobsRef.current.error || researchRef.current.error)),
      );
      failures.current = failed ? failures.current + 1 : 0;
      schedule();
    }
    const onVisibilityChange = () => {
      const next = document.visibilityState !== 'hidden';
      if (next === visible) return;
      visible = next;
      clearTimeout(timer);
      const transition = pollOnVisibilityChange({
        visible,
        ...counts(),
        failures: failures.current,
      });
      // Hidden work falls back to the heartbeat; a window coming back reads the
      // current state right away instead of showing what was true while hidden.
      if (transition.pollNow) void poll();
      else timer = setTimeout(() => void poll(), transition.delay);
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    void poll();
    return () => {
      live = false;
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [key, refresh]);
  async function retry(id: string) {
    setActionError(null);
    // The queue names a request it refused for good in its own dismissable
    // notice: repeating that text here would report one failure twice.
    const queued = requests.pending.find((entry) => entry.id === id);
    try {
      const accepted = await portableActions.retry(id);
      const destination = acceptedTaskDestination(accepted);
      setTarget(destination);
      setSection(destination.section);
      setRefresh((value) => value + 1);
    } catch (error) {
      setActionError(actionErrorText(error, queued));
    }
  }
  async function cancel(id: string, kind: 'job' | 'operation') {
    if (canceling.includes(id)) return;
    setCanceling((old) => [...old, id]);
    setActionError(null);
    try {
      if (kind === 'job')
        await call({ kind: 'job_cancel', payload: { job_id: id } }, 'job', identity);
      else await call({ kind: 'operation_cancel', payload: { operation_id: id } }, 'operation');
      setRefresh((value) => value + 1);
    } catch (error) {
      setActionError(message(error));
    } finally {
      setCanceling((old) => old.filter((value) => value !== id));
    }
  }
  async function reveal(job: PortableJob) {
    if (!identity || job.status !== 'succeeded' || revealing) return;
    setRevealing(job.id);
    setRevealErrors((old) => {
      const next = { ...old };
      delete next[job.id];
      return next;
    });
    try {
      await revealOutputJob(identity, job.id);
    } catch (error) {
      setRevealErrors((old) => ({ ...old, [job.id]: message(error) }));
    } finally {
      setRevealing(null);
    }
  }
  const operationTitle = (kind: AppOperation['kind']) =>
    ({
      create_workspace: t('创建知识库', 'Create workspace'),
      open_workspace: t('打开知识库', 'Open workspace'),
      verify_backup: t('校验备份', 'Verify backup'),
      restore_backup: t('恢复备份', 'Restore backup'),
    })[kind];
  function renderJob(job: PortableJob) {
    return (
      <article
        className={`task-row${target?.section === 'exports' && target.taskId === job.id ? ' is-target' : ''}`}
        data-task-id={job.id}
        key={job.id}
      >
        {job.input.kind === 'backup_create' ? <Archive size={19} /> : <Download size={19} />}
        <div className="task-body">
          <strong>
            {job.input.kind === 'backup_create'
              ? t('完整备份', 'Full backup')
              : job.input.kind === 'export_diagnostics'
                ? t('导出诊断包', 'Export diagnostics')
                : job.input.kind === 'export_source'
                  ? t('导出资料原件', 'Export document original')
                  : t(
                      `导出笔记 · ${EXPORT_FORMAT_LABELS[job.input.format]}`,
                      `Export note · ${EXPORT_FORMAT_LABELS[job.input.format]}`,
                    )}
          </strong>
          <small>{displayDate(job.updated_at, language)}</small>
          <TaskStatus
            status={job.status}
            stage={job.stage}
            cancelRequested={job.cancel_requested}
          />
          {job.error_code && <Failure code={job.error_code} id={job.id} />}
          {job.result?.kind === 'backup' && <Report report={job.result.report} />}
          {job.result?.kind === 'diagnostics' && (
            <div className="task-report">
              <p>
                {t('诊断包已写入所选位置。', 'Diagnostics bundle saved to the selected location.')}
              </p>
              <details>
                <summary>{t('查看导出清单', 'View exported files')}</summary>
                <ul>
                  {job.result.report.files.map((file) => (
                    <li key={file.name}>
                      {file.name} · {file.bytes.toLocaleString(language)} B
                    </li>
                  ))}
                </ul>
                <p>
                  SHA-256 <code>{job.result.report.sha256}</code>
                </p>
              </details>
            </div>
          )}
          {job.result?.kind === 'source_export' && (
            <div className="task-report">
              <p>
                {t(
                  '资料原件已导出到所选位置，可在压缩包中打开。',
                  'The document original was exported to the chosen location; open it from the archive.',
                )}
              </p>
              <p>
                <code>{job.result.report.original_path}</code>
              </p>
              {!!job.result.report.warnings.length && (
                <details>
                  <summary>{t('说明', 'Notes')}</summary>
                  <ul>
                    {job.result.report.warnings.map((warning, index) => (
                      <li key={index}>{warning}</li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}
          {job.result?.kind === 'export' && job.input.kind === 'export_note' && (
            <div className="task-report">
              <p>
                {job.input.format === 'docx' || job.input.format === 'pdf'
                  ? t(
                      `已生成 ${EXPORT_FORMAT_LABELS[job.input.format]} 文件，内嵌 ${job.result.report.attachment_count} 个图片。`,
                      `${EXPORT_FORMAT_LABELS[job.input.format]} file created with ${job.result.report.attachment_count} embedded ${job.result.report.attachment_count === 1 ? 'image' : 'images'}.`,
                    )
                  : t(
                      `已生成 ${EXPORT_FORMAT_LABELS[job.input.format]} 压缩包，内含正文、原始结构与 ${job.result.report.attachment_count} 个附件。`,
                      `${EXPORT_FORMAT_LABELS[job.input.format]} archive created with the document, structured original, and ${job.result.report.attachment_count} ${job.result.report.attachment_count === 1 ? 'attachment' : 'attachments'}.`,
                    )}
              </p>
              <p>
                <code>{job.result.report.document_path}</code>
              </p>
              {!!job.result.report.warnings.length && (
                <details>
                  <summary>{t('格式转换说明', 'Format conversion notes')}</summary>
                  <ul>
                    {job.result.report.warnings.map((warning, index) => (
                      <li key={index}>{warning}</li>
                    ))}
                  </ul>
                </details>
              )}
            </div>
          )}
          {job.status === 'succeeded' && identity && (
            <button
              className="text-button"
              disabled={revealing !== null}
              onClick={() => void reveal(job)}
            >
              <FolderOpen size={14} aria-hidden="true" />
              {revealing === job.id
                ? t('正在打开…', 'Opening…')
                : t('打开所在文件夹', 'Open containing folder')}
            </button>
          )}
          {revealErrors[job.id] && (
            <p className="task-failure" role="alert">
              {revealErrors[job.id]}
            </p>
          )}
        </div>
        {running(job.status) && (
          <button
            className="secondary-button"
            disabled={job.cancel_requested || canceling.includes(job.id)}
            onClick={() => void cancel(job.id, 'job')}
          >
            <X size={14} />
            {t('取消', 'Cancel')}
          </button>
        )}
      </article>
    );
  }
  function renderOperation(operation: AppOperation) {
    return (
      <article
        className={`task-row${target?.section === 'operations' && target.taskId === operation.id ? ' is-target' : ''}`}
        data-task-id={operation.id}
        key={operation.id}
      >
        {operation.kind === 'verify_backup' ? <ShieldCheck size={19} /> : <FolderOpen size={19} />}
        <div className="task-body">
          <strong>{operationTitle(operation.kind)}</strong>
          <small>{displayDate(operation.updated_at, language)}</small>
          <TaskStatus
            status={operation.status}
            stage={operation.stage}
            cancelRequested={operation.cancel_requested}
          />
          {operation.error_code && (
            <Failure code={operation.error_code} id={operation.id} kind={operation.kind} />
          )}
          {operation.report && <Report report={operation.report} />}
          {operation.kind === 'restore_backup' &&
            operation.status === 'succeeded' &&
            operation.result && (
              <button
                className="text-button"
                onClick={() => onOpenWorkspace(operation.result!.identity)}
              >
                <FolderOpen size={14} />
                {t('打开恢复的知识库', 'Open recovered workspace')}
              </button>
            )}
        </div>
        {running(operation.status) && (
          <button
            className="secondary-button"
            disabled={operation.cancel_requested || canceling.includes(operation.id)}
            onClick={() => void cancel(operation.id, 'operation')}
          >
            <X size={14} />
            {t('取消', 'Cancel')}
          </button>
        )}
      </article>
    );
  }
  function renderResearch(task: ResearchTaskSummary) {
    const status = {
      pending: t('待开始', 'Pending'),
      running: t('进行中', 'Running'),
      completed: t('已完成', 'Completed'),
      canceled: t('已取消', 'Canceled'),
      failed: t('未完成', 'Failed'),
    }[task.status];
    return (
      <article
        className={`task-row${target?.section === 'research' && target.taskId === task.id ? ' is-target' : ''}`}
        data-task-id={task.id}
        key={task.id}
      >
        <FlaskConical size={19} aria-hidden="true" />
        <div className="task-body">
          <strong>{task.goal}</strong>
          <small>{displayDate(task.updated_at, language)}</small>
          <p className="task-status">
            {status} · {t(`${task.step_count} 个步骤`, `${task.step_count} steps`)}
          </p>
          <button className="text-button" onClick={() => onOpenResearch(task.id)}>
            {t('打开研究任务', 'Open research task')}
          </button>
        </div>
      </article>
    );
  }
  return (
    <PageFrame
      className="task-center"
      nestedFooter={section === 'sources' && Boolean(identity)}
      footer={
        section === 'exports' ? (
          <ListPagination list={jobs} />
        ) : section === 'operations' ? (
          <ListPagination list={operations} />
        ) : section === 'research' ? (
          <ListPagination list={research} />
        ) : undefined
      }
      toolbar={
        <div
          className="view-switch task-kind-tabs"
          role="group"
          aria-label={t('任务分类', 'Task category')}
        >
          <button
            disabled={!identity}
            aria-pressed={section === 'sources'}
            onClick={() => chooseSection('sources')}
          >
            {t('资料处理', 'Document processing')}
          </button>
          <button
            disabled={!identity}
            aria-pressed={section === 'exports'}
            onClick={() => chooseSection('exports')}
          >
            {t('导出与备份', 'Exports & backups')}
          </button>
          <button
            aria-pressed={section === 'operations'}
            onClick={() => chooseSection('operations')}
          >
            {t('应用操作', 'App operations')}
          </button>
          <button
            disabled={!identity}
            aria-pressed={section === 'research'}
            onClick={() => chooseSection('research')}
          >
            {t('研究任务', 'Research')}
          </button>
        </div>
      }
      title={t('任务中心', 'Activity')}
      description={t(
        '查看资料处理、导出、备份与研究的实际进度。',
        'Track document processing, exports, backups, and research.',
      )}
      actions={
        <button className="secondary-button" onClick={() => setRefresh((value) => value + 1)}>
          <RefreshCw size={15} />
          {t('刷新', 'Refresh')}
        </button>
      }
    >
      <div className="task-content" ref={content}>
        {taskErrors.length > 0 && (
          <div className="task-error" role="alert">
            <CircleAlert size={17} />
            <div>
              {taskErrors.map((error, index) => (
                <p key={index}>{error}</p>
              ))}
              {!requests.loaded && (
                <button
                  className="text-button"
                  onClick={() => void portableActions.load().catch(() => undefined)}
                >
                  {t('重新读取请求记录', 'Reload request records')}
                </button>
              )}
            </div>
          </div>
        )}
        {requests.notice && (
          <div className="task-notice" role="status">
            <CircleAlert size={17} />
            <div>
              <p>
                {t(
                  `已放弃 1 条无法继续的旧请求：${requests.notice.displayName} · ${displayDate(requests.notice.createdAt, language)} · ${requests.notice.message}`,
                  `Dropped one request that cannot continue: ${requests.notice.displayName} · ${displayDate(requests.notice.createdAt, language)} · ${requests.notice.message}`,
                )}
              </p>
              <button className="text-button" onClick={() => portableActions.dismissNotice()}>
                {t('知道了', 'Dismiss')}
              </button>
            </div>
          </div>
        )}
        {requests.pending.length > 0 && (
          <section className="task-section">
            <h2>{t('等待确认的请求', 'Requests awaiting confirmation')}</h2>
            <p className="muted">
              {t(
                '这些请求可能已受理。重新确认会使用原请求，不会重复创建任务。',
                'These requests may already be accepted. Check again with the original request to avoid duplicates.',
              )}
            </p>
            {requests.pending.map((request) => {
              const local = !request.identity || scopeKey(request.identity) === key;
              const title =
                request.command.kind === 'backup_create'
                  ? t('完整备份', 'Full backup')
                  : request.command.kind === 'note_export'
                    ? t('导出笔记', 'Export note')
                    : request.command.kind === 'diagnostics_export'
                      ? t('导出诊断包', 'Export diagnostics')
                      : request.command.kind === 'backup_verify'
                        ? t('校验备份', 'Verify backup')
                        : t('恢复备份', 'Restore backup');
              return (
                <article className="task-row" key={request.id}>
                  <Clock3 size={19} />
                  <div className="task-body">
                    <strong>{title}</strong>
                    <p>{request.displayName}</p>
                    <small>{displayDate(request.createdAt, language)}</small>
                  </div>
                  {local ? (
                    <button
                      className="secondary-button"
                      disabled={requests.busy.includes(request.id)}
                      onClick={() => void retry(request.id)}
                    >
                      {requests.busy.includes(request.id)
                        ? t('正在确认…', 'Checking…')
                        : t('重新确认', 'Check again')}
                    </button>
                  ) : (
                    <button
                      className="secondary-button"
                      onClick={() => onOpenWorkspace(request.identity!)}
                    >
                      {t('打开原知识库', 'Open original workspace')}
                    </button>
                  )}
                </article>
              );
            })}
          </section>
        )}
        {target?.taskId && target.section === section && section !== 'sources' && (
          <section className="task-located" aria-label={t('定位的任务', 'Located task')}>
            <div className="task-located-heading">
              <strong>{t('定位的任务', 'Located task')}</strong>
              <button className="text-button" onClick={() => setTarget(null)}>
                {t('取消定位', 'Clear selection')}
              </button>
            </div>
            {targetError ? (
              <p role="alert">
                {targetError}{' '}
                <button
                  className="text-button"
                  disabled={targetBusy}
                  onClick={() => void loadTarget()}
                >
                  {t('重新读取', 'Reload')}
                </button>
              </p>
            ) : !focused ? (
              <p role="status">{t('正在定位任务…', 'Locating task…')}</p>
            ) : focused.section === 'exports' ? (
              !jobsItems.some((item) => item.id === focused.item.id) ? (
                renderJob(focused.item)
              ) : (
                <p>
                  {t('该任务已在下方列表中标记。', 'The task is highlighted in the list below.')}
                </p>
              )
            ) : focused.section === 'operations' ? (
              !operationsItems.some((item) => item.id === focused.item.id) ? (
                renderOperation(focused.item)
              ) : (
                <p>
                  {t('该任务已在下方列表中标记。', 'The task is highlighted in the list below.')}
                </p>
              )
            ) : !researchItems.some((item) => item.id === focused.item.id) ? (
              renderResearch(focused.item)
            ) : (
              <p>{t('该任务已在下方列表中标记。', 'The task is highlighted in the list below.')}</p>
            )}
          </section>
        )}
        {/* The key only tracks the workspace; a manual refresh reloads the list
          instead of remounting the section and dropping its in-flight polls. */}
        {identity && (
          <div hidden={section !== 'sources'}>
            <SourceTasks
              key={key}
              identity={identity}
              refresh={refresh}
              targetId={
                section === 'sources' && target?.section === 'sources' ? target.taskId : undefined
              }
              onOpenSource={onOpenSource}
            />
          </div>
        )}
        <section className="task-section" hidden={section !== 'exports'}>
          <h2>
            <Archive size={17} />
            {t('导出与备份', 'Exports and backups')}
          </h2>
          {!identity ? (
            <p className="task-empty">
              {t(
                '打开知识库后，可查看导出与备份任务。',
                'Open a workspace to see its exports and backups.',
              )}
            </p>
          ) : jobsState === 'loading' ? (
            <p className="task-empty task-loading" role="status">
              <LoaderCircle size={14} className="spin" aria-hidden="true" />
              {t('正在读取任务…', 'Loading tasks…')}
            </p>
          ) : jobsState === 'error' ? (
            <PageState
              className="task-state"
              compact
              kind="error"
              title={t('无法读取导出与备份任务', 'Could not load exports and backups')}
              retryLabel={t('重新读取任务', 'Reload tasks')}
              onRetry={() => void jobs.refresh()}
            />
          ) : jobsState === 'empty' ? (
            <p className="task-empty">
              {t('还没有导出或备份任务。', 'No exports or backups yet.')}
            </p>
          ) : (
            jobsItems.map(renderJob)
          )}
        </section>
        <section className="task-section" hidden={section !== 'operations'}>
          <h2>
            <Activity size={17} />
            {t('应用操作', 'App operations')}
          </h2>
          <p className="muted">
            {t(
              '知识库打开、备份校验与恢复的记录。恢复完成后，你可以自行打开新知识库。',
              'Workspace opening, backup verification, and recovery. Open a recovered workspace when you are ready.',
            )}
          </p>
          {operationsState === 'loading' ? (
            <p className="task-empty task-loading" role="status">
              <LoaderCircle size={14} className="spin" aria-hidden="true" />
              {t('正在读取操作…', 'Loading operations…')}
            </p>
          ) : operationsState === 'error' ? (
            <PageState
              className="task-state"
              compact
              kind="error"
              title={t('无法读取应用操作', 'Could not load app operations')}
              retryLabel={t('重新读取操作', 'Reload operations')}
              onRetry={() => void operations.refresh()}
            />
          ) : operationsState === 'empty' ? (
            <p className="task-empty">{t('还没有应用操作。', 'No app operations yet.')}</p>
          ) : (
            operationsItems.map(renderOperation)
          )}
        </section>
        <section className="task-section" hidden={section !== 'research'}>
          <h2>
            <FlaskConical size={17} />
            {t('研究任务', 'Research tasks')}
          </h2>
          {!identity ? (
            <p className="task-empty">
              {t('打开知识库后可查看研究任务。', 'Open a workspace to see its research tasks.')}
            </p>
          ) : researchState === 'loading' ? (
            <p role="status">{t('正在读取研究任务…', 'Loading research tasks…')}</p>
          ) : researchState === 'error' ? (
            <PageState
              compact
              kind="error"
              title={t('无法读取研究任务', 'Could not load research tasks')}
              onRetry={() => void research.refresh()}
            />
          ) : researchState === 'empty' ? (
            <p className="task-empty">{t('还没有研究任务。', 'No research tasks yet.')}</p>
          ) : (
            researchItems.map(renderResearch)
          )}
        </section>
      </div>
    </PageFrame>
  );
}

function TaskStatus({
  status,
  stage,
  cancelRequested,
}: {
  status: string;
  stage: string;
  cancelRequested: boolean;
}) {
  const { t } = useI18n();
  const stages: Record<string, string> = {
    queued: t('等待开始', 'Queued'),
    resume_pending: t('等待继续', 'Waiting to resume'),
    preparing: t('准备文件', 'Preparing files'),
    snapshot: t('创建一致快照', 'Creating snapshot'),
    copy_database: t('复制知识库', 'Copying workspace'),
    copy_blobs: t('复制附件', 'Copying attachments'),
    verify_files: t('校验文件', 'Verifying files'),
    verify_images: t('校验图片', 'Verifying images'),
    database_integrity: t('检查知识库完整性', 'Checking workspace integrity'),
    verify_revisions: t('校验历史版本', 'Verifying revisions'),
    rebuild_search: t('重建搜索索引', 'Rebuilding search index'),
    export_images: t('导出图片', 'Exporting images'),
    render_document: t('生成文档', 'Rendering document'),
    publishing: t('写入目标位置', 'Writing to destination'),
    restore_ready: t('准备完成恢复', 'Finalizing recovery'),
    validating: t('检查知识库', 'Checking workspace'),
    opening: t('打开知识库', 'Opening workspace'),
    registering: t('登记知识库', 'Registering workspace'),
    directory_ready: t('目标位置已就绪', 'Destination ready'),
  };
  const label =
    status === 'succeeded'
      ? t('已完成', 'Completed')
      : status === 'canceled'
        ? t('已取消', 'Canceled')
        : status === 'failed'
          ? t('未完成', 'Failed')
          : cancelRequested
            ? t('已请求取消，等待确认', 'Cancel requested; awaiting confirmation')
            : (stages[stage] ?? t('正在处理', 'Processing'));
  return (
    <p className={`task-status task-status-${status}`} role="status">
      {status === 'succeeded' ? (
        <Check size={14} />
      ) : status === 'failed' ? (
        <CircleAlert size={14} />
      ) : status === 'canceled' ? (
        <X size={14} />
      ) : (
        <LoaderCircle className="spin" size={14} />
      )}
      {label}
    </p>
  );
}
function Failure({ code, id, kind }: { code: string; id: string; kind?: OperationKind | null }) {
  const { t, language } = useI18n();
  return (
    <div className="task-failure">
      <p>{operationErrorMessage(code, language, kind)}</p>
      <details>
        <summary>{t('诊断信息', 'Diagnostics')}</summary>
        <code>
          {code}
          <br />
          {id}
        </code>
      </details>
    </div>
  );
}
function Report({ report }: { report: BackupReport }) {
  const { t } = useI18n();
  return (
    <div className="task-report">
      <p>
        {t(
          `${report.manifest.counts.notes} 篇笔记 · ${report.manifest.counts.sources} 份资料 · ${report.manifest.counts.annotations} 条批注`,
          `${report.manifest.counts.notes} notes · ${report.manifest.counts.sources} sources · ${report.manifest.counts.annotations} annotations`,
        )}
      </p>
      {report.chat ? (
        <p>
          {t(
            `${report.chat.conversations} 个 AI 会话 · ${report.chat.messages} 条消息`,
            `${report.chat.conversations} AI conversations · ${report.chat.messages} messages`,
          )}
        </p>
      ) : report.manifest.database_schema >= 14 ? (
        <p>
          {t(
            '此旧报告未统计 AI 会话，可重新校验备份查看。',
            'This older report has no AI conversation counts. Verify the backup again to see them.',
          )}
        </p>
      ) : null}
      <details>
        <summary>{t('查看报告', 'View report')}</summary>
        <p>
          {t(
            `已校验 ${report.verified_revisions} 个笔记版本、${report.verified_blobs} 个原件与附件文件。`,
            `Verified ${report.verified_revisions} note revisions and ${report.verified_blobs} original and attachment files.`,
          )}
        </p>
        <p>
          {t(
            `${report.manifest.counts.source_versions} 个资料版本 · ${report.manifest.counts.web_captures} 次网页采集`,
            `${report.manifest.counts.source_versions} source versions · ${report.manifest.counts.web_captures} web captures`,
          )}
        </p>
        <p>
          {t(
            `${report.manifest.counts.collections} 个集合 · ${report.manifest.counts.saved_views} 个保存视图 · ${report.manifest.counts.canvases} 个画布 · ${report.manifest.counts.tags} 个标签`,
            `${report.manifest.counts.collections} collections · ${report.manifest.counts.saved_views} saved views · ${report.manifest.counts.canvases} canvases · ${report.manifest.counts.tags} tags`,
          )}
        </p>
      </details>
    </div>
  );
}
