import { Select } from '../ui/Select';
import { useEffect, useRef, useState } from 'react';
import {
  ArrowUpRight,
  ChevronDown,
  History,
  ListFilter,
  Search,
  SlidersHorizontal,
  Sparkles,
  Trash2,
} from 'lucide-react';
import type {
  AnswerMutation,
  CollectionDetail,
  SearchEvidence,
  SearchMode,
  SearchQuery,
  SearchResults,
  SearchScope,
  RetrievalParams,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useInferenceAction } from './useInferenceAction';
import { inferenceErrorMessage } from './inference-errors';
import { TagScopePicker } from './TagScopePicker';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { PageFrame } from '../layout/PageFrame';
import './inference.css';
import {
  DEFAULT_RETRIEVAL_PARAMS,
  RetrievalParamsForm,
  saveRetrievalParams,
  useRetrievalSettings,
} from './retrieval-settings';
import { searchHighlightParts } from './search-highlight';
import {
  parseSearchHistory,
  recordSearchHistory,
  searchHistoryEntry,
  searchHistoryStorageKey,
  searchQueryFromHistory,
  serializeSearchHistory,
  type SearchHistoryEntry,
} from './search-history';
import { rerankSearchResults, type SearchResultOrder } from './search-rerank';

export const emptyScope = (): SearchScope => ({
  object_ids: [],
  kinds: [],
  tag_ids: [],
  collection_id: null,
  inbox_state: null,
  reading_status: null,
  source_format: null,
  archived: null,
});
export type OpenEvidence = (evidence: SearchEvidence) => Promise<void>;

export function SearchPage({
  identity,
  onOpenEvidence,
  onOpenAnswer,
  onOpenModels,
  initialScope,
}: {
  identity: WorkspaceIdentity;
  onOpenEvidence: OpenEvidence;
  onOpenAnswer: (id: string) => void;
  onOpenModels: () => void;
  initialScope?: SearchScope;
}) {
  const { t, language } = useI18n();
  const navigationScope = `search/${initialScope?.collection_id ?? 'all'}`;
  const [query, setQuery] = usePageNavigationState<SearchQuery>(`${navigationScope}/query`, {
    query: '',
    mode: 'hybrid',
    scope: initialScope ?? emptyScope(),
    limit: DEFAULT_RETRIEVAL_PARAMS.top_k,
    minimum_cosine: DEFAULT_RETRIEVAL_PARAMS.minimum_cosine ?? null,
  });
  const [result, setResult] = usePageNavigationState<{
    query: SearchQuery;
    data: SearchResults;
  } | null>(`${navigationScope}/result`, null);
  const [resultOrder, setResultOrder] = usePageNavigationState<SearchResultOrder>(
    `${navigationScope}/order`,
    'fusion',
  );
  const historyStorageKey = searchHistoryStorageKey(identity);
  const [history, setHistory] = useState<SearchHistoryEntry[]>(() => {
    try {
      return parseSearchHistory(window.localStorage.getItem(historyStorageKey));
    } catch {
      return [];
    }
  });
  const scopeSummary = describeScope(query.scope, t);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const live = useRef(true);
  const actions = useInferenceAction(identity, 'search-answer');
  const answerPreparing = useRef(false);
  const retrieval = useRetrievalSettings(identity);
  const [params, setParams] = useState<RetrievalParams>(DEFAULT_RETRIEVAL_PARAMS);
  const [paramsBusy, setParamsBusy] = useState(false);
  const [paramsMessage, setParamsMessage] = useState<string | null>(null);
  const [embedding, setEmbedding] = useState<{ model: string; dimensions: number } | null>(null);
  const seededParams = useRef(false);
  useEffect(() => {
    let live = true;
    void call({ kind: 'embedding_profile_get' }, 'embedding_profile', identity)
      .then((profile) => {
        if (live)
          setEmbedding(profile ? { model: profile.model, dimensions: profile.dimensions } : null);
      })
      .catch(() => {
        if (live) setEmbedding(null);
      });
    return () => {
      live = false;
    };
  }, [identity]);
  useEffect(() => {
    const loaded = retrieval.settings;
    if (!loaded) return;
    setParams(loaded.params);
    if (seededParams.current) return;
    seededParams.current = true;
    setQuery((old) => ({
      ...old,
      limit: loaded.params.top_k,
      minimum_cosine: loaded.params.minimum_cosine ?? null,
    }));
  }, [retrieval.settings]);
  function applyParams(next: RetrievalParams) {
    setParams(next);
    setParamsMessage(null);
    setQuery((old) => ({
      ...old,
      limit: next.top_k,
      minimum_cosine: next.minimum_cosine ?? null,
    }));
  }
  async function persistParams() {
    const loaded = retrieval.settings;
    if (!loaded) return;
    setParamsBusy(true);
    setParamsMessage(null);
    try {
      const saved = await saveRetrievalParams(identity, loaded.row_version, params);
      retrieval.setSettings(saved);
      setParamsMessage(t('已保存为默认检索参数。', 'Saved as the default retrieval parameters.'));
    } catch (failure) {
      setParamsMessage(message(failure));
    } finally {
      setParamsBusy(false);
    }
  }
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      request.current++;
    };
  }, []);
  useEffect(() => {
    try {
      window.localStorage.setItem(historyStorageKey, serializeSearchHistory(history));
    } catch {
      // Search history is local convenience only; an unavailable storage bucket
      // must never block searching.
    }
  }, [history, historyStorageKey]);
  async function search(
    frozen: SearchQuery = structuredClone(query),
    order: SearchResultOrder = resultOrder,
  ) {
    const normalized = { ...frozen, query: frozen.query.trim() };
    if (!normalized.query) return;
    const sequence = ++request.current;
    setBusy(true);
    setError(null);
    try {
      const data = await call(
        { kind: 'search_query', payload: normalized },
        'search_results',
        identity,
      );
      if (sequence === request.current) {
        setResult({ query: normalized, data });
        setHistory((previous) =>
          recordSearchHistory(
            previous,
            searchHistoryEntry({
              query: normalized,
              order,
              resultCount: data.items.length,
            }),
          ),
        );
      }
    } catch (e) {
      if (sequence === request.current) setError(message(e));
    } finally {
      if (sequence === request.current) setBusy(false);
    }
  }
  function recallHistory(entry: SearchHistoryEntry) {
    const next = searchQueryFromHistory(entry);
    setQuery(next);
    setResultOrder(entry.order);
    void search(next, entry.order);
  }
  async function answer(retry = false) {
    if (answerPreparing.current || (!retry && actions.locked)) return;
    answerPreparing.current = true;
    const frozenQuery = structuredClone(query);
    try {
      // Read only when accepting a new request; replay must keep its frozen budget.
      let maxOutputTokens: number | undefined;
      if (!retry) {
        try {
          const settings = await call(
            { kind: 'generation_settings_get' },
            'generation_settings',
            identity,
          );
          if (!live.current) return;
          maxOutputTokens = settings.profile?.provider === 'ollama' ? 384 : 8192;
          if (settings.profile && maxOutputTokens >= settings.profile.context_tokens) {
            setError(inferenceErrorMessage('answer_output_budget_invalid', t));
            return;
          }
        } catch (error) {
          if (live.current) setError(message(error));
          return;
        }
      }
      const value = retry
        ? await actions.retry<'answer_mutation'>()
        : await actions.execute(
            {
              kind: 'answers_start',
              payload: {
                command_id: crypto.randomUUID(),
                search: frozenQuery,
                max_output_tokens: maxOutputTokens,
              },
            },
            'answer_mutation',
          );
      if (value && live.current) onOpenAnswer((value as AnswerMutation).answer.id);
    } finally {
      answerPreparing.current = false;
    }
  }
  async function open(evidence: SearchEvidence) {
    try {
      await onOpenEvidence(evidence);
    } catch (e) {
      if (live.current) setError(message(e));
    }
  }
  const orderedItems = result ? rerankSearchResults(result.data.items, resultOrder) : [];
  const hasLexicalSignal = result?.data.items.some((item) => item.lexical_rank !== null) ?? false;
  const hasSemanticSignal =
    result?.data.items.some((item) => item.cosine_similarity !== null) ?? false;
  return (
    <PageFrame
      className="inference-page search-page-frame"
      title={t('搜索', 'Search')}
      description={t(
        '按词语或意思查找，并回到命中的原文。',
        'Find by words or meaning, then return to the source.',
      )}
      actions={
        <button className="text-button" onClick={onOpenModels}>
          {t('查看模型与索引状态', 'Model and index status')}
        </button>
      }
    >
      <form
        className="inference-search-form"
        onSubmit={(e) => {
          e.preventDefault();
          void search();
        }}
      >
        <label className="inference-query">
          <Search size={18} />
          <input
            aria-label={t('搜索内容', 'Search query')}
            value={query.query}
            onChange={(e) => setQuery((old) => ({ ...old, query: e.target.value }))}
            placeholder={t('输入问题、专名或关键词…', 'A question, name, or keyword…')}
            required
            maxLength={4000}
          />
        </label>
        <div className="inference-controls">
          <div className="choice-field">
            {t('检索方式', 'Search mode')}
            <Select
              aria-label={t('检索方式', 'Search mode')}
              value={query.mode}
              onValueChange={(nextValue) =>
                setQuery((old) => ({ ...old, mode: nextValue as SearchQuery['mode'] }))
              }
            >
              <option value="lexical">{t('关键词', 'Keyword')}</option>
              <option value="semantic">{t('语义', 'Semantic')}</option>
              <option value="hybrid">{t('混合', 'Hybrid')}</option>
            </Select>
          </div>
          <button className="primary-button" disabled={busy || !query.query.trim()}>
            {busy ? t('正在搜索…', 'Searching…') : t('搜索', 'Search')}
          </button>
          <button
            type="button"
            className="secondary-button"
            disabled={actions.locked || !query.query.trim()}
            onClick={() => void answer()}
          >
            <Sparkles size={15} />
            {t('在此范围内提问', 'Ask within this scope')}
          </button>
        </div>
        <details className="inference-scope-disclosure">
          <summary>
            <ListFilter size={16} aria-hidden="true" />
            <span className="inference-scope-label">{t('范围筛选', 'Scope filters')}</span>
            <span className="inference-scope-summary">{scopeSummary}</span>
            <ChevronDown size={14} className="inference-scope-chevron" aria-hidden="true" />
          </summary>
          <ScopePicker
            identity={identity}
            value={query.scope}
            onChange={(scope) => setQuery((old) => ({ ...old, scope }))}
          />
        </details>
        <details className="inference-params-disclosure">
          <summary>
            <SlidersHorizontal size={16} aria-hidden="true" />
            <span className="inference-scope-label">{t('检索参数', 'Retrieval parameters')}</span>
            <span className="inference-scope-summary">
              {t(
                `Top-K ${params.top_k} · 引用 ${params.citation_count} 条`,
                `Top-K ${params.top_k} · ${params.citation_count} citations`,
              )}
            </span>
            <ChevronDown size={14} className="inference-scope-chevron" aria-hidden="true" />
          </summary>
          <RetrievalParamsForm
            value={params}
            onChange={applyParams}
            disabled={paramsBusy}
            model={embedding?.model ?? null}
            dimensions={embedding?.dimensions ?? null}
          />
          <div className="inference-params-actions">
            <button
              type="button"
              className="secondary-button"
              disabled={paramsBusy || !retrieval.settings}
              onClick={() => void persistParams()}
            >
              {paramsBusy ? t('正在保存…', 'Saving…') : t('保存为默认', 'Save as default')}
            </button>
            {retrieval.error && <span role="alert">{retrieval.error}</span>}
            {paramsMessage && <span role="status">{paramsMessage}</span>}
          </div>
        </details>{' '}
      </form>
      {history.length > 0 && (
        <section className="inference-history" aria-label={t('最近搜索', 'Recent searches')}>
          <div className="inference-history-heading">
            <History size={15} aria-hidden="true" />
            <strong>{t('最近搜索', 'Recent searches')}</strong>
            <button
              type="button"
              className="text-button inference-history-clear"
              onClick={() => setHistory([])}
            >
              <Trash2 size={13} aria-hidden="true" />
              {t('清空', 'Clear')}
            </button>
          </div>
          <div className="inference-history-list">
            {history.slice(0, 8).map((entry) => (
              <button
                type="button"
                className="inference-history-chip"
                key={entry.id}
                onClick={() => recallHistory(entry)}
                aria-label={`${entry.query} · ${modeLabel(entry.mode, t)} · ${orderLabel(entry.order, t)} · ${historyCountLabel(entry, t)}`}
                title={`${entry.query} · ${modeLabel(entry.mode, t)} · ${orderLabel(entry.order, t)} · ${historyCountLabel(entry, t)}`}
              >
                <span>{entry.query}</span>
                <small>
                  {modeLabel(entry.mode, t)} · {historyCountLabel(entry, t)}
                </small>
              </button>
            ))}
          </div>
        </section>
      )}
      {(error || actions.error) && (
        <div className="inference-error" role="alert">
          {error || actions.error}
        </div>
      )}
      {(!actions.ready || actions.pending) && (
        <div className="inference-notice">
          <p>
            {t(
              '有请求需要读取或重新确认；将保留原请求编号与检索范围。',
              'A request needs reading or confirmation; its original ID and scope are preserved.',
            )}
          </p>
          <button
            className="secondary-button"
            disabled={actions.busy}
            onClick={() => void answer(true)}
          >
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      )}
      {result && (
        <div className="inference-results" aria-busy={busy}>
          <div className="inference-results-heading">
            <h2>{t(`“${result.query.query}”的检索结果`, `Results for “${result.query.query}”`)}</h2>
            {orderedItems.length > 1 && (
              <div className="choice-field inference-sort">
                <span>{t('结果排序', 'Result order')}</span>
                <Select
                  aria-label={t('结果排序', 'Result order')}
                  value={resultOrder}
                  onValueChange={(nextValue) => setResultOrder(nextValue as SearchResultOrder)}
                >
                  <option value="fusion">{t('融合', 'Fusion')}</option>
                  <option value="keyword" disabled={!hasLexicalSignal}>
                    {t('关键词优先', 'Keyword first')}
                  </option>
                  <option value="semantic" disabled={!hasSemanticSignal}>
                    {t('语义优先', 'Semantic first')}
                  </option>
                </Select>
              </div>
            )}
          </div>
          <p className="muted">
            {result.data.lexical_available
              ? t('本次关键词检索可用', 'Keyword search available for this query')
              : t('本次关键词检索不可用', 'Keyword search unavailable for this query')}{' '}
            ·{' '}
            {result.query.mode === 'lexical'
              ? t('本次未请求语义检索', 'Semantic search was not requested')
              : result.data.semantic_available
                ? t('本次语义检索可用', 'Semantic search available for this query')
                : t('本次语义检索不可用', 'Semantic search unavailable for this query')}
          </p>
          <p className="muted">
            {t(
              '结果来自本次提交的范围；修改筛选后请重新搜索。',
              'Results use the submitted scope; search again after changing filters.',
            )}
          </p>
          {orderedItems.length > 1 && (
            <p className="muted inference-sort-note">
              {t(
                '重排只调整本次返回的候选，不扩大检索范围。',
                'Re-ranking only changes the order of the returned candidates; it does not widen the search.',
              )}
              {resultOrder === 'semantic' && !hasSemanticSignal
                ? t(
                    ' 当前结果没有语义分数，已按融合/关键词信号回退。',
                    ' This result set has no semantic scores, so fusion/keyword signals are used instead.',
                  )
                : resultOrder === 'keyword' && !hasLexicalSignal
                  ? t(
                      ' 当前结果没有关键词名次，已按融合/语义信号回退。',
                      ' This result set has no keyword ranks, so fusion/semantic signals are used instead.',
                    )
                  : ''}
            </p>
          )}
          {!result.data.semantic_available && result.query.mode !== 'lexical' && (
            <div className="inference-notice">
              {t('语义检索未就绪：', 'Semantic search unavailable: ')}
              {result.data.unavailable_reason
                ? inferenceErrorMessage(result.data.unavailable_reason, t)
                : t('没有活动向量索引', 'No active vector index')}
              {result.data.lexical_available && result.query.mode === 'hybrid' && (
                <p>
                  {t(
                    '以下仅显示关键词结果，未扩大检索范围。',
                    'Only keyword results are shown within the same scope.',
                  )}
                </p>
              )}
            </div>
          )}
          {!result.data.items.length &&
            (result.data.semantic_available ||
              (result.query.mode !== 'semantic' && result.data.lexical_available)) && (
              <p className="inference-empty">
                {t(
                  '此范围内没有命中。可检查词语，或由你调整范围后再搜索。',
                  'No matches in this scope. Check the query or adjust the scope and search again.',
                )}
              </p>
            )}
          {orderedItems.map((item) => (
            <article className="inference-result" key={`${item.id}/${item.generation_id ?? ''}`}>
              <div className="inference-result-heading">
                <span className="inference-kind">
                  {item.target.kind === 'note' ? t('笔记', 'Note') : t('资料', 'Source')}
                  {item.target.page_index !== null ? ` · P${item.target.page_index + 1}` : ''}
                </span>
                <button className="text-button" onClick={() => void open(item)}>
                  {t('打开原文', 'Open source')}
                  <ArrowUpRight size={14} />
                </button>
              </div>
              <h3>
                {item.title ? (
                  <HighlightedText
                    text={item.title}
                    query={result.query.query}
                    language={language}
                  />
                ) : (
                  t('未命名', 'Untitled')
                )}
              </h3>
              <p className="inference-excerpt">
                <HighlightedText text={item.text} query={result.query.query} language={language} />
              </p>
              <details>
                <summary>{t('检索依据', 'Ranking details')}</summary>
                <p>
                  {t(
                    '以下是检索排序信号，不表示答案正确率。',
                    'These are ranking signals, not answer accuracy.',
                  )}
                </p>
                <dl>
                  <dt>{t('关键词名次', 'Keyword rank')}</dt>
                  <dd>{item.lexical_rank ?? '—'}</dd>
                  <dt>{t('余弦相似度', 'Cosine similarity')}</dt>
                  <dd>{item.cosine_similarity?.toFixed(4) ?? '—'}</dd>
                  <dt>{t('融合名次', 'Fusion rank')}</dt>
                  <dd>{item.fusion_rank ?? '—'}</dd>
                </dl>
              </details>
            </article>
          ))}
          {result.data.items.length === result.query.limit && (
            <p className="muted">
              {t(
                `显示前 ${result.query.limit} 条命中，请缩小范围获取更精确结果。`,
                `Showing the top ${result.query.limit} matches; narrow the scope for more precise results.`,
              )}
            </p>
          )}
        </div>
      )}
    </PageFrame>
  );
}

function HighlightedText({
  text,
  query,
  language,
}: {
  text: string;
  query: string;
  language: string;
}) {
  return (
    <>
      {searchHighlightParts(text, query, language).map((part, index) =>
        part.match ? (
          <mark className="inference-highlight" key={index}>
            {part.text}
          </mark>
        ) : (
          <span key={index}>{part.text}</span>
        ),
      )}
    </>
  );
}

function modeLabel(mode: SearchMode, t: (zh: string, en: string) => string): string {
  return mode === 'lexical'
    ? t('关键词', 'Keyword')
    : mode === 'semantic'
      ? t('语义', 'Semantic')
      : t('混合', 'Hybrid');
}

function orderLabel(order: SearchResultOrder, t: (zh: string, en: string) => string): string {
  return order === 'keyword'
    ? t('关键词优先', 'Keyword first')
    : order === 'semantic'
      ? t('语义优先', 'Semantic first')
      : t('融合', 'Fusion');
}

function historyCountLabel(
  entry: SearchHistoryEntry,
  t: (zh: string, en: string) => string,
): string {
  return entry.result_count >= entry.limit
    ? t(`返回前 ${entry.result_count} 条`, `Top ${entry.result_count} returned`)
    : t(`返回 ${entry.result_count} 条`, `${entry.result_count} returned`);
}

function describeScope(scope: SearchScope, t: (zh: string, en: string) => string): string {
  const parts = [
    scope.kinds.length === 1
      ? scope.kinds[0] === 'note'
        ? t('仅笔记', 'Notes only')
        : t('仅资料', 'Sources only')
      : t('笔记与资料', 'Notes and sources'),
  ];
  if (scope.object_ids.length)
    parts.push(
      t(`${scope.object_ids.length} 个指定对象`, `${scope.object_ids.length} selected objects`),
    );
  if (scope.collection_id) parts.push(t('所选集合', 'Selected collection'));
  if (scope.tag_ids.length)
    parts.push(t(`${scope.tag_ids.length} 个标签`, `${scope.tag_ids.length} tags`));
  if (scope.archived !== null)
    parts.push(scope.archived ? t('已归档', 'Archived') : t('未归档', 'Unarchived'));
  if (scope.inbox_state)
    parts.push(
      {
        pending: t('待整理', 'Pending'),
        snoozed: t('稍后整理', 'Snoozed'),
        processed: t('已整理', 'Processed'),
      }[scope.inbox_state],
    );
  if (scope.reading_status)
    parts.push(
      {
        unread: t('未读', 'Unread'),
        reading: t('阅读中', 'Reading'),
        finished: t('已读', 'Finished'),
      }[scope.reading_status],
    );
  if (scope.source_format)
    parts.push(
      {
        pdf: 'PDF',
        html: 'HTML',
        markdown: 'Markdown',
        text: t('纯文本', 'Text'),
        docx: 'DOCX',
        doc: 'DOC',
        epub: 'EPUB',
        mobi: 'MOBI',
        csv: 'CSV',
        xlsx: 'XLSX',
        xls: 'XLS',
        pptx: 'PPTX',
      }[scope.source_format],
    );
  return parts.join(' · ');
}

export function ScopePicker({
  identity,
  value,
  onChange,
}: {
  identity: WorkspaceIdentity;
  value: SearchScope;
  onChange: (value: SearchScope) => void;
}) {
  const { t } = useI18n();
  const [collections, setCollections] = useState<CollectionDetail[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  async function load(after: string | null) {
    setBusy(true);
    try {
      const page = await call(
        {
          kind: 'collections_list',
          payload: {
            limit: 50,
            cursor: after,
            trashed: false,
            archived: false,
            pinned_only: false,
            query: null,
          },
        },
        'collections',
        identity,
      );
      if (live.current) {
        setCollections((old) => (after ? [...old, ...page.items] : page.items));
        setCursor(page.next_cursor);
        setError(null);
      }
    } catch (e) {
      if (live.current) setError(message(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }
  useEffect(() => {
    live.current = true;
    void load(null);
    return () => {
      live.current = false;
    };
  }, []);
  return (
    <fieldset className="inference-scope">
      <legend>{t('来源范围', 'Source scope')}</legend>
      {(value.object_ids.length > 0 || value.tag_ids.length > 0) && (
        <p>
          {t(
            `限定 ${value.object_ids.length} 个对象、${value.tag_ids.length} 个标签`,
            `Restricted to ${value.object_ids.length} objects and ${value.tag_ids.length} tags`,
          )}{' '}
          <button
            type="button"
            className="text-button"
            onClick={() => onChange({ ...value, object_ids: [], tag_ids: [] })}
          >
            {t('移除这些限制', 'Remove these restrictions')}
          </button>
        </p>
      )}
      <div className="inference-controls">
        <div className="choice-field">
          {t('内容类型', 'Content type')}
          <Select
            aria-label={t('内容类型', 'Content type')}
            value={value.kinds[0] ?? ''}
            onValueChange={(nextValue) =>
              onChange({
                ...value,
                kinds: nextValue ? [nextValue as 'note' | 'source'] : [],
              })
            }
          >
            <option value="">{t('笔记与资料', 'Notes and sources')}</option>
            <option value="note">{t('仅笔记', 'Notes')}</option>
            <option value="source">{t('仅资料', 'Sources')}</option>
          </Select>
        </div>
        <div className="choice-field">
          {t('集合', 'Collection')}
          <Select
            aria-label={t('集合', 'Collection')}
            value={value.collection_id ?? ''}
            onValueChange={(nextValue) => onChange({ ...value, collection_id: nextValue || null })}
          >
            <option value="">{t('全部集合', 'All collections')}</option>
            {value.collection_id &&
              !collections.some((item) => item.id === value.collection_id) && (
                <option value={value.collection_id}>
                  {t('已限定集合（名称尚未读取）', 'Restricted collection (name not loaded)')}
                </option>
              )}
            {collections.map((item) => (
              <option key={item.id} value={item.id}>
                {item.name}
              </option>
            ))}
          </Select>
        </div>
        <div className="choice-field">
          {t('归档状态', 'Archive status')}
          <Select
            aria-label={t('归档状态', 'Archive status')}
            value={value.archived === null ? '' : String(value.archived)}
            onValueChange={(nextValue) =>
              onChange({
                ...value,
                archived: nextValue === '' ? null : nextValue === 'true',
              })
            }
          >
            <option value="">{t('全部', 'All')}</option>
            <option value="false">{t('未归档', 'Unarchived')}</option>
            <option value="true">{t('已归档', 'Archived')}</option>
          </Select>
        </div>
        <div className="choice-field">
          {t('资料格式', 'Source format')}
          <Select
            aria-label={t('资料格式', 'Source format')}
            value={value.source_format ?? ''}
            onValueChange={(nextValue) =>
              onChange({
                ...value,
                source_format: (nextValue || null) as SearchScope['source_format'],
              })
            }
          >
            <option value="">{t('全部格式', 'All formats')}</option>
            <option value="pdf">PDF</option>
            <option value="html">HTML</option>
            <option value="text">{t('纯文本', 'Text')}</option>
            <option value="markdown">Markdown</option>
            <option value="docx">DOCX</option>
            <option value="doc">DOC</option>
            <option value="epub">EPUB</option>
            <option value="mobi">MOBI</option>
            <option value="csv">CSV</option>
            <option value="xlsx">XLSX</option>
            <option value="xls">XLS</option>
            <option value="pptx">PPTX</option>
          </Select>
        </div>
        {cursor && (
          <button
            type="button"
            className="text-button"
            disabled={busy}
            onClick={() => void load(cursor)}
          >
            {t('加载更多集合', 'More collections')}
          </button>
        )}
      </div>
      <TagScopePicker
        identity={identity}
        selected={value.tag_ids}
        onChange={(tag_ids) => onChange({ ...value, tag_ids })}
      />
      {error && (
        <p className="inference-error" role="alert">
          {error}
          <button
            type="button"
            className="text-button"
            disabled={busy}
            onClick={() => void load(null)}
          >
            {t('重读集合', 'Reload collections')}
          </button>
        </p>
      )}
    </fieldset>
  );
}
