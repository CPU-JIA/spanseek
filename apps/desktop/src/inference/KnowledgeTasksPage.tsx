import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ArrowLeft, BookOpen, History, Plus, RefreshCw, Sparkles, X } from 'lucide-react';
import type {
  GenerationSettings,
  KnowledgeTask,
  KnowledgeTaskOutputsPage,
  KnowledgeTasksPage as TaskHistoryPage,
  KnowledgeTaskTemplate,
  SearchEvidence,
  SearchScope,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { Pagination } from '../layout/Pagination.tsx';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { ChatMaterials } from './ChatMaterials';
import { ChannelModelPicker } from './ChannelModelPicker';
import { ChatMarkdown } from './ChatMarkdown';
import { ChatProposalPanel } from './ChatProposalPanel';
import { chatActive } from './chat-stream';
import { emptyScope, type OpenEvidence } from './SearchPage';
import { useInferenceAction } from './useInferenceAction';
import { inferenceErrorMessage } from './inference-errors';
import { TaskTemplatesDialog } from './TaskTemplatesDialog';
import {
  emptyTaskDraft,
  parseTaskDraft,
  taskStatus,
  materialCoverageText,
  type TaskDraft,
} from './knowledge-task-state';
import { IncomingTaskMaterials } from './IncomingTaskMaterials';
import type { TaskMaterialRequest } from './knowledge-task-navigation';
import { useTaskDraft } from './useTaskDraft';
import './knowledge-tasks.css';
import './chat.css';

type Props = {
  identity: WorkspaceIdentity;
  settingsRevision?: number;
  materialRequest?: TaskMaterialRequest | null;
  onMaterialsHandled?: (id: string) => void;
  initialScope?: SearchScope;
  onOpenModels: () => void;
  onOpenEvidence: OpenEvidence;
  onOpenProposal: (id: string) => void;
  onOpenNote: (id: string) => Promise<boolean | void>;
};
const draftKey = (identity: WorkspaceIdentity) =>
  `spanseek/knowledge-task-draft/v1/${identity.local_instance_id}/${identity.workspace_id}`;

export function KnowledgeTasksPage(props: Props) {
  const { identity } = props;
  const { t } = useI18n();
  const [location, setLocation] = usePageNavigationState<{
    mode: 'compose' | 'history' | 'task';
    id: string | null;
  }>('knowledge-task.location', { mode: 'compose', id: null });
  const draft = useTaskDraft(draftKey(identity), emptyTaskDraft, parseTaskDraft);
  const [notice, setNotice] = useState('');
  const [reusePending, setReusePending] = useState<KnowledgeTask | null>(null);
  function reuse(task: KnowledgeTask) {
    const next: TaskDraft = {
      title: task.input.title,
      instructions: task.input.instructions,
      materials: task.input.materials,
      template: task.template_current ? task.input.template : null,
      templateName: task.template_current ? (task.template_snapshot?.name ?? '') : '',
      retryOf: task.id,
    };
    draft.save(next, true);
    setNotice(
      !task.materials_current
        ? t(
            '部分材料已变化，请移除旧片段并重新选择后生成。',
            'Some materials have changed. Replace the old excerpts before generating again.',
          )
        : !task.template_current
          ? t(
              '原模板已变化或删除，已保留当时的任务要求。',
              'The original template changed or was deleted. The task instructions are retained.',
            )
          : '',
    );
    setLocation({ mode: 'compose', id: null });
  }
  const newTask = () => {
    setNotice('');
    setLocation({ mode: 'compose', id: null });
  };
  const history = () => setLocation({ mode: 'history', id: null });
  if (location.mode === 'history')
    return (
      <TaskHistory
        identity={identity}
        onNew={newTask}
        onSelect={(id) => setLocation({ mode: 'task', id })}
      />
    );
  if (location.mode === 'task' && location.id)
    return (
      <>
        <TaskDetail
          key={location.id}
          {...props}
          taskId={location.id}
          onNew={newTask}
          onHistory={history}
          onReuse={(task) => {
            if (
              draft.value.title ||
              draft.value.instructions ||
              draft.value.materials.length ||
              draft.unreadable
            )
              setReusePending(task);
            else reuse(task);
          }}
        />
        {reusePending && (
          <ActionConfirmDialog
            title={t('替换尚未提交的任务草稿？', 'Replace the unsubmitted task draft?')}
            description={t(
              '已有任务草稿尚未提交。继续将用这次历史任务的要求和材料替换它。',
              'There is an unsubmitted draft. Continuing replaces it with this historical task’s instructions and materials.',
            )}
            confirmLabel={t('替换草稿', 'Replace draft')}
            onCancel={() => setReusePending(null)}
            onConfirm={() => {
              const task = reusePending;
              setReusePending(null);
              reuse(task);
            }}
          />
        )}
      </>
    );
  return (
    <TaskComposer
      {...props}
      draft={draft}
      notice={notice}
      onHistory={history}
      onStarted={(task) => {
        draft.save(emptyTaskDraft());
        setLocation({ mode: 'task', id: task.id });
      }}
    />
  );
}

function TaskComposer({
  identity,
  settingsRevision,
  materialRequest,
  onMaterialsHandled,
  initialScope,
  onOpenModels,
  onOpenEvidence,
  draft,
  notice,
  onHistory,
  onStarted,
}: Props & {
  draft: ReturnType<typeof useTaskDraft<TaskDraft>>;
  notice: string;
  onHistory: () => void;
  onStarted: (task: KnowledgeTask) => void;
}) {
  const { t, language } = useI18n();
  const [scope] = useState(() => initialScope ?? emptyScope());
  const [settings, setSettings] = useState<GenerationSettings | null>(null);
  const [templates, setTemplates] = useState(false);
  const [chosenTemplate, setChosenTemplate] = useState<KnowledgeTaskTemplate | null>(null);
  const [clear, setClear] = useState(false);
  const [error, setError] = useState('');
  const actions = useInferenceAction(identity, 'knowledge-task/start');
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const value = draft.value;
  const needsMaterialReview = !!value.materialWarnings?.length && !value.materialWarningsAccepted;
  function change(patch: Partial<TaskDraft>) {
    draft.save({ ...value, ...patch });
  }
  function apply(template: KnowledgeTaskTemplate) {
    change({
      instructions: template.instructions,
      template: { id: template.id, row_version: template.row_version },
      templateName: template.name,
    });
    setChosenTemplate(null);
    setTemplates(false);
  }
  async function start() {
    if (
      !settings ||
      actions.locked ||
      draft.error ||
      materialRequest ||
      needsMaterialReview ||
      !value.materials.length ||
      !value.instructions.trim()
    )
      return;
    setError('');
    const result = await actions.execute(
      {
        kind: 'knowledge_tasks_start',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            title: value.title.trim() || value.instructions.trim().split('\n')[0].slice(0, 120),
            instructions: value.instructions.trim(),
            materials: value.materials,
            template: value.template,
            period: null,
            generation_settings_version: settings.row_version,
            options: null,
            language,
            retry_of: value.retryOf,
          },
        },
      },
      'knowledge_task_mutation',
    );
    if (result && live.current) onStarted(result.task);
  }
  async function recover() {
    const result = await actions.retry<'knowledge_task_mutation'>();
    if (result && live.current) onStarted(result.task);
  }
  return (
    <PageFrame
      title={t('知识任务', 'Knowledge tasks')}
      description={t(
        '用已有资料和想法，完成眼前的事。',
        'Use your knowledge to work toward a concrete result.',
      )}
      className="knowledge-task-page"
      actions={
        <button className="secondary-button" onClick={onHistory}>
          <History size={16} />
          {t('任务历史', 'Task history')}
        </button>
      }
      footer={
        <>
          <span className="task-footer-hint">
            {t('生成后由你审阅并保存。', 'Review the result before saving.')}
          </span>
          {actions.pending ? (
            <button
              className="primary-button"
              disabled={actions.busy}
              onClick={() => void recover()}
            >
              {t('确认上次任务', 'Recover previous task')}
            </button>
          ) : (
            <button
              className="primary-button"
              disabled={
                actions.locked ||
                !settings ||
                !!draft.error ||
                !!materialRequest ||
                (settings.profile !== null &&
                  (needsMaterialReview || !value.materials.length || !value.instructions.trim()))
              }
              onClick={settings?.profile ? () => void start() : onOpenModels}
            >
              <Sparkles size={16} />
              {actions.busy
                ? t('正在创建…', 'Starting…')
                : settings && !settings.profile
                  ? t('配置模型', 'Configure model')
                  : t('开始生成', 'Generate')}
            </button>
          )}
        </>
      }
    >
      <div className="task-content task-form">
        {materialRequest && onMaterialsHandled && (
          <IncomingTaskMaterials
            identity={identity}
            request={materialRequest}
            value={value}
            unreadable={draft.unreadable}
            save={draft.save}
            onHandled={onMaterialsHandled}
          />
        )}
        {!!value.materialWarnings?.length && (
          <section
            className="task-material-warning"
            aria-label={t('材料范围提示', 'Material coverage')}
          >
            <p>
              {t(
                '部分所选内容没有完整带入。请核对下方实际使用的片段。',
                'Some selected content was not loaded in full. Review the excerpts actually included below.',
              )}
            </p>
            <ul className="task-coverage-list">
              {value.materialWarnings.map((item) => (
                <li key={item.object_id}>
                  <strong>{item.title || t('所选条目', 'Selected item')}</strong>
                  <span>{materialCoverageText(item, t)}</span>
                </li>
              ))}
            </ul>
            <label className="task-coverage-accept">
              <input
                type="checkbox"
                checked={!!value.materialWarningsAccepted}
                onChange={(e) => change({ materialWarningsAccepted: e.target.checked })}
              />
              {t('本次仅使用已载入的片段', 'Use only the loaded excerpts for this task')}
            </label>
          </section>
        )}
        {notice && <p role="status">{notice}</p>}
        {(error || actions.error) && <p role="alert">{error || actions.error}</p>}
        {draft.error && (
          <div role="alert">
            <p>
              {t('任务草稿暂未安全保存：', 'The task draft could not be saved: ')}
              {draft.error}
            </p>
            <button
              className="secondary-button"
              onClick={() => (draft.unreadable ? setClear(true) : draft.save(value))}
            >
              {draft.unreadable
                ? t('重置损坏的草稿', 'Reset unreadable draft')
                : t('重试保存草稿', 'Retry saving draft')}
            </button>
          </div>
        )}
        <label>
          {t('任务名称', 'Task name')}
          <input
            className="ui-input"
            placeholder={t(
              '例如：比较两种方案（可选）',
              'For example: Compare two options (optional)',
            )}
            value={value.title}
            maxLength={200}
            disabled={actions.locked || draft.unreadable}
            onChange={(event) => change({ title: event.target.value })}
          />
        </label>
        <div className="task-field-heading">
          <label htmlFor="knowledge-task-goal">
            {t('想完成什么', 'What would you like to accomplish?')}
          </label>
          <button
            className="text-button"
            disabled={actions.locked || draft.unreadable}
            onClick={() => setTemplates(true)}
          >
            {t('使用模板', 'Use a template')}
          </button>
        </div>
        <textarea
          id="knowledge-task-goal"
          className="ui-input task-instructions"
          placeholder={t(
            '说明你的目标、关注点和期望的成果。',
            'Describe your goal, what matters, and the result you need.',
          )}
          value={value.instructions}
          maxLength={8000}
          disabled={actions.locked || draft.unreadable}
          onChange={(event) => change({ instructions: event.target.value })}
        />
        {value.template && (
          <div className="task-template-selected">
            <span>
              {t('来自模板：', 'Template: ')}
              {value.templateName}
            </span>
            <button
              className="icon-button"
              disabled={actions.locked}
              aria-label={t('不使用模板，保留当前要求', 'Remove template and keep instructions')}
              onClick={() => change({ template: null, templateName: '' })}
            >
              <X size={14} />
            </button>
          </div>
        )}
        <section className="task-material-section" aria-labelledby="knowledge-task-materials-title">
          <div className="task-field-heading">
            <h2 id="knowledge-task-materials-title">
              {t('所用材料', 'Materials')}
              {value.materials.length ? ` · ${value.materials.length}` : ''}
            </h2>
            <ChatMaterials
              identity={identity}
              scope={scope}
              value={value.materials}
              onChange={(materials) => change({ materials })}
              disabled={actions.locked || draft.unreadable}
              triggerLabel={t('添加材料', 'Add materials')}
            />
          </div>
          {!value.materials.length && (
            <p className="muted">
              {t(
                '选择这次任务实际使用的资料或笔记片段。',
                'Choose the source or note excerpts to use for this task.',
              )}
            </p>
          )}
          <TaskMaterials
            materials={value.materials}
            onOpen={onOpenEvidence}
            onError={setError}
            onRemove={
              actions.locked
                ? undefined
                : (id) => change({ materials: value.materials.filter((item) => item.id !== id) })
            }
          />
        </section>
        <ChannelModelPicker
          identity={identity}
          scope="knowledge-task"
          refreshKey={settingsRevision}
          onChanged={setSettings}
          onManage={onOpenModels}
          disabled={actions.locked}
        />
        {!!(value.title || value.instructions || value.materials.length) && (
          <button
            className="text-button task-clear"
            disabled={actions.locked}
            onClick={() => setClear(true)}
          >
            {t('清空草稿', 'Clear draft')}
          </button>
        )}
      </div>
      {templates && (
        <TaskTemplatesDialog
          identity={identity}
          onClose={() => setTemplates(false)}
          onUse={(template) => {
            if (value.instructions.trim() && value.instructions !== template.instructions)
              setChosenTemplate(template);
            else apply(template);
          }}
        />
      )}
      {chosenTemplate && (
        <ActionConfirmDialog
          title={t('用模板替换任务要求？', 'Replace instructions with this template?')}
          description={t(
            '已选材料和任务名称会保留，当前填写的任务要求将被替换。',
            'Selected materials and task name are kept. The current instructions will be replaced.',
          )}
          confirmLabel={t('使用模板', 'Use template')}
          onCancel={() => setChosenTemplate(null)}
          onConfirm={() => apply(chosenTemplate)}
        />
      )}
      {clear && (
        <ActionConfirmDialog
          title={t('清空当前任务草稿？', 'Clear this task draft?')}
          description={t(
            '清空任务要求和材料选择，已有任务历史保持不变。',
            'Clear these instructions and selected materials. Existing tasks remain in history.',
          )}
          confirmLabel={t('清空草稿', 'Clear draft')}
          onCancel={() => setClear(false)}
          onConfirm={() => {
            draft.save(emptyTaskDraft(), true);
            setClear(false);
          }}
        />
      )}
    </PageFrame>
  );
}

function TaskMaterials({
  materials,
  onOpen,
  onRemove,
  onError,
}: {
  materials: SearchEvidence[];
  onOpen: OpenEvidence;
  onRemove?: (id: string) => void;
  onError: (error: string) => void;
}) {
  const { t } = useI18n();
  return (
    <ol className="task-material-list">
      {materials.map((item, index) => (
        <li key={item.id}>
          <details>
            <summary>
              <span className="task-material-number">{index + 1}</span>
              {item.title}
            </summary>
            <p className="task-preserve-text">{item.text}</p>
            <button
              className="text-button"
              onClick={() =>
                void onOpen(item).catch((error) => onError(inferenceErrorMessage(error, t)))
              }
            >
              <BookOpen size={14} />
              {t('打开原文', 'Open source')}
            </button>
          </details>
          {onRemove && (
            <button
              className="icon-button"
              aria-label={`${t('移除材料', 'Remove material')} ${index + 1}`}
              onClick={() => onRemove(item.id)}
            >
              <X size={16} />
            </button>
          )}
        </li>
      ))}
    </ol>
  );
}

function TaskHistory({
  identity,
  onNew,
  onSelect,
}: {
  identity: WorkspaceIdentity;
  onNew: () => void;
  onSelect: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const [query, setQuery] = usePageNavigationState('knowledge-task.history-query', '');
  const [page, setPage] = usePageNavigationState('knowledge-task.history-page', 1);
  const [size, setSize] = usePageNavigationState('knowledge-task.history-size', 10);
  const [scroll, setScroll] = usePageNavigationState('knowledge-task.history-scroll', 0);
  const [data, setData] = useState<TaskHistoryPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const viewport = useRef<HTMLUListElement>(null);
  useEffect(() => {
    let live = true;
    setLoading(true);
    setError('');
    const timer = setTimeout(
      () => {
        void call(
          {
            kind: 'knowledge_tasks_page',
            payload: { page, limit: size, query, template_id: null },
          },
          'knowledge_tasks_page',
          identity,
        )
          .then((result) => {
            if (live) {
              setData(result);
              setPage(result.page);
            }
          })
          .catch((failure) => {
            if (live) setError(inferenceErrorMessage(failure, t));
          })
          .finally(() => {
            if (live) setLoading(false);
          });
      },
      query ? 180 : 0,
    );
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [identity.local_instance_id, identity.workspace_id, query, page, size, reload]);
  useLayoutEffect(() => {
    if (data && viewport.current) viewport.current.scrollTop = scroll;
  }, [data]);
  function move(next: number) {
    setScroll(0);
    setPage(next);
  }
  return (
    <PageFrame
      title={t('任务历史', 'Task history')}
      className="knowledge-task-page task-history-page"
      actions={
        <button className="secondary-button" onClick={onNew}>
          <Plus size={16} />
          {t('新任务', 'New task')}
        </button>
      }
      toolbar={
        <>
          <input
            className="ui-input"
            type="search"
            aria-label={t('搜索任务历史', 'Search task history')}
            placeholder={t('搜索任务', 'Search tasks')}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              move(1);
            }}
          />
          <button
            className="icon-button"
            aria-label={t('刷新任务历史', 'Refresh task history')}
            disabled={loading}
            onClick={() => setReload((n) => n + 1)}
          >
            <RefreshCw size={16} />
          </button>
        </>
      }
      footer={
        <Pagination
          index={data?.page ?? page}
          size={size}
          count={data?.items.length ?? 0}
          total={data?.total ?? 0}
          loading={loading}
          canPrevious={page > 1}
          canNext={!!data && page * size < data.total}
          onSize={(n) => {
            setSize(n);
            move(1);
          }}
          onPrevious={() => move(page - 1)}
          onNext={() => move(page + 1)}
          onGoTo={move}
        />
      }
    >
      {error && (
        <p role="alert" className="task-content">
          {error}
        </p>
      )}
      {!loading && !data?.items.length && !error && (
        <div className="task-content">
          <h2>{t('还没有任务记录', 'No tasks yet')}</h2>
          <p className="muted">
            {t(
              '选择材料并提出目标，开始你的第一个知识任务。',
              'Choose materials and a goal to start your first knowledge task.',
            )}
          </p>
        </div>
      )}
      <ul
        className="task-history-list"
        ref={viewport}
        aria-busy={loading}
        onScroll={() => {
          if (!loading && data) setScroll(viewport.current?.scrollTop ?? 0);
        }}
      >
        {data?.items.map((item) => (
          <li key={item.id}>
            <button disabled={loading} onClick={() => onSelect(item.id)}>
              <strong>{item.title}</strong>
              <span>
                {taskStatus(item.status, t)}
                {item.output_count
                  ? t(` · ${item.output_count} 条审阅记录`, ` · ${item.output_count} reviews`)
                  : ''}
              </span>
              <small>
                {displayDate(item.created_at, language)}
                {item.template_name ? ` · ${item.template_name}` : ''}
              </small>
            </button>
          </li>
        ))}
      </ul>
    </PageFrame>
  );
}

function TaskDetail({
  identity,
  taskId,
  onHistory,
  onNew,
  onReuse,
  onOpenModels,
  onOpenEvidence,
  onOpenNote,
  onOpenProposal,
}: Props & {
  taskId: string;
  onHistory: () => void;
  onNew: () => void;
  onReuse: (task: KnowledgeTask) => void;
}) {
  const { t, language } = useI18n();
  const [task, setTask] = useState<KnowledgeTask | null>(null);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const [savedScroll, setSavedScroll] = usePageNavigationState(
    `knowledge-task.scroll/${taskId}`,
    0,
  );
  const content = useRef<HTMLDivElement>(null);
  const initialScroll = useRef(savedScroll);
  const actions = useInferenceAction(identity, `knowledge-task/cancel/${taskId}`);
  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      try {
        const value = await call(
          { kind: 'knowledge_tasks_get', payload: { task_id: taskId } },
          'knowledge_task',
          identity,
        );
        if (!live) return;
        setTask(value);
        setError('');
        if (chatActive(value.run.status)) timer = setTimeout(() => void read(), 850);
      } catch (failure) {
        if (live) setError(inferenceErrorMessage(failure, t));
      }
    }
    void read();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [taskId, reload, identity.local_instance_id, identity.workspace_id]);
  useLayoutEffect(() => {
    const root = content.current;
    const body = root?.closest('.page-frame-body');
    if (!task || !root || !body) return;
    let restoring = true;
    const remember = () => {
      if (!restoring) setSavedScroll(body.scrollTop);
    };
    const stop = () => {
      restoring = false;
      mutation.disconnect();
      resize.disconnect();
    };
    const restore = () => {
      if (!restoring) return;
      // The result and save records load separately. Keep the saved position until
      // both have rendered, so a temporarily short page cannot overwrite it with 0.
      body.scrollTop = initialScroll.current;
      if (!root.querySelector('[aria-busy="true"]')) stop();
    };
    const mutation = new MutationObserver(restore);
    const resize = new ResizeObserver(restore);
    mutation.observe(root, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['aria-busy'],
    });
    resize.observe(root);
    resize.observe(body);
    body.addEventListener('scroll', remember, { passive: true });
    body.addEventListener('wheel', stop, { passive: true });
    body.addEventListener('pointerdown', stop, { passive: true });
    body.addEventListener('touchstart', stop, { passive: true });
    body.addEventListener('keydown', stop);
    restore();
    return () => {
      remember();
      stop();
      body.removeEventListener('scroll', remember);
      body.removeEventListener('wheel', stop);
      body.removeEventListener('pointerdown', stop);
      body.removeEventListener('touchstart', stop);
      body.removeEventListener('keydown', stop);
    };
  }, [!!task, setSavedScroll]);
  async function cancel(recover = false) {
    const result = recover
      ? await actions.retry<'chat_run_mutation'>()
      : await actions.execute(
          {
            kind: 'chat_runs_cancel',
            payload: { command_id: crypto.randomUUID(), run_id: taskId },
          },
          'chat_run_mutation',
        );
    if (result) setReload((n) => n + 1);
  }
  const active = !!task && chatActive(task.run.status);
  return (
    <PageFrame
      title={task?.input.title ?? t('正在读取任务…', 'Loading task…')}
      className="knowledge-task-page task-detail-page"
      actions={
        <>
          <button className="secondary-button" onClick={onHistory}>
            <ArrowLeft size={16} />
            {t('任务历史', 'Task history')}
          </button>
          <button className="secondary-button" onClick={onNew}>
            <Plus size={16} />
            {t('新任务', 'New task')}
          </button>
        </>
      }
      toolbar={
        <>
          <span role="status" className="task-state-hint">
            {task
              ? task.latest_output?.status === 'applied'
                ? t('已保存到知识库', 'Saved to knowledge')
                : taskStatus(task.run.status, t)
              : t('正在读取…', 'Loading…')}
          </span>
          {actions.pending ? (
            <button disabled={actions.busy} onClick={() => void cancel(true)}>
              {t('确认停止结果', 'Recover stop result')}
            </button>
          ) : active ? (
            <button
              className="secondary-button"
              disabled={actions.locked}
              onClick={() => void cancel()}
            >
              {t('停止生成', 'Stop generation')}
            </button>
          ) : (
            task && (
              <button className="secondary-button" onClick={() => onReuse(task)}>
                {t('调整后再生成', 'Adjust and run again')}
              </button>
            )
          )}
        </>
      }
    >
      <div className="task-content" ref={content}>
        {(error || actions.error) && (
          <div role="alert">
            <p>{error || actions.error}</p>
            <button onClick={() => setReload((n) => n + 1)}>
              {t('重新读取任务', 'Reload task')}
            </button>
          </div>
        )}
        {task && (
          <>
            <div className="task-run-summary">
              <span>{displayDate(task.run.created_at, language)}</span>
              <span>
                {task.channel_name ?? task.run.model?.provider}
                {task.run.model ? ` · ${task.run.model.model}` : ''}
              </span>
            </div>
            {task.run.error_code && (
              <p role="alert">{inferenceErrorMessage(task.run.error_code, t)}</p>
            )}
            {task.run.status === 'model_unconfigured' && (
              <button className="secondary-button" onClick={onOpenModels}>
                {t('打开模型与连接', 'Open models & connections')}
              </button>
            )}
            {!task.materials_current && (
              <p role="status">
                {t(
                  '部分来源已变化或不可用；这里保留的是生成时的材料快照。',
                  'Some sources have changed or are unavailable. The original material snapshots are retained here.',
                )}
              </p>
            )}
            <details className="task-context">
              <summary>
                {t(
                  `所用材料与任务要求 · ${task.input.materials.length} 个片段`,
                  `Materials and instructions · ${task.input.materials.length} excerpts`,
                )}
              </summary>
              <p className="task-preserve-text">{task.input.instructions}</p>
              {task.template_snapshot && (
                <p className="muted">
                  {t('所用模板：', 'Template: ')}
                  {task.template_snapshot.name} ·{' '}
                  {t(
                    `版本 ${task.template_snapshot.row_version}`,
                    `version ${task.template_snapshot.row_version}`,
                  )}
                  {!task.template_current && t('（模板已修改或删除）', ' (changed or deleted)')}
                </p>
              )}
              <TaskMaterials
                materials={task.input.materials}
                onOpen={onOpenEvidence}
                onError={setError}
              />
            </details>
            {task.run.status === 'completed' ? (
              <TaskResultEditor
                key={task.id}
                identity={identity}
                task={task}
                onOpenProposal={onOpenProposal}
              />
            ) : task.output_text ? (
              <ChatMarkdown text={task.output_text} identity={identity} renderRich={!active} />
            ) : (
              <p className="task-empty-result">
                {active
                  ? t('正在根据所选材料生成…', 'Generating from selected materials…')
                  : t(
                      '这次任务没有生成正文。任务要求与材料已保留。',
                      'This run produced no text. Its instructions and materials are retained.',
                    )}
              </p>
            )}
            {task.output_count > 0 && (
              <TaskOutputs
                identity={identity}
                task={task}
                onOpenNote={onOpenNote}
                onOpenProposal={onOpenProposal}
              />
            )}
          </>
        )}
      </div>
    </PageFrame>
  );
}

function TaskResultEditor({
  identity,
  task,
  onOpenProposal,
}: {
  identity: WorkspaceIdentity;
  task: KnowledgeTask;
  onOpenProposal: (id: string) => void;
}) {
  const { t } = useI18n();
  const draft = useTaskDraft(
    `${draftKey(identity)}/result/${task.id}`,
    () => task.output_text,
    (raw) => {
      const value: unknown = JSON.parse(raw);
      if (typeof value !== 'string') throw new Error('Invalid result draft');
      return value;
    },
  );
  const [editing, setEditing] = useState(false);
  const [review, setReview] = useState(false);
  const [reset, setReset] = useState(false);
  return (
    <section className="task-result" aria-label={t('任务成果', 'Task result')}>
      <div className="task-field-heading">
        <h2>{t('成果草稿', 'Result draft')}</h2>
        <button
          className="text-button"
          disabled={draft.unreadable}
          onClick={() => setEditing((value) => !value)}
        >
          {editing ? t('预览草稿', 'Preview draft') : t('编辑草稿', 'Edit draft')}
        </button>
      </div>
      {draft.error && (
        <div role="alert">
          <p>
            {t('审阅草稿未保存：', 'Review draft was not saved: ')}
            {draft.error}
          </p>
          <button onClick={() => (draft.unreadable ? setReset(true) : draft.save(draft.value))}>
            {draft.unreadable
              ? t('恢复生成原文', 'Restore generated text')
              : t('重试保存草稿', 'Retry saving draft')}
          </button>
        </div>
      )}
      {editing ? (
        <label className="task-form">
          {t('编辑成果（Markdown）', 'Edit result (Markdown)')}
          <textarea
            className="ui-input task-result-editor"
            aria-label={t('编辑成果（Markdown）', 'Edit result (Markdown)')}
            value={draft.value}
            onChange={(e) => draft.save(e.target.value)}
          />
          <span className="muted">
            {t('原始生成内容保留在任务中。', 'The original generated text remains in the task.')}
          </span>
        </label>
      ) : (
        <ChatMarkdown
          identity={identity}
          text={draft.value}
          renderRich
          materials={task.input.materials}
        />
      )}
      <div className="task-result-actions">
        <button
          className="primary-button"
          disabled={!!draft.error || !draft.value.trim()}
          onClick={() => setReview((value) => !value)}
        >
          {review ? t('收起保存选项', 'Hide save options') : t('审阅并保存', 'Review and save')}
        </button>
        {draft.value !== task.output_text && (
          <button className="text-button" onClick={() => setReset(true)}>
            {t('恢复生成原文', 'Restore generated text')}
          </button>
        )}
      </div>
      {review && (
        <ChatProposalPanel
          identity={identity}
          runId={task.id}
          text={draft.value}
          initialTitle={task.input.title}
          sourceKind="task"
          onOpenProposal={onOpenProposal}
        />
      )}
      {reset && (
        <ActionConfirmDialog
          title={t('恢复生成原文？', 'Restore generated text?')}
          description={t(
            '将替换当前审阅草稿；已保存的笔记不会改变。',
            'This replaces the review draft. Saved notes stay unchanged.',
          )}
          confirmLabel={t('恢复原文', 'Restore text')}
          onCancel={() => setReset(false)}
          onConfirm={() => {
            draft.save(task.output_text, true);
            setReset(false);
          }}
        />
      )}
    </section>
  );
}

function TaskOutputs({
  identity,
  task,
  onOpenNote,
  onOpenProposal,
}: {
  identity: WorkspaceIdentity;
  task: KnowledgeTask;
  onOpenNote: Props['onOpenNote'];
  onOpenProposal: Props['onOpenProposal'];
}) {
  const { t } = useI18n();
  const [data, setData] = useState<KnowledgeTaskOutputsPage | null>(null);
  const [page, setPage] = usePageNavigationState(`knowledge-task.outputs-page/${task.id}`, 1);
  const [size, setSize] = usePageNavigationState(`knowledge-task.outputs-size/${task.id}`, 10);
  const [scroll, setScroll] = usePageNavigationState(`knowledge-task.outputs-scroll/${task.id}`, 0);
  const list = useRef<HTMLUListElement>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let live = true;
    setLoading(true);
    setError('');
    void call(
      { kind: 'knowledge_task_outputs_page', payload: { task_id: task.id, page, limit: size } },
      'knowledge_task_outputs_page',
      identity,
    )
      .then((value) => {
        if (live) {
          setData(value);
          setPage(value.page);
        }
      })
      .catch((failure) => {
        if (live) setError(inferenceErrorMessage(failure, t));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [task.id, task.output_count, task.latest_output?.status, page, size, reload]);
  useLayoutEffect(() => {
    if (data && list.current) list.current.scrollTop = scroll;
  }, [data]);
  return (
    <section className="task-outputs">
      <h2>{t('审阅与保存记录', 'Review and save history')}</h2>
      {error && (
        <p role="alert">
          {error}
          <button onClick={() => setReload((n) => n + 1)}>{t('重试', 'Retry')}</button>
        </p>
      )}
      <ul
        ref={list}
        aria-busy={loading}
        onScroll={() => {
          if (!loading) setScroll(list.current?.scrollTop ?? 0);
        }}
      >
        {data?.items.map((item) => (
          <li key={item.id}>
            <strong>{item.title}</strong>
            <span>
              {item.status === 'applied'
                ? t('已保存', 'Saved')
                : item.status === 'pending'
                  ? t('待审阅', 'Pending review')
                  : t('未采用', 'Rejected')}
            </span>
            <div className="task-row-actions">
              {item.proposal_id && (
                <button className="text-button" onClick={() => onOpenProposal(item.proposal_id!)}>
                  {t('查看审阅', 'View review')}
                </button>
              )}
              {item.applied_note_id && (
                <button
                  className="text-button"
                  disabled={!item.note_available || item.note_trashed}
                  onClick={() =>
                    void onOpenNote(item.applied_note_id!).catch((failure) =>
                      setError(inferenceErrorMessage(failure, t)),
                    )
                  }
                >
                  {item.note_trashed
                    ? t('成果在回收站', 'Result in trash')
                    : !item.note_available
                      ? t('成果已不可用', 'Result unavailable')
                      : t('打开成果', 'Open result')}
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
      {data && data.total > size && (
        <Pagination
          index={data?.page ?? page}
          size={size}
          count={data?.items.length ?? 0}
          total={data?.total ?? 0}
          loading={loading}
          canPrevious={page > 1}
          canNext={!!data && page * size < data.total}
          onSize={(n) => {
            setScroll(0);
            setSize(n);
            setPage(1);
          }}
          onPrevious={() => {
            setScroll(0);
            setPage((n) => n - 1);
          }}
          onNext={() => {
            setScroll(0);
            setPage((n) => n + 1);
          }}
          onGoTo={(n) => {
            setScroll(0);
            setPage(n);
          }}
        />
      )}
    </section>
  );
}
