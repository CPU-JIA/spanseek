import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowLeft,
  Check,
  ChevronRight,
  FileSearch,
  FlaskConical,
  History,
  Plus,
  Play,
  MoreHorizontal,
  RefreshCw,
  ShieldCheck,
  Settings2,
  X,
} from 'lucide-react';
import type {
  ApplyChangeItem,
  AuditEvent,
  ChangeItem,
  ChangeSet,
  ChangeSetKind,
  ChangeSetSummary,
  CreateResearchTask,
  ResearchEvidence,
  ResearchSource,
  ResolveLink,
  ResearchStep,
  ResearchTask,
  ResearchTaskSummary,
  ReviewInsightSettings,
  ReviewInsights,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message, openExternalUrl } from '../api';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { PopupMenu } from '../ui/PopupMenu';
import { useAutomationActions } from './useAutomationActions';
import { readChangeSetPage, readResearchPage } from './automation-pages';
import type { AutomationOutcome } from './automation-intents';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { ListPagination, Pagination } from '../layout/Pagination.tsx';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { usePageNavigationPaging } from '../layout/usePageNavigationPaging';
import './automation.css';

type Props = {
  identity: WorkspaceIdentity;
  onOpenNote: (id: string) => void;
  onOpenTarget: (target: ResolveLink) => void;
};

type Tab = 'tidy' | 'research' | 'audit';

const CHANGE_KINDS: { kind: ChangeSetKind; zh: string; en: string }[] = [
  { kind: 'tag_suggestions', zh: '标签建议', en: 'Tag suggestions' },
  { kind: 'duplicate_candidates', zh: '重复候选', en: 'Duplicate candidates' },
  { kind: 'relation_suggestions', zh: '关系建议', en: 'Relation suggestions' },
  { kind: 'stale_sources', zh: '过期来源', en: 'Stale sources' },
];
const ACTION_NAMES = {
  automation_create_change_set: ['创建提案', 'Create proposal'],
  automation_apply_item: ['应用提案条目', 'Apply proposal item'],
  automation_skip_item: ['跳过提案条目', 'Skip proposal item'],
  automation_reject_change_set: ['拒绝提案', 'Reject proposal'],
  research_create: ['创建研究任务', 'Create research task'],
  research_advance: ['执行研究步骤', 'Run research step'],
  research_run_external: ['执行外部研究步骤', 'Run external research step'],
  research_cancel: ['取消研究任务', 'Cancel research task'],
  audit_undo: ['撤销审计操作', 'Undo audited action'],
  review_insight_settings_set: ['保存回顾建议设置', 'Save review insight settings'],
} as const;

function statusLabel(value: string, t: (zh: string, en: string) => string) {
  switch (value) {
    case 'pending':
      return t('待处理', 'Pending');
    case 'applied':
      return t('已处理', 'Applied');
    case 'rejected':
      return t('已拒绝', 'Rejected');
    case 'expired':
      return t('已过期', 'Expired');
    case 'skipped':
      return t('已跳过', 'Skipped');
    case 'running':
      return t('进行中', 'Running');
    case 'succeeded':
      return t('已成功', 'Succeeded');
    case 'completed':
      return t('已完成', 'Completed');
    case 'canceled':
      return t('已取消', 'Canceled');
    case 'failed':
      return t('已失败', 'Failed');
    default:
      return value;
  }
}

type Translate = (zh: string, en: string) => string;

function changeKindLabel(kind: ChangeSetKind, t: Translate) {
  const entry = CHANGE_KINDS.find((item) => item.kind === kind);
  return entry ? t(entry.zh, entry.en) : t('整理提案', 'Tidy proposal');
}

function stopReasonLabel(reason: string, t: Translate) {
  const labels: Record<string, [string, string]> = {
    deadline: ['已到截止时间', 'Deadline reached'],
    budget_steps: ['已到步骤上限', 'Step limit reached'],
    budget_sources: ['已到来源上限', 'Source limit reached'],
    queries_exhausted: ['查询已执行完毕', 'All queries completed'],
    missing_sources: ['缺少可用来源', 'No usable sources'],
  };
  const label = labels[reason];
  return label ? t(...label) : reason;
}

function auditKindLabel(kind: string, t: Translate) {
  const labels: Record<string, [string, string]> = {
    'automation.assign_tag': ['应用标签建议', 'Tag suggestion applied'],
    'automation.trash_duplicate': ['回收重复笔记', 'Duplicate note moved to trash'],
    'automation.append_relation': ['添加知识关系', 'Knowledge relation added'],
    'automation.acknowledge_stale': ['确认来源检查', 'Source check acknowledged'],
    'automation.change_set_rejected': ['拒绝整理提案', 'Change set rejected'],
    'research.created': ['创建研究任务', 'Research task created'],
    'research.completed': ['研究任务结束', 'Research task finished'],
    'research.canceled': ['取消研究任务', 'Research task canceled'],
    'research.external_failed': ['外部检索失败', 'External lookup failed'],
    'review.insight_settings': ['更新回顾设置', 'Review settings updated'],
    'audit.undo': ['撤销先前操作', 'Previous action undone'],
  };
  const label = labels[kind];
  return label ? t(...label) : t('知识库操作记录', 'Workspace activity');
}

function PagingNotice({
  loading,
  error,
  onRetry,
}: {
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}) {
  const { t } = useI18n();
  return error ? (
    <p className="automation-note" role="alert">
      {error}{' '}
      <button disabled={loading} onClick={onRetry}>
        {t('重新读取', 'Reload')}
      </button>
    </p>
  ) : loading ? (
    <p className="automation-note" role="status">
      {t('正在读取…', 'Loading…')}
    </p>
  ) : null;
}

function useRememberedScroll(key: string, ready = true) {
  const [scroll, setScroll] = usePageNavigationState(key + '.scroll', 0);
  const element = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (ready && element.current) element.current.scrollTop = scroll;
  }, [key, ready]);
  return {
    element,
    rememberScroll: () => {
      if (ready && element.current) setScroll(element.current.scrollTop);
    },
  };
}

function useEvidencePaging<T>(key: string, items: readonly T[], visible = true) {
  const [index, setIndex] = usePageNavigationState(key + '.page', 1);
  const [size, setSize] = usePageNavigationState(key + '.size', 10);
  const last = Math.max(1, Math.ceil(items.length / size));
  const page = Math.min(index, last);
  const scrolling = useRememberedScroll(`${key}.${page}.${size}`, visible);
  useEffect(() => {
    if (index !== page) setIndex(page);
  }, [page, index, setIndex]);
  return {
    items: items.slice((page - 1) * size, page * size),
    scrolling,
    pagination:
      items.length > size || page > 1 ? (
        <Pagination
          index={page}
          size={size}
          count={Math.max(0, Math.min(size, items.length - (page - 1) * size))}
          total={items.length}
          canPrevious={page > 1}
          canNext={page < last}
          onPrevious={() => setIndex(page - 1)}
          onNext={() => setIndex(page + 1)}
          onGoTo={setIndex}
          onSize={(value) => {
            setSize(value);
            setIndex(1);
          }}
        />
      ) : null,
  };
}

function SourceCitation({
  source,
  navigationKey,
  onOpenTarget,
  onFailure,
}: {
  source: ResearchSource;
  navigationKey: string;
  onOpenTarget: (target: ResolveLink) => void;
  onFailure: (message: string) => void;
}) {
  const { t } = useI18n();
  const [expanded, setExpanded] = usePageNavigationState(navigationKey, false);
  let href: string | null = null;
  if (source.url) {
    try {
      const url = new URL(source.url);
      if ((url.protocol === 'https:' || url.protocol === 'http:') && !url.username && !url.password)
        href = url.href;
    } catch {
      /* The original URL is retained below; invalid URLs are not opened. */
    }
  }
  return (
    <div className="automation-source" data-source-id={source.object_id}>
      {source.origin === 'private_library' ? (
        <>
          <button
            className="automation-link"
            disabled={!source.revision_id}
            onClick={() =>
              onOpenTarget({
                target_id: source.object_id,
                target_revision_id: source.revision_id,
                target_block_id: null,
              })
            }
          >
            {source.title || t('来源笔记', 'Source note')}
          </button>
          <small>
            {source.revision_id
              ? t('查看取证时的修订', 'Open the captured revision')
              : t(
                  '这条来源未记录修订，无法准确回查。',
                  'This source has no recorded revision and cannot be located exactly.',
                )}
          </small>
        </>
      ) : href ? (
        <a
          className="automation-link"
          href={href}
          rel="noreferrer"
          onClick={(event) => {
            event.preventDefault();
            void openExternalUrl(href!).catch((failure) => onFailure(message(failure)));
          }}
        >
          {source.title || href}
        </a>
      ) : (
        <>
          <strong>{source.title || t('外部来源', 'External source')}</strong>
          <small>{t('没有可用的网页地址。', 'No usable web address was recorded.')}</small>
        </>
      )}
      <details open={expanded} onToggle={(event) => setExpanded(event.currentTarget.open)}>
        <summary>{t('完整摘录与来源信息', 'Full excerpt and source details')}</summary>
        <p>{source.excerpt || t('未记录摘录。', 'No excerpt was recorded.')}</p>
        {source.url && <p>{source.url}</p>}
        <small>
          {t('内容校验值：', 'Content checksum: ')}
          {source.excerpt_hash}
        </small>
      </details>
    </div>
  );
}

function ResearchDetails({
  task,
  onOpenTarget,
  onFailure,
}: {
  task: ResearchTask;
  onOpenTarget: (target: ResolveLink) => void;
  onFailure: (message: string) => void;
}) {
  const { t, language } = useI18n();
  const key = `automation.research.${task.id}`;
  const [section, setSection] = usePageNavigationState<'steps' | 'evidence'>(
    key + '.section',
    'steps',
  );
  const [expanded, setExpanded] = usePageNavigationState<number[]>(key + '.steps.expanded', () =>
    task.steps.length ? [task.steps[0].seq] : [],
  );
  const steps = useEvidencePaging(key + '.steps', task.steps, section === 'steps');
  const evidence = useEvidencePaging<ResearchEvidence>(
    key + '.evidence',
    task.evidence,
    section === 'evidence',
  );
  return (
    <div className="automation-research-content">
      <div
        className="automation-research-sections"
        role="group"
        aria-label={t('研究内容', 'Research content')}
      >
        <button aria-pressed={section === 'steps'} onClick={() => setSection('steps')}>
          {t('执行步骤', 'Steps')} · {task.steps.length}
        </button>
        <button
          aria-pressed={section === 'evidence'}
          aria-label={t(
            `证据快照 · ${task.evidence.length}`,
            `Evidence snapshots · ${task.evidence.length}`,
          )}
          onClick={() => setSection('evidence')}
        >
          {t('证据', 'Evidence')} · {task.evidence.length}
        </button>
        <span
          className="automation-task-state"
          title={`${statusLabel(task.status, t)}${task.stop_reason ? ` · ${stopReasonLabel(task.stop_reason, t)}` : ''} · ${task.step_count}/${task.budget.max_steps}`}
        >
          {statusLabel(task.status, t)}
        </span>
      </div>
      {section === 'steps' ? (
        <>
          <div
            className="automation-detail-scroll"
            ref={steps.scrolling.element}
            onScroll={steps.scrolling.rememberScroll}
          >
            <ol className="automation-steps">
              {steps.items.map((step) => (
                <li key={step.seq} data-step-seq={step.seq}>
                  <details
                    open={expanded.includes(step.seq)}
                    onToggle={(event) => {
                      const open = event.currentTarget.open;
                      setExpanded((old) =>
                        open
                          ? old.includes(step.seq)
                            ? old
                            : [...old, step.seq]
                          : old.filter((value) => value !== step.seq),
                      );
                    }}
                  >
                    <summary>
                      <strong>
                        #{step.seq}{' '}
                        {step.kind === 'private_retrieval'
                          ? t('私库检索', 'Private retrieval')
                          : t('外部检索', 'External search')}
                      </strong>
                      <small>{statusLabel(step.status, t)}</small>
                    </summary>
                    <p>
                      <span>{t('输入', 'Input')}</span>
                      {stepInput(step)}
                    </p>
                    <p>
                      <span>{t('结果', 'Result')}</span>
                      {step.result.kind === 'private_retrieval'
                        ? t(
                            `匹配 ${step.result.matched} 条，保留 ${step.result.returned} 个来源`,
                            `Matched ${step.result.matched}; retained ${step.result.returned} sources`,
                          )
                        : step.result.kind === 'external_search'
                          ? t(
                              `收到 ${step.result.body_bytes} 字节响应（HTTP ${step.result.http_status}）`,
                              `Received ${step.result.body_bytes} bytes (HTTP ${step.result.http_status})`,
                            )
                          : step.result.kind === 'canceled'
                            ? stopReasonLabel(step.result.reason, t)
                            : step.result.kind === 'failed'
                              ? t(
                                  '本步骤失败，详见记录信息。',
                                  'This step failed. See the record details.',
                                )
                              : t('尚无结果。', 'No result yet.')}
                    </p>
                    {step.sources.map((source, index) => (
                      <SourceCitation
                        key={`${source.object_id}/${source.excerpt_hash}/${index}`}
                        source={source}
                        navigationKey={`${key}.step.${step.seq}.source.${index}`}
                        onOpenTarget={onOpenTarget}
                        onFailure={onFailure}
                      />
                    ))}
                    {(step.result.kind === 'failed' || step.result.kind === 'canceled') && (
                      <details className="automation-technical">
                        <summary>{t('记录信息', 'Record details')}</summary>
                        <p>{stepResult(step)}</p>
                      </details>
                    )}
                  </details>
                </li>
              ))}
            </ol>
            {!task.steps.length && (
              <p className="automation-empty">{t('尚未执行步骤。', 'No steps yet.')}</p>
            )}
          </div>
          {steps.pagination}
        </>
      ) : (
        <>
          <div
            className="automation-detail-scroll"
            ref={evidence.scrolling.element}
            onScroll={evidence.scrolling.rememberScroll}
          >
            <ul className="automation-evidence-list">
              {evidence.items.map((item) => (
                <li key={item.id} data-evidence-id={item.id}>
                  <p className="automation-note">
                    {item.revision_state === 'current'
                      ? t('当前修订', 'Current revision')
                      : item.revision_state === 'superseded'
                        ? t(
                            '原文已有更新，保留取证时版本',
                            'The note changed; this preserves the captured revision',
                          )
                        : item.revision_state === 'missing'
                          ? t('修订缺失，摘录仍保留', 'Revision missing; excerpt retained')
                          : t('外部来源', 'External source')}{' '}
                    · {displayDate(item.captured_at, language)}
                  </p>
                  <SourceCitation
                    source={{
                      object_id: item.object_id,
                      revision_id: item.object_revision_id,
                      title: item.title,
                      origin: item.origin,
                      url: item.source_url,
                      excerpt: item.excerpt,
                      excerpt_hash: item.excerpt_hash,
                    }}
                    navigationKey={`${key}.evidence.${item.id}.expanded`}
                    onOpenTarget={onOpenTarget}
                    onFailure={onFailure}
                  />
                </li>
              ))}
            </ul>
            {!task.evidence.length && (
              <p className="automation-empty">
                {t('尚无证据快照。', 'No evidence snapshots yet.')}
              </p>
            )}
          </div>
          {evidence.pagination}
        </>
      )}
    </div>
  );
}

function itemTitle(item: ChangeItem, t: (zh: string, en: string) => string) {
  switch (item.payload.kind) {
    case 'assign_tag':
      return t(`添加标签「${item.payload.tag}」`, `Add tag "${item.payload.tag}"`);
    case 'trash_duplicate':
      return t(
        `重复：${item.payload.duplicate_title}`,
        `Duplicate: ${item.payload.duplicate_title}`,
      );
    case 'append_relation':
      return t(`关联到「${item.payload.target_title}」`, `Link to "${item.payload.target_title}"`);
    case 'stale_source':
      return t(`来源检查：${item.payload.reason}`, `Source check: ${item.payload.reason}`);
    default:
      return item.target_title;
  }
}

function stepInput(step: ResearchStep) {
  return step.input.kind === 'private_retrieval'
    ? `${step.input.query} · ${step.input.limit}`
    : `${step.input.query} · ${step.input.endpoint}`;
}

function stepResult(step: ResearchStep) {
  switch (step.result.kind) {
    case 'private_retrieval':
      return `${step.result.matched} / ${step.result.returned}`;
    case 'external_search':
      return `${step.result.http_status} · ${step.result.body_bytes} B`;
    case 'failed':
      return step.result.code;
    case 'canceled':
      return step.result.reason;
    default:
      return '';
  }
}

export function AutomationPage(props: Props) {
  return (
    <AutomationScope
      key={`${props.identity.local_instance_id}/${props.identity.workspace_id}`}
      {...props}
    />
  );
}

function AutomationScope({ identity, onOpenNote, onOpenTarget }: Props) {
  const { t, language } = useI18n();
  const scope = `${identity.local_instance_id}/${identity.workspace_id}`;
  const id = useId();
  const [tab, setTab] = usePageNavigationState<Tab>('automation.tab', 'tidy');
  const [selectedId, setSelectedId] = usePageNavigationState<string | null>(
    'automation.changeSet',
    null,
  );
  const [taskId, setTaskId] = usePageNavigationState<string | null>(
    'automation.researchTask',
    null,
  );
  const [creatingResearch, setCreatingResearch] = usePageNavigationState(
    'automation.creatingResearch',
    false,
  );
  const [goal, setGoal] = usePageNavigationState('automation.draft.goal', '');
  const [queries, setQueries] = usePageNavigationState('automation.draft.queries', '');
  const [maxSteps, setMaxSteps] = usePageNavigationState('automation.draft.maxSteps', 5);
  const [maxSources, setMaxSources] = usePageNavigationState('automation.draft.maxSources', 10);
  const [external, setExternal] = usePageNavigationState('automation.draft.external', false);
  const [endpoint, setEndpoint] = usePageNavigationState('automation.draft.endpoint', '');
  const [auditOptions, setAuditOptions] = usePageNavigationState('automation.auditOptions', false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const alive = useRef(true);
  const actionRunning = useRef(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [selected, setSelected] = useState<ChangeSet | null>(null);
  const [task, setTask] = useState<ResearchTask | null>(null);
  const [stale, setStale] = useState<{
    checked: number;
    findings: number;
    empty: string | null;
  } | null>(null);
  const [settings, setSettings] = useState<ReviewInsightSettings | null>(null);
  const [insights, setInsights] = useState<ReviewInsights | null>(null);
  const setsPaging = usePageNavigationPaging<ChangeSetSummary>(
    `${scope}/sets`,
    'automation.sets',
    (cursor, size) => readChangeSetPage(identity, cursor, size),
    tab === 'tidy',
    tab === 'tidy' && !selectedId,
  );
  const tasksPaging = usePageNavigationPaging<ResearchTaskSummary>(
    `${scope}/tasks`,
    'automation.tasks',
    (cursor, size) => readResearchPage(identity, cursor, size),
    tab === 'research',
    tab === 'research' && !taskId && !creatingResearch,
  );
  const auditPaging = usePageNavigationPaging<AuditEvent>(
    `${scope}/audit`,
    'automation.audit',
    async (cursor, size) => {
      const before = cursor === null ? null : Number(cursor);
      if (before !== null && (!Number.isSafeInteger(before) || before < 1))
        throw new Error(
          t(
            '审计页码超出可安全读取的范围。',
            'The audit cursor is outside the safe integer range.',
          ),
        );
      const page = await call(
        {
          kind: 'audit_page',
          payload: {
            limit: size,
            before_seq: before,
            subject_kind: null,
            subject_id: null,
          },
        },
        'audit_page',
        identity,
      );
      if (page.next_before_seq !== null && !Number.isSafeInteger(Number(page.next_before_seq)))
        throw new Error(
          t(
            '审计响应超出可安全读取的范围。',
            'The audit response is outside the safe integer range.',
          ),
        );
      return {
        items: page.items,
        next_cursor: page.next_before_seq === null ? null : String(page.next_before_seq),
      };
    },
    tab === 'audit',
  );
  const sets = setsPaging.pager.page?.items ?? [];
  const tasks = tasksPaging.pager.page?.items ?? [];
  const audit = auditPaging.pager.page?.items ?? [];
  function consumeOutcome({ intent, result }: AutomationOutcome) {
    const command = intent.command;
    setError(null);
    if ('change' in result) {
      setTab('tidy');
      setSelected(result.change);
      setSelectedId(result.change.id);
      void setsPaging.pager.refresh();
    } else if ('task' in result) {
      setTab('research');
      setTask(result.task);
      setTaskId(result.task.id);
      setCreatingResearch(false);
      if (command.kind === 'research_create') {
        const original = command.payload.request;
        // A reply must not erase a later draft edited while the result was unknown.
        if (JSON.stringify(researchRequest()) === JSON.stringify(original)) {
          setGoal('');
          setQueries('');
        }
      }
      void tasksPaging.pager.refresh();
    } else if ('settings' in result) {
      setSettings(result.settings);
      void loadInsights().catch((failure) => {
        if (alive.current) setError(message(failure));
      });
    } else {
      void auditPaging.pager.refresh();
    }
  }
  const actions = useAutomationActions(
    identity,
    JSON.stringify([tab, selectedId, taskId, creatingResearch, auditOptions]),
    consumeOutcome,
  );
  const mutationLocked = busy || actions.locked;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const [decision, setDecision] = useState<
    | { kind: 'apply'; item: ChangeItem }
    | { kind: 'cancel-task' }
    | { kind: 'undo'; event: AuditEvent }
    | null
  >(null);

  const run = useCallback(async (action: () => Promise<void>) => {
    if (actionRunning.current) return;
    actionRunning.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (failure) {
      if (alive.current) setError(message(failure));
    } finally {
      actionRunning.current = false;
      if (alive.current) setBusy(false);
    }
  }, []);

  async function loadSets() {
    await setsPaging.pager.refresh();
    if (selectedId)
      setSelected(
        await call(
          { kind: 'automation_change_set_get', payload: { change_id: selectedId } },
          'change_set',
          identity,
        ),
      );
  }
  async function loadTasks() {
    await tasksPaging.pager.refresh();
    if (taskId)
      setTask(
        await call(
          { kind: 'research_get', payload: { task_id: taskId } },
          'research_task',
          identity,
        ),
      );
  }
  async function loadInsights() {
    const [current, next] = await Promise.all([
      call({ kind: 'review_insight_settings_get' }, 'review_insight_settings', identity),
      call({ kind: 'review_insights_get', payload: { limit: 12 } }, 'review_insights', identity),
    ]);
    if (alive.current) {
      setSettings(current);
      setInsights(next);
    }
  }
  async function loadAudit() {
    await Promise.all([auditPaging.pager.refresh(), loadInsights()]);
  }

  useEffect(() => {
    void call({ kind: 'automation_expire' }, 'done', identity)
      .then(() => setsPaging.pager.refresh())
      .catch((failure) => {
        if (alive.current) setError(message(failure));
      });
  }, [scope]);
  useEffect(() => {
    // Reads remain available when the user changes sections during a mutation.
    if (tab === 'audit')
      void loadInsights().catch((failure) => {
        if (alive.current) setError(message(failure));
      });
  }, [tab, scope]);
  useEffect(() => {
    let live = true;
    if (tab !== 'tidy') return;
    if (!selectedId) {
      setSelected(null);
      setDetailLoading(false);
      return;
    }
    setDetailLoading(true);
    setSelected(null);
    void call(
      { kind: 'automation_change_set_get', payload: { change_id: selectedId } },
      'change_set',
      identity,
    )
      .then((next) => {
        if (live) {
          setSelected(next);
          setError(null);
        }
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      })
      .finally(() => {
        if (live) setDetailLoading(false);
      });
    return () => {
      live = false;
    };
  }, [tab, selectedId, scope]);
  useEffect(() => {
    let live = true;
    if (tab !== 'research') return;
    if (!taskId || creatingResearch) {
      setTask(null);
      setDetailLoading(false);
      return;
    }
    setDetailLoading(true);
    setTask(null);
    void call({ kind: 'research_get', payload: { task_id: taskId } }, 'research_task', identity)
      .then((next) => {
        if (live) {
          setTask(next);
          setError(null);
        }
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      })
      .finally(() => {
        if (live) setDetailLoading(false);
      });
    return () => {
      live = false;
    };
  }, [tab, taskId, creatingResearch, scope]);
  async function createSet(kind: ChangeSetKind) {
    await run(async () => {
      await actions.submit({
        kind: 'automation_create_change_set',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            kind,
            collection_id: null,
            limit: 20,
            stale_after_days: kind === 'stale_sources' ? 365 : null,
          },
        },
      });
    });
  }

  function openSet(id: string) {
    setSelectedId(id);
  }

  async function applyItem(item: ChangeItem) {
    if (!selected) return;
    if (item.requires_confirmation) {
      setDecision({ kind: 'apply', item });
      return;
    }
    await applyItemNow(item);
  }

  async function applyItemNow(item: ChangeItem) {
    if (!selected) return;
    await run(async () => {
      await actions.submit({
        kind: 'automation_apply_item',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            change_id: selected.id,
            item_id: item.item_id,
            confirm: item.requires_confirmation,
          } satisfies ApplyChangeItem,
        },
      });
    });
  }

  async function skipItem(item: ChangeItem) {
    if (!selected) return;
    await run(async () => {
      await actions.submit({
        kind: 'automation_skip_item',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            change_id: selected.id,
            item_id: item.item_id,
            confirm: false,
          } satisfies ApplyChangeItem,
        },
      });
    });
  }

  async function rejectSet() {
    if (!selected) return;
    await run(async () => {
      await actions.submit({
        kind: 'automation_reject_change_set',
        payload: {
          command_id: crypto.randomUUID(),
          request: { change_id: selected.id },
        },
      });
    });
  }

  async function staleCheck() {
    await run(async () => {
      const report = await call(
        { kind: 'automation_stale_report', payload: { window_days: 365, limit: 20 } },
        'stale_source_report',
        identity,
      );
      setStale({
        checked: report.checked,
        findings: report.findings.length,
        empty: report.empty_reason,
      });
    });
  }

  function researchRequest(): CreateResearchTask {
    return {
      goal: goal.trim(),
      collection_id: null,
      queries: queries
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean),
      budget: { max_steps: maxSteps, max_sources: maxSources },
      stop: { deadline_seconds: null, missing_after_step: null },
      external_allowed: external,
      external_search_endpoint: external ? endpoint.trim() : null,
    };
  }
  async function createTask() {
    await run(async () => {
      const request = researchRequest();
      await actions.submit({
        kind: 'research_create',
        payload: { command_id: crypto.randomUUID(), request },
      });
    });
  }

  function openTask(id: string) {
    setCreatingResearch(false);
    setTaskId(id);
  }

  async function advance() {
    if (!task) return;
    await run(async () => {
      await actions.submit({
        kind: 'research_advance',
        payload: {
          command_id: crypto.randomUUID(),
          request: { task_id: task.id, base_row_version: task.row_version },
        },
      });
    });
  }

  async function runExternal() {
    if (!task) return;
    await run(async () => {
      await actions.submit({
        kind: 'research_run_external',
        payload: {
          command_id: crypto.randomUUID(),
          task_id: task.id,
          base_row_version: task.row_version,
        },
      });
    });
  }

  function cancel() {
    if (!task) return;
    setDecision({ kind: 'cancel-task' });
  }

  async function cancelTask(reason: string) {
    if (!task) return;
    await run(async () => {
      await actions.submit({
        kind: 'research_cancel',
        payload: {
          command_id: crypto.randomUUID(),
          request: { task_id: task.id, base_row_version: task.row_version, reason },
        },
      });
    });
  }

  function undo(event: AuditEvent) {
    setDecision({ kind: 'undo', event });
  }

  async function undoEvent(event: AuditEvent) {
    await run(async () => {
      await actions.submit({
        kind: 'audit_undo',
        payload: {
          command_id: crypto.randomUUID(),
          request: { audit_id: event.id, confirm: true },
        },
      });
    });
  }

  async function saveSettings(next: Partial<ReviewInsightSettings>) {
    if (!settings) return;
    await run(async () => {
      await actions.submit({
        kind: 'review_insight_settings_set',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            base_row_version: settings.row_version,
            enabled: next.enabled ?? settings.enabled,
            window_days: next.window_days ?? settings.window_days,
            include_completed: next.include_completed ?? settings.include_completed,
            include_stale: next.include_stale ?? settings.include_stale,
          },
        },
      });
    });
  }

  const pendingItems = useMemo(
    () => selected?.items.filter((item) => item.status === 'pending') ?? [],
    [selected],
  );

  const sectionTitle =
    tab === 'tidy'
      ? t('整理提案', 'Tidy proposals')
      : tab === 'research'
        ? t('研究任务', 'Research tasks')
        : t('审计与回顾', 'Audit & review');
  const detailScroll = useRememberedScroll(
    `automation.detail.${tab}.${selectedId ?? taskId ?? 'none'}`,
    !detailLoading,
  );
  return (
    <PageFrame
      className="automation-page"
      title={
        tab === 'research' && creatingResearch
          ? t('新建研究任务', 'New research task')
          : tab === 'research' && task
            ? task.goal
            : tab === 'tidy' && selected
              ? changeKindLabel(selected.kind, t)
              : tab === 'audit' && auditOptions
                ? t('可解释回顾', 'Review insights')
                : sectionTitle
      }
      description={t(
        '逐项审阅提案，回查每一步来源。',
        'Review each proposal and trace every source.',
      )}
      actions={
        <>
          {((tab === 'tidy' && selectedId) ||
            (tab === 'research' && (taskId || creatingResearch)) ||
            (tab === 'audit' && auditOptions)) && (
            <button
              className="icon-button"
              disabled={busy}
              aria-label={
                tab === 'tidy'
                  ? t('返回变更集', 'Back to change sets')
                  : tab === 'audit'
                    ? t('返回审计记录', 'Back to audit log')
                    : t('返回任务列表', 'Back to tasks')
              }
              onClick={() => {
                if (tab === 'tidy') setSelectedId(null);
                else if (tab === 'audit') setAuditOptions(false);
                else if (creatingResearch) setCreatingResearch(false);
                else setTaskId(null);
              }}
            >
              <ArrowLeft size={17} />
            </button>
          )}
          {tab === 'tidy' && selected?.status === 'pending' && (
            <button
              className="text-button"
              disabled={mutationLocked}
              aria-label={t('拒绝整组', 'Reject set')}
              onClick={() => void rejectSet()}
            >
              {t('拒绝', 'Reject')}
            </button>
          )}
          {tab === 'tidy' && !selectedId && (
            <PopupMenu
              label={t('创建提案', 'Create a proposal')}
              trigger={<Plus size={17} />}
              disabled={busy}
              placement="bottom-start"
              items={[
                ...CHANGE_KINDS.map((entry) => ({
                  id: entry.kind,
                  label: t(entry.zh, entry.en),
                  disabled: mutationLocked,
                  onSelect: () => void createSet(entry.kind),
                })),
                {
                  id: 'stale-report',
                  label: t('只读检查过期来源', 'Read-only stale check'),
                  separatorBefore: true,
                  onSelect: () => void staleCheck(),
                },
              ]}
            />
          )}
          {tab === 'research' && !taskId && !creatingResearch && (
            <button
              className="icon-button"
              disabled={busy}
              aria-label={t('新建研究任务', 'New research task')}
              onClick={() => setCreatingResearch(true)}
            >
              <Plus size={17} />
            </button>
          )}
          {tab === 'research' && task && !creatingResearch && (
            <>
              {['pending', 'running'].includes(task.status) && (
                <button
                  className="icon-button"
                  disabled={mutationLocked}
                  aria-label={t('执行一步', 'Run one step')}
                  onClick={() => void advance()}
                >
                  <Play size={17} />
                </button>
              )}
              <PopupMenu
                label={t('研究任务操作', 'Research task actions')}
                trigger={<MoreHorizontal size={17} />}
                disabled={busy}
                placement="bottom-start"
                items={[
                  {
                    id: 'reload',
                    label: t('重新读取任务', 'Reload task'),
                    onSelect: () => void run(loadTasks),
                  },
                  ...(task.external_allowed
                    ? [
                        {
                          id: 'external',
                          label: t('执行外部一步', 'Run external step'),
                          disabled: mutationLocked || !['pending', 'running'].includes(task.status),
                          onSelect: () => void runExternal(),
                        },
                      ]
                    : []),
                  {
                    id: 'cancel',
                    label: t('取消研究任务', 'Cancel research task'),
                    disabled: mutationLocked || !['pending', 'running'].includes(task.status),
                    onSelect: cancel,
                  },
                ]}
              />
            </>
          )}
          {tab === 'audit' && !auditOptions && (
            <button
              className="icon-button"
              disabled={busy}
              aria-label={t('回顾建议设置', 'Review insight settings')}
              onClick={() => setAuditOptions(true)}
            >
              <Settings2 size={17} />
            </button>
          )}
          {!(tab === 'research' && (creatingResearch || task)) && (
            <button
              className="icon-button"
              disabled={busy || detailLoading}
              aria-label={
                tab === 'tidy'
                  ? selectedId
                    ? t('重新读取变更集', 'Reload change set')
                    : t('刷新变更集', 'Refresh change sets')
                  : tab === 'research'
                    ? taskId
                      ? t('重新读取任务', 'Reload task')
                      : t('刷新研究任务', 'Refresh research tasks')
                    : t('刷新审计记录', 'Refresh audit log')
              }
              onClick={() =>
                void run(tab === 'tidy' ? loadSets : tab === 'research' ? loadTasks : loadAudit)
              }
            >
              <RefreshCw size={17} />
            </button>
          )}
        </>
      }
      toolbar={
        <div
          className="automation-tabs"
          role="tablist"
          aria-label={t('自动化区域', 'Automation sections')}
          onKeyDown={(event) => {
            const tabs: Tab[] = ['tidy', 'research', 'audit'];
            const index = tabs.indexOf(tab);
            const next =
              event.key === 'ArrowRight'
                ? tabs[(index + 1) % tabs.length]
                : event.key === 'ArrowLeft'
                  ? tabs[(index + tabs.length - 1) % tabs.length]
                  : event.key === 'Home'
                    ? tabs[0]
                    : event.key === 'End'
                      ? tabs.at(-1)
                      : undefined;
            if (next) {
              event.preventDefault();
              setTab(next);
              document.getElementById(`${id}-${next}-tab`)?.focus();
            }
          }}
        >
          {(['tidy', 'research', 'audit'] as const).map((value) => (
            <button
              key={value}
              id={`${id}-${value}-tab`}
              type="button"
              role="tab"
              aria-selected={tab === value}
              aria-controls={`${id}-${value}-panel`}
              tabIndex={tab === value ? 0 : -1}
              className={tab === value ? 'active' : ''}
              onClick={() => setTab(value)}
            >
              {value === 'tidy' ? (
                <FileSearch size={15} />
              ) : value === 'research' ? (
                <FlaskConical size={15} />
              ) : (
                <History size={15} />
              )}
              {value === 'tidy'
                ? t('整理提案', 'Tidy proposals')
                : value === 'research'
                  ? t('研究任务', 'Research tasks')
                  : t('审计与回顾', 'Audit & review')}
            </button>
          ))}
        </div>
      }
    >
      {(actions.pending || actions.error || !actions.ready) && (
        <div className="automation-request" role={actions.error ? 'alert' : 'status'}>
          <div>
            <strong>
              {actions.pending
                ? t('确认原请求', 'Confirm original request')
                : t('自动化请求记录', 'Automation request record')}
            </strong>
            {actions.pending && (
              <small>
                {t(
                  ACTION_NAMES[actions.pending.command.kind][0],
                  ACTION_NAMES[actions.pending.command.kind][1],
                )}
              </small>
            )}
            <p>
              {actions.pending
                ? t(
                    '已保留原操作。确认结果后再提交其他变更。',
                    'The original action is retained. Confirm its result before making another change.',
                  )
                : actions.ready
                  ? t(
                      '操作未完成。请检查错误，重读当前内容后再试。',
                      'The action did not finish. Check the error and reload the current content before retrying.',
                    )
                  : t('正在读取请求记录…', 'Reading request record…')}
            </p>
            {actions.error && <p>{actions.error}</p>}
          </div>
          {actions.pending ? (
            <button
              className="secondary-button"
              disabled={actions.busy || busy}
              onClick={() => void actions.submit()}
            >
              {actions.busy
                ? t('正在确认…', 'Confirming…')
                : t('确认原请求', 'Confirm original request')}
            </button>
          ) : !actions.ready && actions.error ? (
            <button className="secondary-button" onClick={actions.reload}>
              {t('重新读取请求记录', 'Reload request record')}
            </button>
          ) : null}
        </div>
      )}
      {error && (
        <div className="automation-error" role="alert">
          <ShieldCheck size={16} />
          <span>{error}</span>
          <button aria-label={t('关闭', 'Dismiss')} onClick={() => setError(null)}>
            <X size={14} />
          </button>
        </div>
      )}
      <div
        role="tabpanel"
        id={`${id}-${tab}-panel`}
        aria-labelledby={`${id}-${tab}-tab`}
        className="automation-region"
      >
        {tab === 'tidy' && !selectedId && (
          <section
            className="automation-panel automation-directory"
            aria-label={t('变更集列表', 'Change set list')}
          >
            <div
              className="automation-list-scroll"
              ref={setsPaging.element}
              onScroll={setsPaging.rememberScroll}
            >
              {stale && (
                <p className="automation-note">
                  {t(
                    `已检查 ${stale.checked} 个来源，发现 ${stale.findings} 条结论`,
                    `Checked ${stale.checked} sources, ${stale.findings} findings`,
                  )}
                  {stale.empty ? ` · ${stale.empty}` : ''}
                </p>
              )}
              <PagingNotice
                loading={setsPaging.pager.loading}
                error={setsPaging.pager.error}
                onRetry={() => void run(loadSets)}
              />
              <ul className="automation-list">
                {sets.map((row) => (
                  <li key={row.id} data-change-id={row.id}>
                    <button disabled={busy} onClick={() => openSet(row.id)}>
                      <span>
                        {changeKindLabel(row.kind, t)}
                        <small>
                          {statusLabel(row.status, t)} · {row.applied_count}/{row.item_count} ·{' '}
                          {displayDate(row.created_at, language)}
                        </small>
                      </span>
                      <ChevronRight size={15} />
                    </button>
                  </li>
                ))}
              </ul>
              {setsPaging.ready && !setsPaging.pager.error && sets.length === 0 && (
                <p className="automation-empty">
                  {t(
                    '还没有提案。从上方创建，再逐项审阅。',
                    'No proposals yet. Create one above, then review its items.',
                  )}
                </p>
              )}
            </div>
            <ListPagination list={setsPaging.pager} disabled={busy} />
          </section>
        )}
        {tab === 'tidy' && selectedId && (
          <section
            className="automation-panel automation-detail"
            aria-label={t('变更集详情', 'Change set detail')}
          >
            {detailLoading && <p role="status">{t('正在读取提案…', 'Loading proposal…')}</p>}
            {selected && (
              <>
                <div
                  className="automation-detail-scroll"
                  ref={detailScroll.element}
                  onScroll={detailScroll.rememberScroll}
                >
                  {pendingItems.length === 0 && (
                    <p className="muted">
                      {t('这一组没有待处理项。', 'No pending items in this set.')}
                    </p>
                  )}
                  <ul className="automation-items">
                    {selected.items.map((item) => (
                      <li key={item.item_id} className={item.status}>
                        <div className="automation-item-body">
                          <button
                            className="automation-link"
                            disabled={busy}
                            onClick={() => onOpenNote(item.target_object_id)}
                          >
                            {item.target_title}
                          </button>
                          <strong>{itemTitle(item, t)}</strong>
                          {item.blocked_reason === 'tag_changed' && (
                            <p className="muted">
                              {t(
                                '标签已改名或删除，这条旧建议已跳过。可重新生成标签建议。',
                                'The tag changed or was removed. This old suggestion was skipped; generate fresh tag suggestions to continue.',
                              )}
                            </p>
                          )}
                          {item.evidence.map((evidence, index) => (
                            <p key={`${item.item_id}-${index}`} className="automation-evidence">
                              <span>{evidence.field}</span>
                              {evidence.excerpt}
                            </p>
                          ))}
                        </div>
                        <div className="automation-item-actions">
                          <em>{statusLabel(item.status, t)}</em>
                          {item.status === 'pending' && (
                            <>
                              <button
                                disabled={mutationLocked}
                                onClick={() => void applyItem(item)}
                              >
                                <Check size={14} />
                                {t('应用', 'Apply')}
                              </button>
                              <button disabled={mutationLocked} onClick={() => void skipItem(item)}>
                                {t('跳过', 'Skip')}
                              </button>
                            </>
                          )}
                        </div>
                      </li>
                    ))}
                  </ul>
                </div>
              </>
            )}
          </section>
        )}
        {tab === 'research' && !taskId && !creatingResearch && (
          <section
            className="automation-panel automation-directory"
            aria-label={t('研究任务列表', 'Research task list')}
          >
            <div
              className="automation-list-scroll"
              ref={tasksPaging.element}
              onScroll={tasksPaging.rememberScroll}
            >
              <PagingNotice
                loading={tasksPaging.pager.loading}
                error={tasksPaging.pager.error}
                onRetry={() => void run(loadTasks)}
              />
              <ul className="automation-list">
                {tasks.map((row) => (
                  <li key={row.id} data-task-id={row.id}>
                    <button disabled={busy} onClick={() => openTask(row.id)}>
                      <span>
                        {row.goal}
                        <small>
                          {statusLabel(row.status, t)} · {row.step_count} ·{' '}
                          {displayDate(row.created_at, language)}
                        </small>
                      </span>
                      <ChevronRight size={15} />
                    </button>
                  </li>
                ))}
              </ul>
              {tasksPaging.ready && !tasksPaging.pager.error && tasks.length === 0 && (
                <p className="automation-empty">
                  {t(
                    '还没有研究任务。新建任务后可按步骤执行和回查。',
                    'No research tasks yet. Create one to work step by step and revisit its sources.',
                  )}
                </p>
              )}
            </div>
            <ListPagination list={tasksPaging.pager} disabled={busy} />
          </section>
        )}
        {tab === 'research' && creatingResearch && (
          <section
            className="automation-panel automation-detail"
            aria-label={t('新建研究任务', 'New research task')}
          >
            <div className="automation-form-scroll">
              <label>
                {t('目标', 'Goal')}
                <input value={goal} onChange={(e) => setGoal(e.target.value)} maxLength={2000} />
              </label>
              <label>
                {t('范围查询（每行一条，按顺序执行）', 'Scoped queries (one per line, in order)')}
                <textarea value={queries} onChange={(e) => setQueries(e.target.value)} rows={4} />
              </label>
              <div className="automation-inline">
                <label>
                  {t('最多步骤', 'Max steps')}
                  <input
                    disabled={busy}
                    type="number"
                    min={1}
                    max={50}
                    value={maxSteps}
                    onChange={(e) => setMaxSteps(Number(e.target.value))}
                  />
                </label>
                <label>
                  {t('最多来源', 'Max sources')}
                  <input
                    disabled={busy}
                    type="number"
                    min={1}
                    max={100}
                    value={maxSources}
                    onChange={(e) => setMaxSources(Number(e.target.value))}
                  />
                </label>
              </div>
              <label className="automation-check">
                <input
                  disabled={busy}
                  type="checkbox"
                  checked={external}
                  onChange={(e) => setExternal(e.target.checked)}
                />
                {t(
                  '允许单次外部检索（只发送声明的查询）',
                  'Allow one external lookup (declared query only)',
                )}
              </label>
              {external && (
                <label>
                  {t(
                    '外部检索地址模板（必须包含 {query}）',
                    'External endpoint template (must contain {query})',
                  )}
                  <input
                    disabled={busy}
                    value={endpoint}
                    onChange={(e) => setEndpoint(e.target.value)}
                    placeholder="https://example.com/search?q={query}"
                  />
                </label>
              )}
            </div>
            <div className="automation-form-footer">
              <button
                className="primary-button"
                disabled={
                  mutationLocked ||
                  !goal.trim() ||
                  !queries.trim() ||
                  !Number.isInteger(maxSteps) ||
                  maxSteps < 1 ||
                  maxSteps > 50 ||
                  !Number.isInteger(maxSources) ||
                  maxSources < 1 ||
                  maxSources > 100 ||
                  (external && !endpoint.includes('{query}'))
                }
                onClick={() => void createTask()}
              >
                <Plus size={14} />
                {t('创建任务', 'Create task')}
              </button>
            </div>
          </section>
        )}
        {tab === 'research' && taskId && !creatingResearch && (
          <section
            className="automation-panel automation-detail"
            aria-label={t('研究任务详情', 'Research task detail')}
          >
            {detailLoading && <p role="status">{t('正在读取任务…', 'Loading task…')}</p>}
            {task && (
              <>
                <ResearchDetails task={task} onOpenTarget={onOpenTarget} onFailure={setError} />
              </>
            )}
          </section>
        )}
        {tab === 'audit' && (
          <section
            className="automation-panel automation-directory"
            aria-label={t('审计记录', 'Audit log')}
          >
            <div className="automation-insight-options" hidden={!auditOptions}>
              <div className="automation-settings">
                <p className="automation-note">
                  {t(
                    '建议仅依据你启用的使用记录；它们不能代替你的判断。',
                    'Suggestions use the activity records you enabled; they do not replace your judgement.',
                  )}
                </p>
                {settings && (
                  <>
                    <label className="automation-check">
                      <input
                        type="checkbox"
                        disabled={mutationLocked}
                        checked={settings.enabled}
                        onChange={(e) => void saveSettings({ enabled: e.target.checked })}
                      />
                      {t(
                        '启用回顾建议（基于自选使用记录）',
                        'Enable review suggestions (selected usage records)',
                      )}
                    </label>
                    <label>
                      {t('观察窗口（天）', 'Window (days)')}
                      <input
                        type="number"
                        min={1}
                        max={365}
                        disabled={mutationLocked}
                        value={settings.window_days}
                        onChange={(e) => void saveSettings({ window_days: Number(e.target.value) })}
                      />
                    </label>
                    <label className="automation-check">
                      <input
                        type="checkbox"
                        disabled={mutationLocked}
                        checked={settings.include_completed}
                        onChange={(e) => void saveSettings({ include_completed: e.target.checked })}
                      />
                      {t('包含已完成回顾', 'Include completed reviews')}
                    </label>
                    <label className="automation-check">
                      <input
                        type="checkbox"
                        disabled={mutationLocked}
                        checked={settings.include_stale}
                        onChange={(e) => void saveSettings({ include_stale: e.target.checked })}
                      />
                      {t('包含从未安排回顾的笔记', 'Include never-scheduled notes')}
                    </label>
                  </>
                )}
                {insights?.enabled === false && (
                  <p className="muted">{t('回顾建议已关闭。', 'Review suggestions are off.')}</p>
                )}
                <ul className="automation-insights">
                  {insights?.items.map((insight) => (
                    <li key={`${insight.object_id}-${insight.kind}`}>
                      <button
                        className="automation-link"
                        onClick={() => onOpenNote(insight.object_id)}
                      >
                        {insight.title}
                      </button>
                      <span>{insight.reason}</span>
                      <small>{insight.caveat}</small>
                    </li>
                  ))}
                </ul>
                {insights?.enabled && !insights.items.length && (
                  <p className="muted">
                    {t('当前没有可解释的回顾建议。', 'No explainable suggestions right now.')}
                  </p>
                )}
              </div>
            </div>
            {!auditOptions && (
              <>
                <div
                  className="automation-list-scroll"
                  ref={auditPaging.element}
                  onScroll={auditPaging.rememberScroll}
                >
                  <PagingNotice
                    loading={auditPaging.pager.loading}
                    error={auditPaging.pager.error}
                    onRetry={() => void run(loadAudit)}
                  />
                  <ul className="automation-audit">
                    {audit.map((event) => (
                      <li
                        key={event.id}
                        data-audit-id={event.id}
                        className={event.undone ? 'undone' : ''}
                      >
                        <div>
                          <strong>{auditKindLabel(event.kind, t)}</strong>
                          <span className="automation-record-summary">{event.summary}</span>
                          <small>
                            {event.origin === 'automation'
                              ? t('自动化', 'Automation')
                              : event.origin === 'user'
                                ? t('用户操作', 'User action')
                                : t('其他来源', 'Other origin')}{' '}
                            · {displayDate(event.occurred_at, language)}
                            {event.undone ? ` · ${t('已撤销', 'Undone')}` : ''}
                          </small>
                          <details className="automation-technical">
                            <summary>{t('记录信息', 'Record details')}</summary>
                            <p>
                              {event.kind} · {event.origin}
                            </p>
                          </details>
                        </div>
                        {event.undoable && !event.undone && (
                          <button disabled={mutationLocked} onClick={() => undo(event)}>
                            {t('撤销', 'Undo')}
                          </button>
                        )}
                      </li>
                    ))}
                  </ul>
                  {auditPaging.ready && !auditPaging.pager.error && audit.length === 0 && (
                    <p className="automation-empty">
                      {t('还没有审计记录。', 'No audit events yet.')}
                    </p>
                  )}
                </div>
                <ListPagination list={auditPaging.pager} disabled={busy} />
              </>
            )}
          </section>
        )}
      </div>
      {decision?.kind === 'apply' && (
        <ActionConfirmDialog
          title={t('应用这条自动化提案？', 'Apply this automation item?')}
          description={t(
            '此操作会写入或移动知识。每个条目仍需单独确认，写入记录可在审计页撤销。',
            'This writes or moves knowledge. Each item is confirmed separately, and every write can be undone from the audit log.',
          )}
          confirmLabel={t('确认应用', 'Apply item')}
          onCancel={() => setDecision(null)}
          onConfirm={() => {
            const item = decision.item;
            setDecision(null);
            void applyItemNow(item);
          }}
        />
      )}
      {decision?.kind === 'cancel-task' && (
        <ActionConfirmDialog
          title={t('取消研究任务？', 'Cancel the research task?')}
          description={t(
            '任务会停止推进；已产生的步骤、证据和审计记录都会保留。',
            'The task stops progressing; completed steps, evidence, and audit records remain.',
          )}
          confirmLabel={t('确认取消任务', 'Cancel task')}
          tone="danger"
          input={{
            label: t('取消原因（可留空）', 'Cancel reason (optional)'),
            placeholder: t(
              '例如：换个问题重新开始',
              'For example: start over with a different question',
            ),
          }}
          onCancel={() => setDecision(null)}
          onConfirm={(reason) => {
            setDecision(null);
            void cancelTask(reason);
          }}
        />
      )}
      {decision?.kind === 'undo' && (
        <ActionConfirmDialog
          title={t('撤销这条自动化写入？', 'Undo this automation write?')}
          description={t(
            '会创建一条补偿变更并写入审计；历史记录不会被删除。',
            'This creates a compensating change and records it in the audit log; history is not deleted.',
          )}
          confirmLabel={t('确认撤销', 'Undo write')}
          tone="danger"
          onCancel={() => setDecision(null)}
          onConfirm={() => {
            const event = decision.event;
            setDecision(null);
            void undoEvent(event);
          }}
        />
      )}{' '}
    </PageFrame>
  );
}
