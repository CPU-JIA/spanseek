import { Select } from '../ui/Select';
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { ChatMarkdown } from './ChatMarkdown';
import {
  appendChatConversations,
  chatPageSeed,
  canSaveChatReply,
  prependChatMessages,
  type ChatStreamSeed,
} from './chat-history';
import {
  ArrowUp,
  MessageCircle,
  PanelLeft,
  PanelLeftClose,
  Plus,
  Settings2,
  Sparkles,
  Square,
} from 'lucide-react';
import type {
  ChatConversation,
  ChatConversationMutation,
  ChatConversationCursor,
  ChatMessage,
  ChatPurpose,
  ChatRunMutation,
  ChatRunOptions,
  GenerationProfile,
  SearchEvidence,
  SearchScope,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { emptyScope, ScopePicker } from './SearchPage';
import { useInferenceAction } from './useInferenceAction';
import { useChatStream } from './useChatStream';
import { chatActive } from './chat-stream';
import { inferenceErrorMessage } from './inference-errors';
import { PageFrame } from '../layout/PageFrame';
import { ChatProposalPanel } from './ChatProposalPanel';
import { ChatComposerPopover, ChatMaterialChips, ChatMaterials } from './ChatMaterials';
import { ChannelModelPicker } from './ChannelModelPicker';
import { shouldSendChat } from './chat-keyboard';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { defaultTimeoutSeconds } from './generation-timeouts.ts';
import './chat.css';
import { AiTrustNote } from './ai-trust';
import { AiUsagePanel } from './ai-usage';
import { chatUsageParts } from './ai-usage-format';

const defaultOptions: ChatRunOptions = {
  max_model_calls: 8,
  max_tool_calls: 16,
  max_output_tokens: 8192,
  total_timeout_seconds: defaultTimeoutSeconds(null, 'chat'),
};

const runOptionFields = [
  {
    key: 'max_output_tokens',
    label: ['每次输出 token 上限', 'Output tokens per request'],
    min: 1,
    max: 32768,
  },
  {
    key: 'max_model_calls',
    label: ['模型请求次数', 'Model requests'],
    min: 1,
    max: 32,
  },
  {
    key: 'max_tool_calls',
    label: ['工具调用次数', 'Tool calls'],
    min: 0,
    max: 64,
  },
  {
    key: 'total_timeout_seconds',
    label: ['总时限（秒）', 'Total seconds'],
    min: 1,
    max: 3600,
  },
] as const;

export function ChatPage({
  identity,
  onOpenModels,
  onOpenProposal,
  onStrict,
  initialScope,
}: {
  identity: WorkspaceIdentity;
  onOpenModels: () => void;
  onOpenProposal: (id: string) => void;
  onStrict: () => void;
  initialScope?: SearchScope;
}) {
  const { t, language } = useI18n();
  const [conversations, setConversations] = useState<ChatConversation[]>([]);
  const [selected, setSelected] = usePageNavigationState<string | null>('chat.selected', null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const historyVisible = historyOpen;
  const historyId = useId();
  const historyToggleRef = useRef<HTMLButtonElement>(null);
  const focusHistoryToggle = useRef(false);
  const [conversation, setConversation] = useState<ChatConversation | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamSeed, setStreamSeed] = useState<ChatStreamSeed | null>(null);
  const [beforeMessage, setBeforeMessage] = useState<number | null>(null);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  const olderLock = useRef(false);
  const [historyCursor, setHistoryCursor] = useState<ChatConversationCursor | null>(null);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const historySequence = useRef(0);
  const historyLock = useRef(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [earlierError, setEarlierError] = useState<string | null>(null);
  const [conversationError, setConversationError] = useState<string | null>(null);
  const modelIdentity = `${identity.local_instance_id}/${identity.workspace_id}`;
  const [modelSelection, setModelSelection] = useState<{
    identity: string;
    profile: GenerationProfile | null;
  } | null>(null);
  const profile = modelSelection?.identity === modelIdentity ? modelSelection.profile : null;
  const [text, setText] = useState('');
  const [proposalMessage, setProposalMessage] = useState<string | null>(null);
  const [purpose, setPurpose] = useState<ChatPurpose>('chat');
  const [options, setOptions] = useState(defaultOptions);
  // The total budget is shown at the provider default until the user edits it,
  // so switching between a local and a remote model never shows a stale number.
  const timeoutTouched = useRef(false);
  useEffect(() => {
    if (timeoutTouched.current) return;
    const next = defaultTimeoutSeconds(profile?.provider, 'chat');
    setOptions((current) =>
      current.total_timeout_seconds === next
        ? current
        : { ...current, total_timeout_seconds: next },
    );
  }, [profile?.provider]);
  const [tools, setTools] = useState(false);
  const [scope, setScope] = useState(initialScope ?? emptyScope());
  const [materials, setMaterials] = useState<SearchEvidence[]>([]);
  const [loading, setLoading] = useState(false);
  const showStart = !messages.length && !loading && !conversationError;
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const actions = useInferenceAction(identity, 'chat');
  const last = messages.at(-1);
  const statusLabels = {
    queued: t('等待生成', 'Queued'),
    running: t('正在思考与生成…', 'Thinking and writing…'),
    completed: t('回答完成', 'Response complete'),
    canceled: t('已停止，保留已生成内容', 'Stopped; generated text is retained'),
    interrupted: t('运行中断，保留已生成内容', 'Interrupted; generated text is retained'),
    timed_out: t('本次生成超时，保留已生成内容', 'Timed out; generated text is retained'),
    failed: t('生成未完成', 'Response failed'),
    truncated: t('已达到运行上限，回答尚未完成', 'Run limit reached; response is incomplete'),
    refused: t('模型未能回答此请求', 'The model declined this request'),
    model_unconfigured: t('请先配置模型', 'Configure a model first'),
  };
  const {
    stream,
    run,
    progress,
    error: streamError,
  } = useChatStream(identity, last?.run_id ?? null, streamSeed);
  const active = !!last && (!run || run.id !== last.run_id || chatActive(stream.status));
  const requestSequence = useRef(0);
  const submitLock = useRef(false);
  const [submitting, setSubmitting] = useState(false);
  const transcriptRef = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);
  const prependAnchor = useRef<{ id: string; offset: number } | null>(null);
  const [awayFromLatest, setAwayFromLatest] = useState(false);
  useLayoutEffect(() => {
    if (!historyVisible && focusHistoryToggle.current) {
      focusHistoryToggle.current = false;
      historyToggleRef.current?.focus();
    }
  }, [historyVisible, selected]);
  useLayoutEffect(() => {
    followLatest.current = true;
    prependAnchor.current = null;
    setAwayFromLatest(false);
    setProposalMessage(null);
  }, [selected]);
  useLayoutEffect(() => {
    const transcript = transcriptRef.current;
    const anchor = prependAnchor.current;
    if (transcript && anchor) {
      const node = transcript.querySelector<HTMLElement>(`[data-message-id="${anchor.id}"]`);
      if (node) transcript.scrollTop += node.getBoundingClientRect().top - anchor.offset;
      prependAnchor.current = null;
    } else if (transcript && followLatest.current) transcript.scrollTop = transcript.scrollHeight;
  }, [stream.seq, messages, loading]);
  useEffect(() => {
    const transcript = transcriptRef.current;
    if (!transcript) return;
    const observer = new ResizeObserver(() => {
      if (followLatest.current) transcript.scrollTop = transcript.scrollHeight;
    });
    observer.observe(transcript);
    return () => observer.disconnect();
  }, []);
  const draftKey = `pkms-chat-draft/${identity.local_instance_id}/${identity.workspace_id}/${selected ?? 'new'}`;
  const draftLoaded = useRef('');
  useEffect(() => {
    setText('');
    setMaterials([]);
    draftLoaded.current = '';
    try {
      setText(localStorage.getItem(draftKey) ?? '');
      const saved: unknown = JSON.parse(localStorage.getItem(`${draftKey}/materials`) ?? '[]');
      if (
        !Array.isArray(saved) ||
        saved.some(
          (item) =>
            !item || typeof item.id !== 'string' || typeof item.text !== 'string' || !item.target,
        )
      )
        throw new Error(
          t(
            '资料草稿无法读取，请重新选择。',
            'The material draft could not be read. Select the material again.',
          ),
        );
      setMaterials(saved as SearchEvidence[]);
      draftLoaded.current = draftKey;
    } catch (e) {
      setError(message(e));
    }
  }, [draftKey]);
  const updateText = (value: string) => {
    setText(value);
    try {
      if (draftLoaded.current === draftKey) localStorage.setItem(draftKey, value);
    } catch (e) {
      setError(message(e));
    }
  };
  const updateMaterials = (value: SearchEvidence[]) => {
    setMaterials(value);
    try {
      localStorage.setItem(`${draftKey}/materials`, JSON.stringify(value));
    } catch (e) {
      setError(message(e));
    }
  };
  useEffect(() => {
    const sequence = ++historySequence.current;
    historyLock.current = true;
    setLoadingHistory(true);
    setHistoryError(null);
    void call(
      { kind: 'chat_conversations_page', payload: { before: null, limit: 40 } },
      'chat_conversation_page',
      identity,
    )
      .then((page) => {
        if (sequence !== historySequence.current) return;
        setConversations(page.items);
        setHistoryCursor(page.next_cursor);
      })
      .catch((e) => {
        if (sequence === historySequence.current) setHistoryError(message(e));
      })
      .finally(() => {
        if (sequence === historySequence.current) {
          historyLock.current = false;
          setLoadingHistory(false);
        }
      });
    return () => {
      ++historySequence.current;
    };
  }, [identity.local_instance_id, identity.workspace_id, reload]);
  async function loadMoreConversations() {
    if (!historyCursor || historyLock.current) return;
    const sequence = historySequence.current;
    historyLock.current = true;
    setLoadingHistory(true);
    setHistoryError(null);
    try {
      const page = await call(
        { kind: 'chat_conversations_page', payload: { before: historyCursor, limit: 40 } },
        'chat_conversation_page',
        identity,
      );
      if (sequence !== historySequence.current) return;
      setConversations((current) => appendChatConversations(current, page.items));
      setHistoryCursor(page.next_cursor);
    } catch (e) {
      if (sequence === historySequence.current) setHistoryError(message(e));
    } finally {
      if (sequence === historySequence.current) {
        historyLock.current = false;
        setLoadingHistory(false);
      }
    }
  }
  useEffect(() => {
    const sequence = ++requestSequence.current;
    setMessages([]);
    setConversation(null);
    setStreamSeed(null);
    setBeforeMessage(null);
    setEarlierError(null);
    setLoadingEarlier(false);
    olderLock.current = false;
    prependAnchor.current = null;
    setConversationError(null);
    if (!selected) {
      setLoading(false);
      return;
    }
    setLoading(true);
    void call(
      {
        kind: 'chat_messages_page',
        payload: { conversation_id: selected, before_ordinal: null, limit: 40 },
      },
      'chat_message_page',
      identity,
    )
      .then((page) => {
        if (sequence !== requestSequence.current) return;
        const seed = chatPageSeed(page);
        setConversation(page.conversation);
        setMessages(page.items);
        setStreamSeed(seed);
        setBeforeMessage(page.next_before);
      })
      .catch((e) => {
        if (sequence === requestSequence.current) setConversationError(message(e));
      })
      .finally(() => {
        if (sequence === requestSequence.current) setLoading(false);
      });
    return () => {
      ++requestSequence.current;
    };
  }, [selected, reload, identity.local_instance_id, identity.workspace_id]);
  async function loadEarlier() {
    if (!selected || beforeMessage === null || olderLock.current || loading) return;
    const sequence = requestSequence.current;
    olderLock.current = true;
    setLoadingEarlier(true);
    setEarlierError(null);
    try {
      const page = await call(
        {
          kind: 'chat_messages_page',
          payload: { conversation_id: selected, before_ordinal: beforeMessage, limit: 40 },
        },
        'chat_message_page',
        identity,
      );
      if (sequence !== requestSequence.current) return;
      const combined = prependChatMessages(messages, page.items);
      const first = transcriptRef.current?.querySelector<HTMLElement>('[data-message-id]');
      if (first)
        prependAnchor.current = {
          id: first.dataset.messageId!,
          offset: first.getBoundingClientRect().top,
        };
      followLatest.current = false;
      setMessages(combined);
      setBeforeMessage(page.next_before);
    } catch (e) {
      if (sequence === requestSequence.current) setEarlierError(message(e));
    } finally {
      if (sequence === requestSequence.current) {
        olderLock.current = false;
        setLoadingEarlier(false);
      }
    }
  }
  async function send() {
    if (selected && conversation?.id !== selected) return;
    if (submitLock.current || actions.locked || active || loading || !text.trim()) return;
    for (const { key, label, min, max } of runOptionFields) {
      const value = options[key];
      if (!Number.isInteger(value) || value < min || value > max) {
        setError(
          t(
            `“${label[0]}”必须为 ${min} 到 ${max} 的整数。请打开“范围与运行选项”修改后再发送。`,
            `“${label[1]}” must be an integer from ${min} to ${max}. Open “Scope and run options” to update it before sending.`,
          ),
        );
        return;
      }
    }
    if (!profile || profile.provider === 'ollama') {
      setError(t('请配置远程对话模型。', 'Configure a remote chat model.'));
      return;
    }
    if (options.max_output_tokens >= profile.context_tokens) {
      setError(
        t(
          '输出预算必须小于模型上下文容量，并为输入与历史留出空间。',
          'Output must fit within context capacity, leaving room for input and history.',
        ),
      );
      return;
    }
    submitLock.current = true;
    setSubmitting(true);
    setError(null);
    try {
      let target = conversation;
      if (!target) {
        const result = await actions.execute(
          {
            kind: 'chat_conversations_create',
            payload: {
              command_id: crypto.randomUUID(),
              request: { title: text.trim().split('\n')[0].slice(0, 80) },
            },
          },
          'chat_conversation_mutation',
        );
        if (!result) return;
        target = result.conversation;
        setConversation(target);
      }
      const result = await actions.execute(
        {
          kind: 'chat_runs_start',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              conversation_id: target.id,
              base_row_version: target.row_version,
              text,
              purpose,
              materials,
              scope,
              use_knowledge_tools: tools,
              options,
            },
          },
        },
        'chat_run_mutation',
      );
      if (result) {
        followLatest.current = true;
        setAwayFromLatest(false);
        prependAnchor.current = null;
        updateText('');
        updateMaterials([]);
        setSelected(result.conversation.id);
        setReload((n) => n + 1);
      }
    } catch (e) {
      setError(message(e));
    } finally {
      submitLock.current = false;
      setSubmitting(false);
    }
  }
  async function retry() {
    const expected = actions.pending?.expected;
    const result = await actions.retry();
    if (!result) return;
    if (expected === 'chat_conversation_mutation' || expected === 'chat_run_mutation') {
      const restored = result as ChatConversationMutation | ChatRunMutation;
      setSelected(restored.conversation.id);
      if (expected === 'chat_run_mutation') {
        followLatest.current = true;
        setAwayFromLatest(false);
        prependAnchor.current = null;
        updateText('');
        updateMaterials([]);
      }
    }
    setReload((n) => n + 1);
  }
  async function cancel() {
    if (!last) return;
    const result = await actions.execute(
      {
        kind: 'chat_runs_cancel',
        payload: { command_id: crypto.randomUUID(), run_id: last.run_id },
      },
      'chat_run_mutation',
    );
    if (result) setReload((n) => n + 1);
  }
  function startNewConversation() {
    focusHistoryToggle.current = true;
    setHistoryOpen(false);
    setSelected(null);
    setConversation(null);
    setMessages([]);
    setStreamSeed(null);
  }
  return (
    <PageFrame
      className="chat-page"
      title={conversation?.title ?? t('AI 对话', 'AI chat')}
      description={t(
        '自由对话用于探索想法；需要可核对的引用时切换到依证问答。',
        'Use chat to explore ideas; switch to evidence Q&A when you need verifiable citations.',
      )}
      actions={
        <button className="secondary-button" onClick={onStrict}>
          {t('依证问答', 'Evidence Q&A')}
        </button>
      }
    >
      <section className="chat-workspace">
        <aside
          id={historyId}
          className="chat-history"
          hidden={!historyVisible}
          aria-label={t('对话历史', 'Conversations')}
        >
          <button
            className="chat-new"
            onClick={startNewConversation}
            disabled={actions.busy || submitting}
          >
            <Plus size={16} />
            {t('新对话', 'New chat')}
          </button>
          {conversations.map((item) => (
            <button
              key={item.id}
              disabled={actions.busy || submitting}
              aria-current={item.id === selected ? 'page' : undefined}
              onClick={() => {
                focusHistoryToggle.current = true;
                setHistoryOpen(false);
                setSelected(item.id);
              }}
            >
              <MessageCircle size={15} />
              <span>{item.title}</span>
            </button>
          ))}
          {historyError && (
            <p className="chat-history-error" role="alert">
              {historyError}
            </p>
          )}
          {historyCursor && (
            <button
              disabled={loadingHistory || submitting}
              onClick={() => void loadMoreConversations()}
            >
              {loadingHistory ? t('正在读取…', 'Loading…') : t('更多对话', 'More conversations')}
            </button>
          )}
          {loadingHistory && !historyCursor && (
            <p role="status">{t('正在读取对话列表…', 'Loading conversations…')}</p>
          )}
          {historyError && !historyCursor && (
            <button onClick={() => setReload((n) => n + 1)}>{t('重新读取', 'Reload')}</button>
          )}
        </aside>
        <div className={`chat-main${showStart ? ' chat-main-start' : ''}`}>
          <header className="chat-header">
            <button
              ref={historyToggleRef}
              className="chat-history-toggle"
              aria-expanded={historyVisible}
              aria-controls={historyId}
              aria-label={
                historyVisible
                  ? t('收起对话历史', 'Hide conversation history')
                  : t('显示所有对话', 'Show all conversations')
              }
              onClick={() => setHistoryOpen((value) => !value)}
            >
              {historyVisible ? (
                <PanelLeftClose size={16} aria-hidden="true" />
              ) : (
                <PanelLeft size={16} aria-hidden="true" />
              )}
              <span>{t('所有对话', 'All conversations')}</span>
            </button>
            {selected && (
              <button
                className="chat-header-new"
                onClick={startNewConversation}
                disabled={actions.busy || submitting}
                aria-label={t('新对话', 'New chat')}
              >
                <Plus size={16} aria-hidden="true" />
                <span>{t('新对话', 'New chat')}</span>
              </button>
            )}
          </header>
          <div
            className="chat-transcript"
            ref={transcriptRef}
            aria-busy={loading}
            onScroll={(event) => {
              const node = event.currentTarget;
              const nearBottom = node.scrollHeight - node.scrollTop - node.clientHeight <= 64;
              followLatest.current = nearBottom;
              setAwayFromLatest(!nearBottom);
            }}
          >
            {showStart && (
              <div className="chat-empty">
                <span className="chat-empty-icon" aria-hidden="true">
                  <Sparkles size={24} />
                </span>
                <h2>{t('开启新的对话', 'Start a new session')}</h2>
                <p>
                  {t(
                    '讨论想法、总结资料、润色文字，或一起检查内容中的问题。',
                    'Discuss ideas, summarize, rewrite, or review material together.',
                  )}
                </p>
                <AiTrustNote mode="chat" compact />
                <AiUsagePanel identity={identity} />
              </div>
            )}
            {loading && <p role="status">{t('正在读取对话…', 'Loading conversation…')}</p>}
            {beforeMessage !== null && (
              <button
                className="chat-load-earlier"
                disabled={loadingEarlier}
                onClick={() => void loadEarlier()}
              >
                {loadingEarlier
                  ? t('正在读取更早消息…', 'Loading earlier messages…')
                  : t('加载更早消息', 'Load earlier messages')}
              </button>
            )}
            {earlierError && (
              <p className="chat-history-error" role="alert">
                {earlierError}
              </p>
            )}
            {messages.map((item) => (
              <article
                key={item.id}
                data-message-id={item.id}
                className={`chat-message chat-message-${item.role}`}
              >
                <span
                  className="chat-role"
                  title={
                    item.role === 'assistant'
                      ? t(
                          'AI 输出可能有误；保存前请审阅提案。',
                          'AI output can be wrong; review the proposal before saving.',
                        )
                      : undefined
                  }
                >
                  {item.role === 'user' ? t('你', 'You') : 'AI'}
                </span>
                <ChatMarkdown
                  identity={identity}
                  renderRich={item.role === 'user' || canSaveChatReply(item, last?.id, run, stream)}
                  text={
                    item.id === last?.id && item.role === 'assistant' && run?.id === item.run_id
                      ? stream.text
                      : item.content
                  }
                />
                {item.role === 'assistant' && item.id !== last?.id && (
                  <p className="chat-history-status" data-run-status={item.run_status}>
                    {statusLabels[item.run_status]}
                    {item.run_error_code
                      ? ` · ${inferenceErrorMessage(item.run_error_code, t)}`
                      : ''}
                  </p>
                )}
                {item.role === 'assistant' &&
                  (item.content || (item.id === last?.id && stream.text)) && (
                    <div className="chat-message-actions">
                      <button
                        type="button"
                        disabled={!canSaveChatReply(item, last?.id, run, stream)}
                        onClick={() =>
                          setProposalMessage(proposalMessage === item.id ? null : item.id)
                        }
                      >
                        {t('保存到笔记…', 'Save to note…')}
                      </button>
                      {proposalMessage === item.id && (
                        <ChatProposalPanel
                          key={item.id}
                          identity={identity}
                          runId={item.run_id}
                          text={
                            item.id === last?.id && run?.id === item.run_id
                              ? stream.text
                              : item.content
                          }
                          onOpenProposal={onOpenProposal}
                        />
                      )}
                    </div>
                  )}
              </article>
            ))}
            {last && run?.id === last.run_id && (
              <p className="chat-run-status" role="status">
                {progress?.run_id === run.id && chatActive(stream.status)
                  ? {
                      generating: t('正在生成回答…', 'Generating…'),
                      searching: t('正在检索知识库…', 'Searching knowledge…'),
                      reading: t('正在阅读资料片段…', 'Reading excerpts…'),
                    }[progress.stage]
                  : statusLabels[stream.status]}
                {progress?.run_id === run.id && (
                  <span className="chat-progress-counts">
                    {t(
                      ` · 模型请求 ${progress.model_calls} · 工具调用 ${progress.tool_calls}`,
                      ` · Model requests ${progress.model_calls} · Tool calls ${progress.tool_calls}`,
                    )}
                  </span>
                )}
                {run.error_code ? ` · ${inferenceErrorMessage(run.error_code, t)}` : ''}
                {last?.run_id === run.id && (
                  <span className="chat-run-usage">
                    {chatUsageParts(run.usage).total !== null
                      ? t(
                          ` · 本次输入 ${chatUsageParts(run.usage).input?.toLocaleString(language) ?? '—'} / 输出 ${chatUsageParts(run.usage).output?.toLocaleString(language) ?? '—'} Token`,
                          ` · This run: ${chatUsageParts(run.usage).input?.toLocaleString(language) ?? '—'} input / ${chatUsageParts(run.usage).output?.toLocaleString(language) ?? '—'} output tokens`,
                        )
                      : ['completed', 'truncated', 'refused'].includes(run.status)
                        ? t(
                            ' · 本次运行未返回 Token 用量',
                            ' · No token usage returned for this run',
                          )
                        : ''}
                  </span>
                )}
              </p>
            )}
          </div>
          {awayFromLatest && (
            <button
              className="chat-jump-latest"
              type="button"
              onClick={() => {
                followLatest.current = true;
                setAwayFromLatest(false);
                const transcript = transcriptRef.current;
                if (transcript) transcript.scrollTop = transcript.scrollHeight;
              }}
            >
              {t('回到最新消息', 'Jump to latest')}
            </button>
          )}
          {(error || conversationError || actions.error || streamError) && (
            <p className="chat-error" role="alert">
              {error || conversationError || actions.error || streamError}
            </p>
          )}
          {conversationError && selected && !conversation && (
            <button onClick={() => setReload((n) => n + 1)}>
              {t('重新读取对话', 'Reload conversation')}
            </button>
          )}
          {actions.pending && (
            <button disabled={actions.busy || submitting} onClick={() => void retry()}>
              {t('确认上次操作结果', 'Recover previous operation')}
            </button>
          )}
          <form
            className="chat-composer"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              if (event.currentTarget.querySelector('dialog[open]')) return;
              void send();
            }}
          >
            <ChatMaterialChips
              value={materials}
              onChange={updateMaterials}
              disabled={actions.locked || submitting || loading}
            />
            <textarea
              aria-label={t('消息', 'Message')}
              aria-describedby={`${historyId}-message-hint`}
              rows={1}
              disabled={actions.locked || submitting || loading}
              placeholder={t('向 AI 提问…', 'Ask AI…')}
              value={text}
              onChange={(event) => updateText(event.target.value)}
              onKeyDown={(event) => {
                if (shouldSendChat(event.nativeEvent)) {
                  event.preventDefault();
                  void send();
                }
              }}
            />
            <div className="chat-composer-actions">
              <ChatMaterials
                identity={identity}
                scope={scope}
                value={materials}
                onChange={updateMaterials}
                disabled={actions.locked || submitting || loading}
                triggerLabel={showStart ? t('添加资料', 'Add material') : undefined}
              />
              <ChatComposerPopover
                label={t('范围与运行选项', 'Scope and run options')}
                icon={<Settings2 size={18} aria-hidden="true" />}
              >
                <div className="chat-run-controls">
                  <div className="choice-field">
                    {t('对话用途', 'Purpose')}
                    <Select
                      aria-label={t('对话用途', 'Purpose')}
                      value={purpose}
                      onValueChange={(nextValue) => setPurpose(nextValue as ChatPurpose)}
                    >
                      <option value="chat">{t('对话', 'Chat')}</option>
                      <option value="summarize">{t('总结', 'Summarize')}</option>
                      <option value="rewrite">{t('润色', 'Rewrite')}</option>
                      <option value="research">{t('研究', 'Research')}</option>
                      <option value="review">{t('审阅与纠错', 'Review')}</option>
                    </Select>
                  </div>
                  <label className="chat-tools-option">
                    <input
                      type="checkbox"
                      checked={tools}
                      onChange={(event) => setTools(event.target.checked)}
                    />
                    {t('检索知识库', 'Search knowledge')}
                  </label>
                </div>
                <ScopePicker
                  identity={identity}
                  value={scope}
                  onChange={(next) => {
                    setScope(next);
                    if (materials.length) {
                      updateMaterials([]);
                      setError(
                        t(
                          '检索范围已改变，请在新范围内重新选择资料。',
                          'The search scope changed. Select material within the new scope.',
                        ),
                      );
                    }
                  }}
                />
                <div className="chat-options">
                  {runOptionFields.map(({ key, label, min, max }) => (
                    <label key={key}>
                      {t(label[0], label[1])}
                      <input
                        type="number"
                        min={min}
                        max={max}
                        step={1}
                        value={options[key]}
                        onChange={(event) => {
                          if (key === 'total_timeout_seconds') timeoutTouched.current = true;
                          setOptions({ ...options, [key]: Number(event.target.value) });
                        }}
                      />
                    </label>
                  ))}
                </div>
              </ChatComposerPopover>
              <div className="chat-composer-trailing">
                <div className="chat-model-slot">
                  <ChannelModelPicker
                    identity={identity}
                    scope="chat"
                    purpose="chat"
                    disabled={actions.locked || submitting || loading || active}
                    onChanged={(settings) =>
                      setModelSelection({ identity: modelIdentity, profile: settings.profile })
                    }
                    onManage={onOpenModels}
                  />
                </div>
                {active ? (
                  <button
                    className="chat-composer-send"
                    type="button"
                    disabled={actions.locked}
                    onClick={() => void cancel()}
                    aria-label={t('停止生成', 'Stop')}
                  >
                    <Square size={16} aria-hidden="true" />
                  </button>
                ) : (
                  <button
                    className="chat-composer-send"
                    type="submit"
                    disabled={actions.locked || submitting || loading || !text.trim()}
                    aria-label={t('发送', 'Send')}
                  >
                    <ArrowUp size={18} aria-hidden="true" />
                  </button>
                )}
              </div>
            </div>
            <p className="chat-composer-hint" id={`${historyId}-message-hint`}>
              {t('Ctrl + Enter 发送 · Enter 换行', 'Ctrl + Enter to send · Enter for a new line')}
            </p>
          </form>
        </div>
      </section>
    </PageFrame>
  );
}
