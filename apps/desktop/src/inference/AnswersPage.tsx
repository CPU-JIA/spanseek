import { Select } from '../ui/Select';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isTauri } from '@tauri-apps/api/core';
import type {
  AgentProgress,
  DesktopEvent,
  GenerationProfile,
  AnswerMutation,
  AnswerRecord,
  AnswerStatus,
  NoteSummary,
  ProposalMutation,
  SearchQuery,
  SearchScope,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { emptyScope, ScopePicker, type OpenEvidence } from './SearchPage';
import { useInferenceAction } from './useInferenceAction';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { PageFrame } from '../layout/PageFrame';
import { requestMarkdown } from './markdown-request';
import { useMarkdownDocument } from './useMarkdownDocument';
import { StandaloneDocumentContent } from '../rich/DocumentContent';
import { EDITOR_DOCUMENT_SCHEMA } from '../editor/document-schema';
import { latestAnswer } from './inference-state';
import { agentDefaults, latestAgentProgress } from './agent-state';
import { FeedbackPanel } from './FeedbackPanel';
import { inferenceErrorMessage } from './inference-errors';
import { ChannelModelPicker } from './ChannelModelPicker';
import { DEFAULT_RETRIEVAL_PARAMS, useRetrievalSettings } from './retrieval-settings';
import { REFUSAL_MARKER, refusalCopy } from './refusal-copy';
import './inference.css';
import { AiTrustNote } from './ai-trust';
import { AiUsagePanel } from './ai-usage';

export function AnswersPage({
  identity,
  answerId,
  onSelectAnswer,
  onOpenEvidence,
  onOpenProposal,
  onOpenModels,
  initialScope,
}: {
  identity: WorkspaceIdentity;
  answerId: string | null;
  onSelectAnswer: (id: string) => void;
  onOpenEvidence: OpenEvidence;
  onOpenProposal: (id: string) => void;
  onOpenModels: () => void;
  initialScope?: SearchScope;
}) {
  const { t, language } = useI18n();
  const [answer, setAnswer] = useState<AnswerRecord | null>(null);
  const [agentMode, setAgentMode] = usePageNavigationState('answers/agent-mode', false);
  const [maxOutputTokens, setMaxOutputTokens] = usePageNavigationState(
    'answers/output-budget',
    8192,
  );
  const modelIdentity = `${identity.local_instance_id}/${identity.workspace_id}`;
  const [modelSelection, setModelSelection] = useState<{
    identity: string;
    profile: GenerationProfile | null;
  } | null>(null);
  const profile = modelSelection?.identity === modelIdentity ? modelSelection.profile : null;
  const [progress, setProgress] = useState<AgentProgress | null>(null);
  const retrieval = useRetrievalSettings(identity);
  const seededParams = useRef(false);
  const [query, setQuery] = useState<SearchQuery>({
    query: '',
    mode: 'hybrid',
    scope: initialScope ?? emptyScope(),
    limit: DEFAULT_RETRIEVAL_PARAMS.top_k,
    minimum_cosine: DEFAULT_RETRIEVAL_PARAMS.minimum_cosine ?? null,
  });
  useEffect(() => {
    const loaded = retrieval.settings;
    if (!loaded || seededParams.current) return;
    seededParams.current = true;
    setQuery((old) => ({
      ...old,
      limit: loaded.params.top_k,
      minimum_cosine: loaded.params.minimum_cosine ?? null,
    }));
  }, [retrieval.settings]);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  // `answers_list` answers "the page that follows this cursor" and reports no
  // cursor of its own, so the next cursor stays the guess the append list used:
  // a full page means older answers may exist. A new workspace or page size
  // starts page 1 again; the reload/answerId effect below re-reads page 1.
  const historyList = usePagedList<AnswerRecord>(
    `${identity.local_instance_id}/${identity.workspace_id}`,
    async (cursor, size) => {
      const items = await call(
        { kind: 'answers_list', payload: { after_id: cursor, limit: size } },
        'answers',
        identity,
      );
      return {
        items,
        next_cursor: items.length === size ? (items.at(-1)?.id ?? null) : null,
      };
    },
  );
  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [noteQuery, setNoteQuery] = useState('');
  const [noteId, setNoteId] = useState('');
  const [newNote, setNewNote] = useState(true);
  const [newTitle, setNewTitle] = useState('');
  const [notesCursor, setNotesCursor] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const preparation = useRef<AbortController | null>(null);
  const preparedPreview = useMarkdownDocument(
    answer?.text ?? '',
    false,
    answer?.status === 'completed',
  );
  const [feedbackMessage, setFeedbackMessage] = useState<string | null>(null);
  const [feedbackComment, setFeedbackComment] = useState('');
  const actions = useInferenceAction(identity, 'answers');
  const live = useRef(true);
  const currentAnswer = useRef(answerId);
  currentAnswer.current = answerId;
  const noteSequence = useRef(0);
  const structuredUnavailable = profile?.capabilities?.structured_transport === null;
  const agentAvailable =
    profile !== null && profile.provider !== 'ollama' && !structuredUnavailable;
  useEffect(() => {
    setProgress(null);
    if (!isTauri() || !answerId) return;
    let current = true;
    const subscription = listen<DesktopEvent>('pkms:event', ({ payload: event }) => {
      if (!current || event.kind !== 'agent_progress') return;
      setProgress((previous) =>
        latestAgentProgress(
          previous,
          event.payload.progress,
          identity,
          event.payload.identity,
          answerId,
        ),
      );
    });
    void subscription.catch((e) => {
      if (current) setError(message(e));
    });
    return () => {
      current = false;
      void subscription.then(
        (unlisten) => unlisten(),
        () => {},
      );
    };
  }, [answerId, identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      preparation.current?.abort();
    };
  }, []);
  // The reload button and a decided mutation (create/cancel/confirm) belong to
  // page 1, exactly like the append list. Choosing a history row that is already
  // on screen is only a selection, so it keeps the page the user is reading.
  // The mount read is the pager's own, so the first run here is skipped.
  const historyTrigger = useRef({ reload, answerId });
  useEffect(() => {
    const previous = historyTrigger.current;
    historyTrigger.current = { reload, answerId };
    if (previous.reload === reload && previous.answerId === answerId) return;
    const selectedVisible =
      !!answerId && (historyList.page?.items.some((item) => item.id === answerId) ?? false);
    if (previous.reload !== reload || !selectedVisible) void historyList.loadFirst();
  }, [reload, answerId, historyList]);
  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    setAnswer(null);
    preparation.current?.abort();
    setFeedbackMessage(null);
    setFeedbackComment('');
    setError(null);
    if (!answerId) return;
    async function read() {
      try {
        // The persisted full snapshot recovers missed stream events and resumes after remount.
        const record = await call(
          { kind: 'answers_get', payload: { answer_id: answerId! } },
          'answer',
          identity,
        );
        if (!current) return;
        setAnswer((previous) => latestAnswer(previous, record));
        if (record.status === 'queued' || record.status === 'running')
          timer = setTimeout(() => void read(), 1000);
      } catch (e) {
        if (current) setError(message(e));
      }
    }
    void read();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [answerId, reload]);
  async function start() {
    if (structuredUnavailable) {
      setError(inferenceErrorMessage('model_unsupported_output', t));
      return;
    }
    if (agentMode && !agentAvailable) return;
    const budget = profile?.provider === 'ollama' ? 384 : maxOutputTokens;
    if (
      !Number.isInteger(budget) ||
      budget < 1 ||
      budget > 32768 ||
      (profile && budget >= profile.context_tokens)
    ) {
      setError(inferenceErrorMessage('answer_output_budget_invalid', t));
      return;
    }
    setError(null);
    const value = await actions.execute(
      agentMode
        ? {
            kind: 'answers_agent_start',
            payload: {
              command_id: crypto.randomUUID(),
              search: structuredClone(query),
              agent: agentDefaults(profile?.provider),
              max_output_tokens: budget,
            },
          }
        : {
            kind: 'answers_start',
            payload: {
              command_id: crypto.randomUUID(),
              search: structuredClone(query),
              max_output_tokens: budget,
            },
          },
      'answer_mutation',
    );
    if (value && live.current) onSelectAnswer(value.answer.id);
  }
  async function recover() {
    const expected = actions.pending?.expected;
    const value = await actions.retry();
    if (!value || !live.current) return;
    if (expected === 'proposal_mutation') onOpenProposal((value as ProposalMutation).proposal.id);
    else if (expected === 'answer_mutation') {
      const record = (value as AnswerMutation).answer;
      setAnswer(record);
      setReload((previous) => previous + 1);
      onSelectAnswer(record.id);
    }
  }
  async function cancel() {
    if (!answer) return;
    const value = await actions.execute(
      {
        kind: 'answers_cancel',
        payload: { command_id: crypto.randomUUID(), answer_id: answer.id },
      },
      'answer_mutation',
    );
    if (value && live.current && currentAnswer.current === value.answer.id) {
      setAnswer(value.answer);
      setReload((old) => old + 1);
    }
  }
  async function feedback(rating: number) {
    if (!answer) return;
    const value = await actions.execute(
      {
        kind: 'answers_feedback',
        payload: {
          command_id: crypto.randomUUID(),
          request: { answer_id: answer.id, rating, comment: feedbackComment },
        },
      },
      'answer_mutation',
    );
    if (value && live.current && currentAnswer.current === value.answer.id) {
      setAnswer(value.answer);
      setFeedbackMessage(t('反馈已保存。', 'Feedback saved.'));
    }
  }
  async function findNotes(cursor: string | null = null) {
    const sequence = ++noteSequence.current;
    try {
      const page = await call(
        { kind: 'notes_list', payload: { limit: 30, cursor, trashed: false, query: noteQuery } },
        'notes',
        identity,
      );
      if (live.current && sequence === noteSequence.current) {
        setNotes((old) => (cursor ? [...old, ...page.items] : page.items));
        setNotesCursor(page.next_cursor);
      }
    } catch (e) {
      if (live.current && sequence === noteSequence.current) setError(message(e));
    }
  }
  async function proposal() {
    if (!answer || (!newNote && !noteId) || preparation.current || actions.locked) return;
    const controller = new AbortController();
    preparation.current = controller;
    setPreparing(true);
    setError(null);
    try {
      const note = newNote
        ? null
        : await call({ kind: 'notes_get', payload: { note_id: noteId } }, 'note', identity);
      if (!live.current || currentAnswer.current !== answer.id || controller.signal.aborted) return;
      const prepared = await requestMarkdown(
        {
          kind: 'answer-proposal',
          base: note?.document ?? { type: 'doc', content: [] },
          answer,
        },
        controller.signal,
      );
      if (!live.current || currentAnswer.current !== answer.id || controller.signal.aborted) return;
      const value = await actions.execute(
        {
          kind: 'proposals_create',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              answer_id: answer.id,
              note_id: note?.id ?? null,
              base_revision_id: note?.revision_id ?? null,
              title: note?.title ?? (newTitle.trim() || answer.request.search.query),
              document_schema: EDITOR_DOCUMENT_SCHEMA,
              document: prepared.document,
            },
          },
        },
        'proposal_mutation',
      );
      if (value && live.current && currentAnswer.current === answer.id)
        onOpenProposal(value.proposal.id);
    } catch (e) {
      if (live.current && !controller.signal.aborted && currentAnswer.current === answer.id)
        setError(message(e));
    } finally {
      if (preparation.current === controller) preparation.current = null;
      if (live.current) setPreparing(false);
    }
  }
  async function open(citationId: string) {
    const item = answer?.evidence.find((item) => item.citation_id === citationId);
    if (!item) return;
    try {
      await onOpenEvidence(item.evidence);
    } catch (e) {
      if (live.current) setError(message(e));
    }
  }
  // The saved comment is the record; the draft is what the reader is editing.
  useEffect(() => {
    setFeedbackComment(answer?.feedback?.comment ?? '');
  }, [answer?.id, answer?.feedback?.comment]);
  const insufficientEvidence = (value: AnswerRecord) =>
    value.status === 'no_evidence' && value.error_code === 'insufficient_evidence';
  // FA4-07: the refusal is fixed copy, authored here, and the stored machine
  // marker is never shown as the answer.
  const refusal = refusalCopy(language === 'en');
  const status = (value: AnswerStatus, errorCode: string | null) =>
    value === 'no_evidence' && errorCode === 'insufficient_evidence'
      ? refusal.label
      : {
          queued: t('等待检索与生成', 'Queued for retrieval and generation'),
          running: t('正在生成，尚未完成', 'Generating; not complete'),
          completed: t('生成完成，请核对原文', 'Generated; verify against sources'),
          canceled: t('已取消；保留未完成内容', 'Canceled; partial content retained'),
          timed_out: t('生成超时', 'Generation timed out'),
          no_evidence: t('此范围内没有可用证据', 'No evidence within this scope'),
          invalid_citation: t('引用校验失败，不可作为有效答案', 'Citation validation failed'),
          evidence_changed: t('生成期间证据发生变化', 'Evidence changed during generation'),
          failed: t('生成失败', 'Generation failed'),
          model_unconfigured: t('尚未配置生成模型', 'Generation model is not configured'),
        }[value];
  const renderCitations = (text: string): ReactNode[] =>
    text.split(/(\[E\d+\])/g).map((part, index) => {
      const id = part.slice(1, -1);
      return /^\[E\d+\]$/.test(part) && answer?.evidence.some((item) => item.citation_id === id) ? (
        <button key={index} className="text-button" onClick={() => void open(id)}>
          {part}
        </button>
      ) : (
        part
      );
    });
  const showAnswerText =
    answer && !insufficientEvidence(answer) && answer.text.trim() !== REFUSAL_MARKER;
  return (
    <PageFrame
      className="inference-page answers-page"
      title={t('向自己的知识提问', 'Ask your knowledge')}
      description={t(
        '答案只使用当前范围内可核对的笔记与资料，并保留引用出处。',
        'Answers use only verifiable notes and sources in the current scope, with citations kept.',
      )}
      actions={
        <button className="secondary-button" onClick={() => setReload((value) => value + 1)}>
          {t('重新读取', 'Reload')}
        </button>
      }
    >
      <AiTrustNote mode="answers" />
      {(error || actions.error || historyList.error) && (
        <div className="inference-error" role="alert">
          {error || actions.error || historyList.error}
        </div>
      )}
      {(!actions.ready || actions.pending) && (
        <div className="inference-notice">
          <p>
            {t(
              '有请求需要读取或重新确认，将使用原请求。',
              'A request needs reading or confirmation; the original request will be used.',
            )}
          </p>
          <button disabled={actions.busy} onClick={() => void recover()}>
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      )}
      <AiUsagePanel identity={identity} />
      {structuredUnavailable && (
        <p className="inference-notice" role="status">
          {t(
            '当前模型仅支持普通对话，请选择支持证据回答的模型。',
            'This model supports chat only. Choose a model that supports evidence answers.',
          )}
        </p>
      )}
      <details>
        <summary>
          {t('回答历史', 'Answer history')} · {historyList.page?.items.length ?? 0}
        </summary>
        {historyList.page?.items.map((item) => (
          <p key={item.id}>
            <button className="text-button" onClick={() => onSelectAnswer(item.id)}>
              {item.request.search.query}
            </button>{' '}
            · {status(item.status, item.error_code)} · {displayDate(item.created_at, language)}
          </p>
        ))}
        <ListPagination list={historyList} />
      </details>
      <FeedbackPanel identity={identity} generation={reload} />
      <form
        className="answer-compose"
        onSubmit={(e) => {
          e.preventDefault();
          void start();
        }}
      >
        <label className="inference-field answer-question">
          {t('新问题', 'New question')}
          <textarea
            value={query.query}
            rows={3}
            maxLength={4000}
            disabled={actions.locked}
            placeholder={t(
              '输入问题，在自己的知识库中寻找依据…',
              'Ask a question grounded in your knowledge…',
            )}
            onChange={(e) => setQuery((old) => ({ ...old, query: e.target.value }))}
          />
        </label>
        <div className="answer-compose-toolbar">
          <div className="choice-field inference-field">
            {t('回答方式', 'Answer mode')}
            <Select
              aria-label={t('回答方式', 'Answer mode')}
              value={agentMode ? 'agent' : 'direct'}
              disabled={actions.locked}
              onValueChange={(nextValue) => setAgentMode(nextValue === 'agent')}
            >
              <option value="direct">{t('直接回答', 'Direct answer')}</option>
              <option value="agent">
                {t('知识助手 · 多轮查找', 'Knowledge assistant · multi-step search')}
              </option>
            </Select>
          </div>
          <ChannelModelPicker
            identity={identity}
            scope="answers"
            purpose="structured"
            disabled={actions.locked}
            onChanged={(settings) =>
              setModelSelection({ identity: modelIdentity, profile: settings.profile })
            }
            onManage={onOpenModels}
          />
        </div>
        {profile?.provider !== 'ollama' && (
          <details>
            <summary>{t('生成设置', 'Generation settings')}</summary>
            <label className="inference-field">
              {t('每轮输出预算（token）', 'Output budget per call (tokens)')}
              <input
                type="number"
                min={1}
                max={32768}
                step={1}
                value={maxOutputTokens}
                disabled={actions.locked}
                onChange={(event) => setMaxOutputTokens(Number(event.target.value))}
              />
            </label>
            <p className="muted">
              {t(
                '推理模型的思考和正文可能共享输出预算。助手每轮均使用此设置；输入、证据与历史也占用上下文。',
                'Reasoning and the final text may share this budget. Each assistant call uses it; input, evidence and history also consume context.',
              )}
            </p>
          </details>
        )}
        {agentMode && (
          <p className="inference-notice">
            {agentAvailable
              ? t(
                  `助手将使用 ${profile.model}，把所选范围内找到的证据片段发送给已配置的模型服务。最多 4 轮模型请求、8 次只读工具调用、32 条证据、4 分钟；写入笔记仍需审阅并应用。`,
                  `The assistant uses ${profile.model} and sends evidence found within your selected scope to the configured model service. Up to 4 model requests, 8 read-only tool calls, 32 evidence items and 4 minutes. Saving a note still requires review and application.`,
                )
              : t(
                  '请先在模型设置中配置远程模型，再使用知识助手。',
                  'Configure a remote model in Model settings to use the knowledge assistant.',
                )}
          </p>
        )}
        <ScopePicker
          identity={identity}
          value={query.scope}
          onChange={(scope) => setQuery((old) => ({ ...old, scope }))}
        />
        <button
          className="primary-button"
          disabled={
            actions.locked ||
            structuredUnavailable ||
            !query.query.trim() ||
            (agentMode && !agentAvailable)
          }
        >
          {agentMode
            ? t('在所选范围内启动助手', 'Start assistant within the selected scope')
            : t('在所选范围内生成答案', 'Generate within the selected scope')}
        </button>
      </form>
      {answerId && !answer && !error && <p>{t('正在读取回答…', 'Loading answer…')}</p>}
      {answer && (
        <section>
          <h2>{answer.request.search.query}</h2>
          <div className="inference-notice">
            <p>
              {answer.request.agent
                ? t('知识助手', 'Knowledge assistant')
                : t('直接回答', 'Direct answer')}
              {answer.request.model && ` · ${answer.request.model.model}`}
              {t(
                ` · 每轮输出预算 ${answer.request.max_output_tokens ?? (answer.request.model?.provider === 'ollama' ? 384 : 2048)} token`,
                ` · Output budget ${answer.request.max_output_tokens ?? (answer.request.model?.provider === 'ollama' ? 384 : 2048)} tokens per call`,
              )}
            </p>
            <p>
              {t('本次回答的冻结范围：', 'Frozen scope for this answer: ')}
              {answer.request.search.scope.collection_id
                ? t('指定集合', 'Selected collection')
                : t('全库基础范围', 'Workspace scope')}
              {answer.request.search.scope.object_ids.length > 0 &&
                t(
                  ` · 限定 ${answer.request.search.scope.object_ids.length} 个对象`,
                  ` · ${answer.request.search.scope.object_ids.length} selected objects`,
                )}
              {answer.request.search.scope.tag_ids.length > 0 &&
                t(
                  ` · ${answer.request.search.scope.tag_ids.length} 个标签限制`,
                  ` · ${answer.request.search.scope.tag_ids.length} tag restrictions`,
                )}
              {answer.request.search.scope.kinds.length > 0 &&
                ` · ${answer.request.search.scope.kinds.map((kind) => (kind === 'note' ? t('笔记', 'Notes') : t('资料', 'Sources'))).join(' / ')}`}
              {answer.request.search.scope.source_format &&
                ` · ${answer.request.search.scope.source_format}`}
            </p>
            <button
              className="text-button"
              onClick={() => setQuery({ ...structuredClone(answer.request.search), query: '' })}
            >
              {t('使用相同范围继续提问', 'Ask again with the same scope')}
            </button>
          </div>
          <p role="status">{status(answer.status, answer.error_code)}</p>
          {answer.status === 'running' &&
            answer.request.agent &&
            progress?.answer_id === answer.id && (
              <p role="status">
                {
                  {
                    planning: t('正在规划查找', 'Planning search'),
                    searching: t('正在查找证据', 'Searching evidence'),
                    reading: t('正在阅读证据', 'Reading evidence'),
                    freezing: t('正在核验证据', 'Validating evidence'),
                    generating: t('正在生成答案', 'Generating answer'),
                  }[progress.stage]
                }
                {t(
                  ` · 模型 ${progress.model_calls}/${answer.request.agent.max_model_calls} 轮 · 工具 ${progress.tool_calls}/${answer.request.agent.max_tool_calls} 次 · ${progress.evidence_count} 条证据`,
                  ` · Model ${progress.model_calls}/${answer.request.agent.max_model_calls} · Tools ${progress.tool_calls}/${answer.request.agent.max_tool_calls} · ${progress.evidence_count} evidence items`,
                )}
              </p>
            )}
          <details>
            <summary>{t('本次生成配置', 'Generation configuration')}</summary>
            <p>
              {answer.request.model
                ? `${answer.request.model.provider} · ${answer.request.model.model} · ${answer.request.model.endpoint}`
                : t('未配置生成模型', 'No generation model configured')}
            </p>
            {answer.request.model && (
              <p>
                {t('固定模型摘要：', 'Pinned model digest: ')}
                {answer.request.model.model_digest}
              </p>
            )}
          </details>
          {answer.error_code && !insufficientEvidence(answer) && (
            <p className="inference-error">{inferenceErrorMessage(answer.error_code, t)}</p>
          )}
          {answer.evidence.length > 0 && !answer.evidence_current && (
            <div className="inference-notice">
              {t(
                '来源已经变化。以下保留生成时的证据版本，不代表当前内容。',
                'Sources have changed. The evidence below preserves the generation-time versions.',
              )}
            </div>
          )}
          {(answer.status === 'queued' || answer.status === 'running') && (
            <button
              className="secondary-button"
              disabled={actions.locked}
              onClick={() => void cancel()}
            >
              {t('停止生成', 'Stop generation')}
            </button>
          )}
          {answer && insufficientEvidence(answer) && (
            <div className="inference-refusal" role="status">
              <p className="inference-refusal-label">{refusal.label}</p>
              <p>{refusal.body}</p>
              <ul>
                {refusal.steps.map((step) => (
                  <li key={step}>{step}</li>
                ))}
              </ul>
            </div>
          )}
          {showAnswerText && (
            <div
              className="inference-answer-text"
              style={preparedPreview.value ? { whiteSpace: 'normal' } : undefined}
            >
              {preparedPreview.value ? (
                <StandaloneDocumentContent
                  document={preparedPreview.value.document}
                  identity={identity}
                  renderText={renderCitations}
                />
              ) : (
                renderCitations(answer.text)
              )}
            </div>
          )}
          <h3>{t('本次证据', 'Evidence for this answer')}</h3>
          {answer.evidence.map((item) => (
            <article className="inference-result" key={item.citation_id}>
              <button className="text-button" onClick={() => void open(item.citation_id)}>
                [{item.citation_id}] {item.evidence.title}
              </button>
              <p className="inference-excerpt">{item.evidence.text}</p>
              {item.context && (
                <p className="inference-context-note">
                  {t(
                    '模型同时读取了该片段所在的完整段落（按需扩展父级上下文）',
                    'The model also read the surrounding block (on-demand parent context).',
                  )}
                </p>
              )}
            </article>
          ))}
          {(answer.status === 'completed' || answer.status === 'no_evidence') && (
            <>
              {feedbackMessage && <p role="status">{feedbackMessage}</p>}
              {answer.feedback && (
                <>
                  <p>
                    {t('已记录反馈：', 'Saved feedback: ')}
                    {answer.feedback.rating > 0
                      ? t('有帮助', 'Helpful')
                      : answer.feedback.rating < 0
                        ? t('需要改进', 'Needs improvement')
                        : t('中立', 'Neutral')}
                    {answer.feedback.updated_at &&
                      ` · ${displayDate(answer.feedback.updated_at, language)}`}
                  </p>
                  {answer.feedback.comment && (
                    <p className="inference-excerpt">{answer.feedback.comment}</p>
                  )}
                  <p className="muted">
                    {t(
                      `本次反馈关联到提问「${answer.request.search.query}」、引用片段 ${
                        answer.cited_ids.length > 0
                          ? answer.cited_ids.map((id) => `[${id}]`).join('、')
                          : '（无引用片段）'
                      }，以及本次冻结的 ${answer.evidence.length} 条证据片段；检索参数与证据快照按该次问答保留。`,
                      `This feedback is attached to the question “${answer.request.search.query}”, to the cited fragments ${
                        answer.cited_ids.length > 0
                          ? answer.cited_ids.map((id) => `[${id}]`).join(', ')
                          : '(no cited fragment)'
                      }, and to the ${answer.evidence.length} evidence fragments frozen with that answer; the retrieval parameters and the evidence snapshot stay with it.`,
                    )}
                  </p>
                </>
              )}
              <label className="inference-field">
                {t(
                  '反馈备注（可选，随该次问答保存）',
                  'Feedback note (optional, kept with this answer)',
                )}
                <textarea
                  rows={2}
                  maxLength={8192}
                  value={feedbackComment}
                  disabled={actions.locked}
                  placeholder={t(
                    '例如：漏掉了哪个片段、答案哪里不对',
                    'For example: which fragment was missed, or what the answer got wrong',
                  )}
                  onChange={(event) => setFeedbackComment(event.target.value)}
                />
              </label>
              <div className="inference-controls">
                <button
                  className="secondary-button"
                  disabled={actions.locked}
                  aria-pressed={answer.feedback?.rating === 1}
                  onClick={() => void feedback(1)}
                >
                  {t('有帮助', 'Helpful')}
                </button>
                <button
                  className="secondary-button"
                  disabled={actions.locked}
                  aria-pressed={answer.feedback?.rating === -1}
                  onClick={() => void feedback(-1)}
                >
                  {t('需要改进', 'Needs improvement')}
                </button>
              </div>
              <details>
                <summary>{t('保存到笔记 · 先审阅提案', 'Save to a note · Review first')}</summary>
                <p>
                  {t(
                    '先创建完整提案并审阅。新笔记只在明确应用时创建，已有笔记也不会提前改写。',
                    'Create and review the full proposal first. New notes are created only on explicit application; existing notes remain unchanged until then.',
                  )}
                </p>
                <div className="inference-controls">
                  <div className="choice-field">
                    {t('保存方式', 'Destination')}
                    <Select
                      aria-label={t('保存方式', 'Destination')}
                      value={newNote ? 'new' : 'existing'}
                      disabled={preparing || actions.locked}
                      onValueChange={(nextValue) => setNewNote(nextValue === 'new')}
                    >
                      <option value="new">{t('新建笔记', 'New note')}</option>
                      <option value="existing">
                        {t('追加到已有笔记', 'Append to existing note')}
                      </option>
                    </Select>
                  </div>
                  {newNote ? (
                    <label>
                      {t('新笔记标题', 'New note title')}
                      <input
                        value={newTitle}
                        disabled={preparing || actions.locked}
                        placeholder={answer.request.search.query}
                        onChange={(e) => setNewTitle(e.target.value)}
                      />
                    </label>
                  ) : (
                    <>
                      <label>
                        {t('笔记标题', 'Note title')}
                        <input
                          value={noteQuery}
                          disabled={preparing || actions.locked}
                          onChange={(e) => {
                            setNoteQuery(e.target.value);
                            setNoteId('');
                            noteSequence.current++;
                            setNotes([]);
                            setNotesCursor(null);
                          }}
                        />
                      </label>
                      <button
                        className="secondary-button"
                        disabled={preparing || actions.locked}
                        onClick={() => void findNotes()}
                      >
                        {t('查找笔记', 'Find note')}
                      </button>
                      <Select
                        aria-label={t('目标笔记', 'Target note')}
                        value={noteId}
                        disabled={preparing || actions.locked}
                        onValueChange={(nextValue) => setNoteId(nextValue)}
                      >
                        <option value="">{t('选择笔记', 'Choose note')}</option>
                        {notes.map((note) => (
                          <option key={note.id} value={note.id}>
                            {note.title || t('未命名', 'Untitled')}
                          </option>
                        ))}
                      </Select>
                      {notesCursor && (
                        <button
                          disabled={preparing || actions.locked}
                          onClick={() => void findNotes(notesCursor)}
                        >
                          {t('更多笔记', 'More notes')}
                        </button>
                      )}
                    </>
                  )}
                  <button
                    className="primary-button"
                    disabled={
                      actions.locked ||
                      (!newNote && !noteId) ||
                      preparing ||
                      !answer.evidence_current
                    }
                    onClick={() => void proposal()}
                  >
                    {preparing
                      ? t('准备提案…', 'Preparing…')
                      : t('创建提案并审阅', 'Create and review proposal')}
                  </button>
                </div>
                {!!preparedPreview.value?.preserved.length && (
                  <p role="status">
                    {t(
                      '部分语法会以原文保留，请在提案中核对。外部图片不会自动下载。',
                      'Some syntax is retained as source. Check it in the proposal. External images are not downloaded.',
                    )}
                  </p>
                )}
                {preparedPreview.error && <p role="alert">{preparedPreview.error}</p>}
                {preparing && !actions.locked && (
                  <button
                    type="button"
                    onClick={() => {
                      preparation.current?.abort();
                      setError(t('已取消准备。', 'Preparation canceled.'));
                    }}
                  >
                    {t('取消准备', 'Cancel preparation')}
                  </button>
                )}
              </details>
            </>
          )}
        </section>
      )}
    </PageFrame>
  );
}
