import { useEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  BookOpen,
  CheckSquare,
  History,
  Plus,
  RefreshCw,
  Search,
  SlidersHorizontal,
  X,
} from 'lucide-react';
import type {
  SourceJob,
  SourcesFilter as SourcesFilterValue,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { PageState } from '../layout/PageState';
import { resolveListDisplayState } from '../layout/page-state';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { PageFrame } from '../layout/PageFrame';
import { CollectionNavigator } from '../organization/CollectionNavigator';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { DEFAULT_PAGE_SIZE } from '../layout/pagination.ts';
import { Select } from '../ui/Select';
import { Tooltip } from '../ui/Tooltip';
import { SourceFilters, emptySourceFilter } from './SourceFilters';
import { SourceBulkOrganization } from './SourceBulkOrganization';
import { useKnowledgeTaskNavigation } from '../inference/knowledge-task-navigation';
import { SourceLibraryRow } from './SourceLibraryRow';
import {
  SourceImportActivity,
  SourceImportDialog,
  SourceLibraryDialog,
} from './SourceImportDialog';
import { useSourceImports, type SourceImportRequest } from './useSourceImports';
import { sourceBatchProgress, sourceBatchProgressLabel } from './source-import-batch';
import type { SelectedSource } from './source-bulk-actions';
import './sources.css';

export type { SourceImportRequest } from './useSourceImports';
export interface SourcesPageProps {
  identity: WorkspaceIdentity;
  onOpenSource: (sourceId: string, versionId?: string) => void;
  onManageCollections: () => void;
  onOpenImportedSource?: (
    sourceId: string,
    versionId: string | undefined,
    inputToken: string,
  ) => void;
  onChanged?: () => void;
  onOpenTasks?: () => void;
  importRequest?: SourceImportRequest | null;
  onImportRequestHandled?: (id: string) => void;
}
export function SourcesPage(props: SourcesPageProps) {
  return <SourcesScope key={JSON.stringify(props.identity)} {...props} />;
}
function SourcesScope({
  identity,
  onOpenSource,
  onManageCollections,
  onOpenImportedSource,
  onChanged,
  onOpenTasks,
  importRequest,
  onImportRequestHandled,
}: SourcesPageProps) {
  const { t } = useI18n();
  const taskNavigation = useKnowledgeTaskNavigation();
  const [query, setQuery] = usePageNavigationState('sources.query', '');
  const [filter, setFilter] = useState(query);
  const [trashed, setTrashed] = usePageNavigationState('sources.trashed', false);
  const [sourcesFilter, setSourcesFilter] = usePageNavigationState<SourcesFilterValue>(
    'sources.filter',
    emptySourceFilter,
  );
  const [savedScroll, setSavedScroll] = usePageNavigationState('sources.scroll', 0);
  const [savedAnchor, setSavedAnchor] = usePageNavigationState<{
    id: string;
    offset: number;
  } | null>('sources.scroll-anchor', null);
  const [pageIndex, setPageIndex] = usePageNavigationState('sources.page', 1);
  const [pageSize, setPageSize] = usePageNavigationState<number>(
    'sources.page-size',
    DEFAULT_PAGE_SIZE,
  );
  const [acknowledgedIssues, setAcknowledgedIssues] = usePageNavigationState<string[]>(
    'sources.acknowledged-import-issues',
    [],
  );
  const [refresh, setRefresh] = useState(0);
  const [importOpen, setImportOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [selecting, setSelecting] = usePageNavigationState('sources.selecting', false);
  const [selected, setSelected] = usePageNavigationState<SelectedSource[]>('sources.selected', []);
  const listViewport = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const selectionTrigger = useRef<HTMLButtonElement>(null);
  const activityTrigger = useRef<HTMLButtonElement>(null);
  const initialPosition = useRef({ scroll: savedScroll, anchor: savedAnchor });
  const restoreScroll = useRef(true);
  const restoreTarget = useRef(pageIndex);
  const restored = useRef(false);
  const [settled, setSettled] = useState(false);
  function changed() {
    setRefresh((value) => value + 1);
    onChanged?.();
  }
  function rememberScroll() {
    const position = readPosition();
    setSavedScroll(position.scroll);
    setSavedAnchor(position.anchor);
  }
  function readPosition() {
    const viewport = listViewport.current;
    const top = viewport?.getBoundingClientRect().top ?? 0;
    const firstVisible = viewport
      ? Array.from(viewport.querySelectorAll<HTMLElement>('[data-source-id]')).find(
          (row) => row.getBoundingClientRect().bottom > top,
        )
      : null;
    return {
      scroll: viewport?.scrollTop ?? 0,
      anchor: firstVisible?.dataset.sourceId
        ? {
            id: firstVisible.dataset.sourceId,
            offset: firstVisible.getBoundingClientRect().top - top,
          }
        : null,
    };
  }
  function openSource(sourceId: string, versionId?: string) {
    rememberScroll();
    onOpenSource(sourceId, versionId);
  }
  function openImported(sourceId: string, versionId: string | undefined, token: string) {
    rememberScroll();
    if (onOpenImportedSource) onOpenImportedSource(sourceId, versionId, token);
    else onOpenSource(sourceId, versionId);
  }
  const imports = useSourceImports({
    identity,
    importRequest,
    onImportRequestHandled,
    onOpenSource: openSource,
    onOpenImportedSource: openImported,
    onChanged: changed,
  });
  const { actions } = imports;
  // Every input that makes page 1 mean something new belongs in the scope, so a
  // filter, a workspace switch or a trash toggle starts a fresh list at page 1.
  const scope = JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    filter,
    trashed,
    sourcesFilter,
  ]);
  const list = usePagedList(
    scope,
    async (cursor, size) => {
      const result = await call(
        {
          kind: 'sources_browse',
          payload: {
            page: cursor ? Number(cursor) : 1,
            limit: size,
            trashed,
            query: filter.trim() || null,
            filter: sourcesFilter,
          },
        },
        'sources_browse_page',
        identity,
      );
      return {
        items: result.items,
        total: result.total,
        page_index: result.page,
        next_cursor: result.page * size < result.total ? String(result.page + 1) : null,
      };
    },
    pageSize,
    true,
  );
  const pageItems = list.page?.items ?? [];
  // An empty list only means empty once a page arrived; before that it is opening.
  const busy = query !== filter || list.loading || (!list.page && !list.error);
  const error = list.error;
  const displayState = resolveListDisplayState({
    hasPage: Boolean(list.page),
    itemCount: pageItems.length,
    loading: list.loading,
    error,
  });
  // The scope the list on screen was read for, so a change can reset the page.
  const previousScope = useRef(scope);
  const hasConditions = Boolean(
    filter.trim() ||
    sourcesFilter.format ||
    sourcesFilter.reading_status ||
    sourcesFilter.inbox_state ||
    sourcesFilter.collection_id ||
    sourcesFilter.tag_ids.length,
  );
  const filterCount =
    [
      sourcesFilter.format,
      sourcesFilter.reading_status,
      sourcesFilter.inbox_state,
      sourcesFilter.collection_id,
    ].filter(Boolean).length + sourcesFilter.tag_ids.length;
  const selectedIds = new Set(selected.map((item) => item.id));
  const titleCounts = new Map<string, number>();
  for (const source of pageItems)
    titleCounts.set(source.display_title, (titleCounts.get(source.display_title) ?? 0) + 1);
  const activeJobs = imports.jobs.filter(
    (job) => job.status === 'queued' || job.status === 'running',
  ).length;
  // When the library is truly empty, the centered empty state is the one clear
  // primary action. Keep the header trigger for filtered or populated lists.
  const emptyLibrary = !busy && !pageItems.length && !error && !hasConditions && !trashed;
  const listUnavailable = displayState === 'error';
  const issueKeys = [
    ...imports.items
      .filter((item) => item.state === 'failed')
      .map((item) => `file:${item.token}:${item.error ?? ''}`),
    ...imports.jobs
      .filter((job) => job.status === 'failed' || job.stage === 'existing_trashed_source')
      .map((job) => `job:${job.id}:${job.status}:${job.updated_at}`),
    // A folder scan reports per-entry results even when nothing failed, so the
    // activity panel is the place those results stay visible.
    ...(imports.scan
      ? [
          // One key per scan keeps the report reachable even when nothing was skipped.
          `scan:${imports.scan.rootName}:${imports.scan.visited}:${imports.scan.skipped.length}`,
          ...imports.scan.skipped.map((entry) => `scan:${entry.relativePath}:${entry.status}`),
          ...imports.scan.duplicates.map((entry) => `scan-duplicate:${entry.relativePath}`),
          ...(imports.scan.truncated ? ['scan:truncated'] : []),
          ...(imports.scan.limited > 0 ? ['scan:limited'] : []),
        ]
      : []),
  ];
  const unacknowledgedIssues = issueKeys.some((key) => !acknowledgedIssues.includes(key));
  const unfinishedFiles = imports.items.some(
    (item) => item.state === 'waiting' || item.state === 'unconfirmed',
  );
  // The banner is closed-dialog feedback: it states how far this batch got, so the
  // user does not have to reopen the import history to know whether it is moving.
  const batchProgress = sourceBatchProgress(imports.items, imports.jobs);

  useEffect(() => {
    const timer = setTimeout(() => setFilter(query), 200);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    if (previousScope.current === scope) return;
    // A new query is a new list: page 1, no selection, no scroll to restore.
    previousScope.current = scope;
    setSelected([]);
    setPageIndex(1);
    setSavedScroll(0);
    setSavedAnchor(null);
    initialPosition.current = { scroll: 0, anchor: null };
    restoreScroll.current = false;
    // The restoration belongs to the previous scope; a new scope ends it.
    restored.current = true;
    if (listViewport.current) listViewport.current.scrollTop = 0;
  }, [scope, setPageIndex, setSavedScroll, setSavedAnchor]);
  // A refresh or a committed command changes the rows, not the query: re-read the
  // page on screen and keep the reader's place, which is what the windowed list did.
  const refreshTrigger = useRef(`${refresh}:${actions.generation}`);
  useEffect(() => {
    const trigger = `${refresh}:${actions.generation}`;
    if (refreshTrigger.current === trigger) return;
    refreshTrigger.current = trigger;
    initialPosition.current = readPosition();
    restoreScroll.current = true;
    setSettled(true);
    void list.refresh();
  }, [refresh, actions.generation, list]);
  useEffect(() => {
    if (restored.current || busy || !list.page) return;
    // The backend returns the current total; restore directly to the requested
    // page, clamped if rows disappeared while the content page was open.
    const target = Math.min(
      restoreTarget.current,
      Math.max(1, Math.ceil((list.page.total ?? 0) / list.size)),
    );
    if (!error && list.index !== target) {
      void list.goTo(target);
      return;
    }
    restored.current = true;
    setPageIndex(list.index);
    setSettled(true);
  }, [busy, error, list.index, list.page?.total, list.size, setPageIndex]);
  useEffect(() => {
    setPageSize(list.size);
  }, [list.size, setPageSize]);
  useEffect(() => {
    // Until restoration settles, the visible page is not the one to remember.
    if (!restored.current) return;
    setPageIndex(list.index);
  }, [list.index, setPageIndex]);
  useEffect(() => {
    if (!settled || busy || !pageItems.length || !restoreScroll.current) return;
    const viewport = listViewport.current;
    if (!viewport) return;
    const { scroll, anchor } = initialPosition.current;
    const row = anchor
      ? viewport.querySelector<HTMLElement>(`[data-source-id="${CSS.escape(anchor.id)}"]`)
      : null;
    if (row && anchor)
      viewport.scrollTop +=
        row.getBoundingClientRect().top - viewport.getBoundingClientRect().top - anchor.offset;
    else viewport.scrollTop = scroll;
    restoreScroll.current = false;
  }, [settled, busy, pageItems.length]);
  function clearFilters() {
    setQuery('');
    setFilter('');
    setSourcesFilter(emptySourceFilter);
    setTrashed(false);
    search.current?.focus();
  }
  function selectSource(id: string, title: string, checked: boolean) {
    setSelecting(true);
    setSelected((old) =>
      checked
        ? old.some((item) => item.id === id) || old.length >= 100
          ? old
          : [...old, { id, title }]
        : old.filter((item) => item.id !== id),
    );
  }
  function openJob(job: SourceJob) {
    if (!job.source_id) return;
    setActivityOpen(false);
    if (job.status === 'succeeded')
      openImported(job.source_id, job.source_version_id ?? undefined, job.input.input_token);
    else openSource(job.source_id, job.source_version_id ?? undefined);
  }
  function showImport() {
    imports.clearError();
    setImportOpen(true);
  }
  const pageActions = (
    <div className="source-tools">
      <Tooltip label={t('导入记录', 'Import history')}>
        <button
          ref={activityTrigger}
          type="button"
          className="icon-button source-activity-trigger"
          aria-label={t('导入记录', 'Import history')}
          onClick={() => setActivityOpen(true)}
        >
          <History size={18} aria-hidden="true" />
          {activeJobs > 0 && (
            <span
              className="source-activity-dot"
              aria-label={t('有导入任务进行中', 'Imports in progress')}
            />
          )}
        </button>
      </Tooltip>
      {!emptyLibrary && !listUnavailable && (
        <button
          type="button"
          className="primary-button source-import-trigger"
          onClick={showImport}
          disabled={imports.importing}
        >
          <Plus size={16} aria-hidden="true" />
          {t('导入', 'Import')}
        </button>
      )}
    </div>
  );
  const toolbar = (
    <div className="sources-toolbar-stack">
      <div className="sources-toolbar">
        <label className="sources-search">
          <Search size={16} aria-hidden="true" />
          <input
            ref={search}
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label={t('搜索资料标题', 'Search document titles')}
            placeholder={t('搜索资料…', 'Search library…')}
          />
          {query && (
            <button
              type="button"
              className="icon-button"
              aria-label={t('清除搜索', 'Clear search')}
              onClick={() => {
                setQuery('');
                search.current?.focus();
              }}
            >
              <X size={14} aria-hidden="true" />
            </button>
          )}
        </label>
        <Select
          aria-label={t('资料范围', 'Library scope')}
          value={trashed ? 'trash' : 'active'}
          onValueChange={(scope) => setTrashed(scope === 'trash')}
        >
          <option value="active">{t('全部资料', 'All documents')}</option>
          <option value="trash">{t('回收站', 'Trash')}</option>
        </Select>
        <div className="sources-toolbar-actions">
          <button
            type="button"
            className="secondary-button source-filter-trigger"
            aria-label={t(
              `筛选资料（${filterCount} 项条件）`,
              `Filter documents (${filterCount} conditions)`,
            )}
            title={t(
              `筛选资料（${filterCount} 项条件）`,
              `Filter documents (${filterCount} conditions)`,
            )}
            onClick={() => setFiltersOpen(true)}
          >
            <SlidersHorizontal size={16} aria-hidden="true" />
            <span className="source-toolbar-label">
              {t('筛选', 'Filters')}
              {filterCount ? ` · ${filterCount}` : ''}
            </span>
          </button>
          <button
            ref={selectionTrigger}
            type="button"
            aria-pressed={selecting}
            className="source-selection-trigger"
            aria-label={selecting ? t('结束选择', 'Done selecting') : t('批量选择', 'Select')}
            title={selecting ? t('结束选择', 'Done selecting') : t('批量选择', 'Select')}
            disabled={listUnavailable || (busy && !pageItems.length)}
            onClick={() => {
              setSelecting(!selecting);
              setSelected([]);
            }}
          >
            <CheckSquare size={16} aria-hidden="true" />
            <span className="source-toolbar-label">
              {selecting ? t('结束选择', 'Done selecting') : t('批量选择', 'Select')}
            </span>
          </button>
          <Tooltip label={t('刷新资料', 'Refresh library')}>
            <button
              type="button"
              className="icon-button"
              aria-label={t('刷新资料', 'Refresh library')}
              disabled={busy}
              onClick={() => setRefresh((value) => value + 1)}
            >
              <RefreshCw size={16} aria-hidden="true" />
            </button>
          </Tooltip>
        </div>
      </div>
    </div>
  );
  return (
    <PageFrame
      className="sources-page"
      title={t('资料库', 'Library')}
      description={t(
        '导入、浏览并阅读原始资料；打开后可摘录并回到原文位置。',
        'Import, browse and read original documents; excerpts keep their source location.',
      )}
      actions={pageActions}
      toolbar={toolbar}
      footer={<ListPagination list={list} disabled={busy} />}
    >
      <div className="sources-browser">
        <CollectionNavigator
          identity={identity}
          selected={sourcesFilter.collection_id}
          onSelect={(id) => setSourcesFilter((current) => ({ ...current, collection_id: id }))}
          onManage={onManageCollections}
        />
        <div className="sources-browser-content">
          {selecting && (
            <div className="sources-selection-bar">
              <label className="source-check-label">
                <input
                  type="checkbox"
                  aria-label={t(
                    '选择已加载资料（最多100份）',
                    'Select loaded documents (up to 100)',
                  )}
                  checked={
                    pageItems.length > 0 &&
                    pageItems.slice(0, 100).every((item) => selectedIds.has(item.id))
                  }
                  disabled={busy || !pageItems.length}
                  onChange={(event) => {
                    const checked = event.target.checked;
                    setSelected((previous) =>
                      checked
                        ? [
                            ...new Map([
                              ...previous.map((item) => [item.id, item] as const),
                              ...pageItems.map(
                                (item) =>
                                  [item.id, { id: item.id, title: item.display_title }] as const,
                              ),
                            ]).values(),
                          ].slice(0, 100)
                        : previous.filter((item) => !pageItems.some((row) => row.id === item.id)),
                    );
                  }}
                />
                {t('选择已加载', 'Select loaded')}
              </label>
              <span role="status">
                {t('已选 ' + selected.length + ' 份', selected.length + ' selected')}
              </span>
              <small>{t('每批最多 100 份', 'Up to 100 per batch')}</small>
              {taskNavigation && !trashed && (
                <button
                  className="secondary-button"
                  aria-label={t('用所选资料发起任务', 'Start task with selection')}
                  disabled={!selected.length}
                  onClick={() => {
                    rememberScroll();
                    taskNavigation.start(
                      selected.map((item) => ({ object_id: item.id, content_revision: null })),
                    );
                  }}
                >
                  {t('发起任务', 'Create task')}
                </button>
              )}
            </div>
          )}
          <SourceBulkOrganization
            identity={identity}
            selected={selected}
            trashed={trashed}
            onClear={() => {
              setSelected([]);
              selectionTrigger.current?.focus();
            }}
            onChanged={changed}
          />
          {(displayState === 'content-with-error' || actions.error || imports.error) && (
            <div className="source-notice" role="alert">
              <AlertCircle size={17} aria-hidden="true" />
              <span>{error || imports.error || actions.error}</span>
              {error && displayState === 'content-with-error' && (
                <button type="button" disabled={busy} onClick={() => void list.refresh()}>
                  {t('重试', 'Retry')}
                </button>
              )}
              {!actions.ready && actions.error && (
                <button
                  type="button"
                  disabled={imports.importing}
                  onClick={() => void imports.retry()}
                >
                  {t('重试本机操作记录', 'Retry local operation storage')}
                </button>
              )}
            </div>
          )}
          {actions.pending && (
            <div className="source-notice">
              <span>
                {t(
                  '导入结果尚未确认，原请求已保留。',
                  'Import acceptance is unconfirmed. The original request is retained.',
                )}
              </span>
              <button
                type="button"
                disabled={actions.busy || imports.importing}
                onClick={() => void imports.retry()}
              >
                {t('核对原请求', 'Check request')}
              </button>
            </div>
          )}
          {(imports.importing ||
            activeJobs > 0 ||
            unfinishedFiles ||
            unacknowledgedIssues ||
            imports.jobsError) && (
            <button
              type="button"
              className="source-import-progress"
              onClick={() => setActivityOpen(true)}
            >
              <span>
                {imports.importing || activeJobs > 0
                  ? t('正在导入资料…', 'Importing documents…')
                  : t('导入需要处理', 'Imports need attention')}
                {batchProgress.total > 0 && (
                  <span className="source-import-progress-count">
                    {' · '}
                    {sourceBatchProgressLabel(batchProgress, t)}
                  </span>
                )}
              </span>
              <span>{t('查看进度与结果', 'View progress and results')}</span>
            </button>
          )}
          <div className="sources-list" ref={listViewport} aria-busy={busy}>
            {pageItems.length > 0 && (
              <ul className="sources-document-list" aria-label={t('资料列表', 'Document list')}>
                {pageItems.map((source) => (
                  <SourceLibraryRow
                    key={source.id}
                    source={source}
                    selecting={selecting}
                    selected={selectedIds.has(source.id)}
                    selectionFull={selected.length >= 100}
                    sameTitle={(titleCounts.get(source.display_title) ?? 0) > 1}
                    onOpen={() => openSource(source.id)}
                    onStartTask={
                      taskNavigation
                        ? () => {
                            rememberScroll();
                            taskNavigation.start([
                              { object_id: source.id, content_revision: source.current_version_id },
                            ]);
                          }
                        : undefined
                    }
                    onSelect={(checked) => selectSource(source.id, source.display_title, checked)}
                  />
                ))}
              </ul>
            )}
            {displayState === 'error' && (
              <PageState
                kind="error"
                title={t('无法读取资料', 'Could not load documents')}
                detail={error}
                retryLabel={t('重新读取资料', 'Reload documents')}
                onRetry={() => void list.refresh()}
              />
            )}
            {displayState === 'loading' && (
              <div className="source-list-loading" role="status">
                <span className="source-loading-line" />
                <span className="source-loading-line" />
                <span className="source-loading-line" />
                {t('正在读取资料…', 'Loading documents…')}
              </div>
            )}
            {displayState === 'empty' && (
              <div className="source-empty">
                <BookOpen size={32} strokeWidth={1.5} aria-hidden="true" />
                <h2>
                  {hasConditions
                    ? t('没有匹配的资料', 'No matching documents')
                    : trashed
                      ? t('回收站为空', 'Trash is empty')
                      : t('从一份值得阅读的资料开始', 'Start with something worth reading')}
                </h2>
                <p>
                  {hasConditions
                    ? t(
                        '试试其他关键词，或清除筛选。',
                        'Try another keyword, or clear the filters.',
                      )
                    : trashed
                      ? t('已回收的资料会显示在这里。', 'Trashed documents will appear here.')
                      : t(
                          '导入文档、电子书、表格、演示或网页，直接阅读、摘录与整理。',
                          'Import documents, ebooks, spreadsheets, presentations or webpages to read, highlight and organize.',
                        )}
                </p>
                {hasConditions || trashed ? (
                  <button type="button" onClick={clearFilters}>
                    {t('查看全部资料', 'View all documents')}
                  </button>
                ) : (
                  <button type="button" className="primary-button" onClick={showImport}>
                    <Plus size={16} aria-hidden="true" />
                    {t('导入资料', 'Import documents')}
                  </button>
                )}
              </div>
            )}
            {busy && pageItems.length > 0 && (
              <p className="source-list-status" role="status">
                {t('正在更新…', 'Updating…')}
              </p>
            )}
          </div>
          {filtersOpen && (
            <SourceLibraryDialog
              title={t('筛选资料', 'Filter documents')}
              className="source-filter-dialog"
              onClose={() => setFiltersOpen(false)}
            >
              <div className="source-filter-dialog-body">
                <p className="muted">
                  {t('选择条件后立即更新列表。', 'Filters update the list immediately.')}
                </p>
                <SourceFilters
                  identity={identity}
                  value={sourcesFilter}
                  onChange={setSourcesFilter}
                  generation={actions.generation + refresh}
                />
              </div>
              <footer className="source-filter-dialog-footer">
                <button className="primary-button" onClick={() => setFiltersOpen(false)}>
                  {t('完成', 'Done')}
                </button>
              </footer>
            </SourceLibraryDialog>
          )}
          {importOpen && (
            <SourceImportDialog
              identity={identity}
              importing={imports.importing}
              locked={actions.locked}
              error={imports.error || actions.error}
              onFiles={imports.importFiles}
              onDirectory={imports.importDirectory}
              onWebpage={imports.importWebpage}
              onClose={() => setImportOpen(false)}
            />
          )}
          {activityOpen && (
            <SourceLibraryDialog
              title={t('导入记录', 'Import history')}
              onClose={() => setActivityOpen(false)}
              className="source-activity-dialog"
            >
              <SourceImportActivity
                items={imports.items}
                jobs={imports.jobs}
                loaded={imports.jobsLoaded}
                error={imports.jobsError}
                pending={!!actions.pending}
                requestError={actions.error}
                importing={imports.importing}
                locked={actions.locked}
                onRetry={imports.retryJobs}
                onRetryJob={imports.retryJob}
                onConfirmPending={() => void imports.retry()}
                onAcknowledge={
                  unacknowledgedIssues
                    ? () => {
                        setAcknowledgedIssues(issueKeys);
                        setActivityOpen(false);
                        requestAnimationFrame(() => activityTrigger.current?.focus());
                      }
                    : undefined
                }
                onContinue={() => void imports.continueRemaining()}
                scan={imports.scan}
                onCancelRemaining={() => void imports.cancelRemaining()}
                onDismissScan={imports.clearScan}
                onOpenJob={openJob}
                onOpenTasks={
                  onOpenTasks
                    ? () => {
                        setActivityOpen(false);
                        rememberScroll();
                        onOpenTasks();
                      }
                    : undefined
                }
              />
            </SourceLibraryDialog>
          )}
        </div>
      </div>
    </PageFrame>
  );
}
