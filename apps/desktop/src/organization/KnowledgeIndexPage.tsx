import { useEffect, useRef, useState } from 'react';
import {
  BookOpen,
  CheckSquare,
  FileText,
  FolderClosed,
  History,
  Inbox,
  LayoutDashboard,
  LayoutList,
  Link2,
  LoaderCircle,
  MessageSquareQuote,
  Network,
  Plus,
  Search,
  Settings2,
  Tags,
  X,
} from 'lucide-react';
import type {
  CollectionDetail,
  InboxState,
  KnowledgeObject,
  KnowledgeObjectFilter,
  KnowledgeObjectHit,
  SearchQuery,
  SearchScope,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { objectTypeQueryTokens, type ObjectTypeDefinition } from './object-types';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { PageFrame } from '../layout/PageFrame';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { KnowledgeContextPanel } from './KnowledgeContextPanel';
import { KnowledgeBatchBar } from './KnowledgeBatchBar';
import { knowledgeSearchNotices } from './knowledge-search-status';
import { SourceImportOrganizerDialog } from './SourceImportOrganizerDialog';
import {
  SourceImportActivity,
  SourceImportDialog,
  SourceLibraryDialog,
} from '../sources/SourceImportDialog';
import { useSourceImports } from '../sources/useSourceImports';
import '../sources/sources.css';
import './knowledge-index.css';

type KnowledgeMode = 'all' | 'inbox' | 'uncategorized';
type KnowledgeSearchMode = 'exact' | 'fuzzy';

export function KnowledgeIndexPage({
  identity,
  onOpenNote,
  onOpenSource,
  onManageCollections,
  onOpenGraph,
  onOpenCanvas,
  onOpenAdvancedSearch,
  objectTypes,
}: {
  identity: WorkspaceIdentity;
  objectTypes: readonly ObjectTypeDefinition[];
  onOpenNote: (id: string) => void;
  onOpenSource: (id: string, versionId?: string, annotationId?: string) => void;
  onManageCollections: () => void;
  onOpenGraph: (collectionId: string) => void;
  onOpenCanvas: (collectionId: string) => void;
  onOpenAdvancedSearch: (collectionId: string | null) => void;
}) {
  const { t, language } = useI18n();
  const [mode, setMode] = usePageNavigationState<KnowledgeMode>('knowledge.mode', 'all');
  const [categoryId, setCategoryId] = usePageNavigationState<string | null>(
    'knowledge.category',
    null,
  );
  const [query, setQuery] = usePageNavigationState('knowledge.query', '');
  const [typeFilter, setTypeFilter] = usePageNavigationState('knowledge.type', '');
  const [searchMode, setSearchMode] = usePageNavigationState<KnowledgeSearchMode>(
    'knowledge.search-mode',
    'exact',
  );
  const [fuzzy, setFuzzy] = useState<{
    ids: string[];
    hits: Record<string, KnowledgeObjectHit>;
    lexicalAvailable: boolean;
    semanticAvailable: boolean;
    unavailableReason: string | null;
  } | null>(null);
  const [fuzzyLoading, setFuzzyLoading] = useState(false);
  const [fuzzyError, setFuzzyError] = useState<string | null>(null);
  const [fuzzyObjects, setFuzzyObjects] = useState<KnowledgeObject[]>([]);
  const [fuzzyObjectsLoading, setFuzzyObjectsLoading] = useState(false);
  const [fuzzyObjectsError, setFuzzyObjectsError] = useState<string | null>(null);
  const [fuzzyObjectsReload, setFuzzyObjectsReload] = useState(0);
  const [collections, setCollections] = useState<CollectionDetail[]>([]);
  const [activeCollection, setActiveCollection] = useState<CollectionDetail | null>(null);
  const [collectionsError, setCollectionsError] = useState<string | null>(null);
  const [collectionsLoading, setCollectionsLoading] = useState(true);
  const [collectionsGeneration, setCollectionsGeneration] = useState(0);
  const [collectionQuery, setCollectionQuery] = useState('');
  const [collectionNext, setCollectionNext] = useState<string | null>(null);
  const [collectionMore, setCollectionMore] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [savedPosition, setSavedPosition] = usePageNavigationState('knowledge.list-position', {
    scope: '',
    page: 1,
    size: 10,
    scroll: 0,
  });
  const listViewport = useRef<HTMLDivElement>(null);
  const restoredPosition = useRef(false);
  const restoringPage = useRef(false);
  const [importOpen, setImportOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [organizeSourceIds, setOrganizeSourceIds] = useState<string[] | null>(null);
  const [importGeneration, setImportGeneration] = useState(0);
  const [selectMode, setSelectMode] = usePageNavigationState('knowledge.select-mode', false);
  const [selectedIds, setSelectedIds] = usePageNavigationState<string[]>(
    'knowledge.selected-ids',
    [],
  );
  const [selectedObjects, setSelectedObjects] = usePageNavigationState<
    Record<string, KnowledgeObject>
  >('knowledge.selected-objects', {});
  const [selectionScope, setSelectionScope] = usePageNavigationState(
    'knowledge.selection-scope',
    '',
  );

  useEffect(() => {
    let live = true;
    setCollectionsLoading(true);
    setCollectionNext(null);
    setCollectionsError(null);
    void call(
      {
        kind: 'collections_list',
        payload: {
          limit: 100,
          cursor: null,
          trashed: false,
          archived: false,
          pinned_only: false,
          query: collectionQuery.trim() || null,
        },
      },
      'collections',
      identity,
    )
      .then((page) => {
        if (live) {
          setCollections(page.items);
          setCollectionNext(page.next_cursor);
        }
      })
      .catch((error) => {
        if (live) setCollectionsError(message(error));
      })
      .finally(() => {
        if (live) setCollectionsLoading(false);
      });
    return () => {
      live = false;
    };
  }, [identity, collectionsGeneration, collectionQuery]);

  useEffect(() => {
    if (!categoryId) {
      setActiveCollection(null);
      return;
    }
    const listed = collections.find((collection) => collection.id === categoryId);
    if (listed) {
      setActiveCollection(listed);
      return;
    }
    let live = true;
    void call(
      { kind: 'collections_get', payload: { collection_id: categoryId } },
      'collection',
      identity,
    )
      .then((collection) => {
        if (live) setActiveCollection(collection);
      })
      .catch((error) => {
        if (live) setCollectionsError(message(error));
      });
    return () => {
      live = false;
    };
  }, [categoryId, collections, identity]);

  async function loadMoreCollections() {
    if (!collectionNext || collectionMore) return;
    setCollectionMore(true);
    setCollectionsError(null);
    try {
      const page = await call(
        {
          kind: 'collections_list',
          payload: {
            limit: 100,
            cursor: collectionNext,
            trashed: false,
            archived: false,
            pinned_only: false,
            query: collectionQuery.trim() || null,
          },
        },
        'collections',
        identity,
      );
      setCollections((current) => [...current, ...page.items]);
      setCollectionNext(page.next_cursor);
    } catch (error) {
      setCollectionsError(message(error));
    } finally {
      setCollectionMore(false);
    }
  }

  useEffect(() => {
    const text = query.trim();
    if (searchMode !== 'fuzzy' || !text) {
      setFuzzy(null);
      setFuzzyLoading(false);
      setFuzzyError(null);
      return;
    }
    setFuzzy(null);
    setFuzzyLoading(true);
    let live = true;
    const timer = window.setTimeout(() => {
      const scope: SearchScope = {
        object_ids: [],
        kinds:
          typeFilter === 'kind:note' || typeFilter.startsWith('type:')
            ? ['note']
            : typeFilter === 'kind:source'
              ? ['source']
              : [],
        tag_ids: [],
        collection_id: mode === 'all' ? categoryId : null,
        inbox_state: mode === 'inbox' ? 'pending' : null,
        reading_status: null,
        source_format: null,
        archived: false,
      };
      setFuzzyLoading(true);
      setFuzzyError(null);
      void call(
        {
          kind: 'search_query',
          payload: {
            query: text,
            mode: 'hybrid',
            scope,
            limit: 100,
            minimum_cosine: null,
          } satisfies SearchQuery,
        },
        'search_results',
        identity,
      )
        .then((result) => {
          if (!live) return;
          const ids = [...new Set(result.items.map((item) => item.target.object_id))];
          const hits: Record<string, KnowledgeObjectHit> = {};
          for (const item of result.items) {
            const objectId = item.target.object_id;
            if (hits[objectId]) continue;
            hits[objectId] = {
              snippet: item.text,
              start_codepoint: item.target.start_codepoint,
              page_index: item.target.page_index,
              block_id: item.target.block_id,
            };
          }
          setFuzzy({
            ids,
            hits,
            lexicalAvailable: result.lexical_available,
            semanticAvailable: result.semantic_available,
            unavailableReason: result.unavailable_reason,
          });
        })
        .catch((failure) => {
          if (live) {
            setFuzzy(null);
            setFuzzyError(message(failure));
          }
        })
        .finally(() => {
          if (live) setFuzzyLoading(false);
        });
    }, 260);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [identity, mode, categoryId, query, searchMode, typeFilter]);

  useEffect(() => {
    let live = true;
    const ids = fuzzy?.ids ?? [];
    if (searchMode !== 'fuzzy' || !fuzzy || ids.length === 0) {
      setFuzzyObjects([]);
      setFuzzyObjectsLoading(false);
      setFuzzyObjectsError(null);
      return;
    }
    setFuzzyObjectsLoading(true);
    setFuzzyObjectsError(null);
    const filter: KnowledgeObjectFilter = {
      kinds: typeFilter.startsWith('kind:')
        ? [typeFilter.slice(5) as 'note' | 'source' | 'annotation']
        : [],
      collection_id: mode === 'uncategorized' ? null : categoryId,
      uncategorized: mode === 'uncategorized',
      tag_ids: [],
      object_ids: ids,
      object_types: typeFilter.startsWith('type:')
        ? objectTypes
            .filter((type) => type.id === typeFilter.slice(5))
            .flatMap(objectTypeQueryTokens)
        : [],
      inbox_state: mode === 'inbox' ? 'pending' : null,
      archived: false,
      trashed: false,
    };
    void call(
      {
        kind: 'knowledge_objects_list',
        payload: { limit: 100, cursor: null, query: null, filter },
      },
      'knowledge_objects',
      identity,
    )
      .then((page) => {
        if (!live) return;
        const order = new Map(ids.map((id, index) => [id, index]));
        setFuzzyObjects(
          [...page.items].sort(
            (left, right) =>
              (order.get(left.object.id) ?? Number.MAX_SAFE_INTEGER) -
              (order.get(right.object.id) ?? Number.MAX_SAFE_INTEGER),
          ),
        );
      })
      .catch((error) => {
        if (live) {
          setFuzzyObjects([]);
          setFuzzyObjectsError(message(error));
        }
      })
      .finally(() => {
        if (live) setFuzzyObjectsLoading(false);
      });
    return () => {
      live = false;
    };
  }, [
    categoryId,
    fuzzy?.ids.join('|'),
    fuzzyObjectsReload,
    identity,
    mode,
    searchMode,
    typeFilter,
    objectTypes,
  ]);

  const fuzzyActive = searchMode === 'fuzzy' && Boolean(query.trim());
  const filter: KnowledgeObjectFilter = {
    kinds: typeFilter.startsWith('kind:')
      ? [typeFilter.slice(5) as 'note' | 'source' | 'annotation']
      : [],
    collection_id: mode === 'uncategorized' ? null : categoryId,
    uncategorized: mode === 'uncategorized',
    tag_ids: [],
    object_ids: [],
    object_types: typeFilter.startsWith('type:')
      ? objectTypes.filter((type) => type.id === typeFilter.slice(5)).flatMap(objectTypeQueryTokens)
      : [],
    inbox_state: mode === 'inbox' ? ('pending' as InboxState) : null,
    archived: false,
    trashed: false,
  };
  const listScope = fuzzyActive
    ? null
    : JSON.stringify([
        identity.local_instance_id,
        identity.workspace_id,
        mode,
        categoryId,
        query.trim(),
        collectionsGeneration,
        importGeneration,
        searchMode,
        typeFilter,
      ]);
  const positionScope = JSON.stringify([mode, categoryId, query.trim(), searchMode, typeFilter]);
  const list = usePagedList<KnowledgeObject>(
    listScope,
    async (cursor, size) => {
      const page = await call(
        {
          kind: 'knowledge_browse',
          payload: {
            limit: size,
            page: cursor ? Number(cursor) : 1,
            query: fuzzyActive ? null : query.trim() || null,
            filter,
            title_query: null,
            updated_from: null,
            updated_to: null,
            source_formats: [],
          },
        },
        'knowledge_browse',
        identity,
      );
      return { ...page, page_index: page.page };
    },
    savedPosition.size,
    true,
  );
  const fuzzyItems = useRef<KnowledgeObject[]>([]);
  fuzzyItems.current = fuzzyObjects;
  const fuzzyListScope =
    fuzzyActive && fuzzy && fuzzy.ids.length > 0 && !fuzzyObjectsLoading && !fuzzyObjectsError
      ? JSON.stringify([
          identity.local_instance_id,
          identity.workspace_id,
          mode,
          categoryId,
          query.trim(),
          fuzzy.ids.join('|'),
          fuzzyObjects.length,
          typeFilter,
        ])
      : null;
  const fuzzyList = usePagedList<KnowledgeObject>(
    fuzzyListScope,
    async (cursor, size) => {
      const offset = cursor ? Number(cursor) : 0;
      const rows = fuzzyItems.current.slice(offset, offset + size);
      const next =
        offset + rows.length < fuzzyItems.current.length ? String(offset + rows.length) : null;
      return { items: rows, next_cursor: next };
    },
    savedPosition.size,
  );
  const activeList = fuzzyActive ? fuzzyList : list;
  const visibleItems = activeList.page?.items ?? [];
  const activeLoading = activeList.loading || (fuzzyActive && fuzzyObjectsLoading);
  const activeError = fuzzyActive ? (fuzzyObjectsError ?? fuzzyList.error) : list.error;
  const fuzzyNotices =
    fuzzyActive && !fuzzyLoading && fuzzy
      ? knowledgeSearchNotices({ ...fuzzy, idCount: fuzzy.ids.length }, t)
      : [];

  useEffect(() => {
    if (
      !listViewport.current ||
      restoredPosition.current ||
      restoringPage.current ||
      activeLoading ||
      !activeList.page
    )
      return;
    if (savedPosition.scope !== positionScope) {
      restoredPosition.current = true;
      return;
    }
    if (activeList.index < savedPosition.page && activeList.page.next_cursor) {
      restoringPage.current = true;
      void activeList.goTo(savedPosition.page).finally(() => {
        restoringPage.current = false;
      });
      return;
    }
    if (listViewport.current) listViewport.current.scrollTop = savedPosition.scroll;
    restoredPosition.current = true;
  }, [activeLoading, activeList, positionScope, savedPosition]);

  function rememberList() {
    setSavedPosition({
      scope: positionScope,
      page: activeList.index,
      size: activeList.size,
      scroll: listViewport.current?.scrollTop ?? 0,
    });
  }
  function openItem(item: KnowledgeObject) {
    rememberList();
    if (item.annotation)
      onOpenSource(
        item.annotation.source_id,
        item.annotation.source_version_id,
        item.annotation.id,
      );
    else if (item.source) onOpenSource(item.source.id, item.source.current_version_id ?? undefined);
    else onOpenNote(item.object.id);
  }

  const selected = visibleItems.find((item) => item.object.id === selectedId) ?? null;
  useEffect(() => {
    setSelectedObjects((old) => {
      const visible = new Map(visibleItems.map((item) => [item.object.id, item]));
      const next = Object.fromEntries(
        selectedIds.flatMap((id) => {
          const item = visible.get(id) ?? old[id];
          return item ? [[id, item]] : [];
        }),
      );
      return Object.keys(next).length === Object.keys(old).length &&
        Object.keys(next).every((id) => next[id] === old[id])
        ? old
        : next;
    });
  }, [visibleItems, selectedIds, setSelectedObjects]);
  const selectedItems = selectedIds.flatMap((id) =>
    selectedObjects[id] ? [selectedObjects[id]] : [],
  );
  const empty =
    !activeLoading &&
    !activeError &&
    !fuzzyLoading &&
    visibleItems.length === 0 &&
    !(fuzzyActive && !fuzzy);
  const imports = useSourceImports({
    identity,
    onOpenSource,
    onChanged: () => {
      setImportGeneration((value) => value + 1);
      setCollectionsGeneration((value) => value + 1);
    },
  });

  useEffect(() => {
    setSelectedId(null);
  }, [mode, categoryId, query]);

  function chooseMode(next: KnowledgeMode) {
    setMode(next);
    if (next === 'inbox') setCategoryId(null);
  }

  async function refreshAfterChange() {
    await list.refresh();
    setCollectionsGeneration((value) => value + 1);
  }

  const pageTitle =
    mode === 'inbox'
      ? t('收件箱', 'Inbox')
      : mode === 'uncategorized'
        ? t('未分类', 'Uncategorized')
        : (activeCollection?.name ?? t('全部知识', 'All knowledge'));
  const pageDescription =
    mode === 'inbox'
      ? t(
          '导入或新建后还没有完成整理的内容。完成整理后会离开收件箱。',
          'Imported or new items leave the inbox after they are organized.',
        )
      : t(
          '按分类查找内容，打开条目即可阅读或编辑。',
          'Find content by category, then open it to read or write.',
        );
  const pageActions = (
    <div className="knowledge-list-actions">
      <button type="button" className="primary-button" onClick={() => setImportOpen(true)}>
        <Plus size={14} />
        {t('导入资料', 'Import')}
      </button>
      <button type="button" className="secondary-button" onClick={() => setActivityOpen(true)}>
        <History size={14} />
        {t('导入记录', 'Import history')}
        {imports.items.length > 0 && <small>{imports.items.length}</small>}
      </button>
      <button
        type="button"
        className="secondary-button"
        aria-pressed={selectMode}
        onClick={() => {
          setSelectMode((value) => !value);
          setSelectedIds([]);
          setSelectedId(null);
        }}
      >
        <CheckSquare size={14} />
        {selectMode ? t('结束选择', 'Done') : t('批量整理', 'Select')}
      </button>
      {mode === 'all' && categoryId && (
        <>
          <button
            type="button"
            className="secondary-button"
            onClick={() => onOpenGraph(categoryId)}
          >
            <Network size={14} />
            {t('图谱', 'Graph')}
          </button>
          <button
            type="button"
            className="secondary-button"
            onClick={() => onOpenCanvas(categoryId)}
          >
            <LayoutDashboard size={14} />
            {t('画布', 'Canvas')}
          </button>
        </>
      )}
      <span className="knowledge-count">
        {fuzzyActive ? fuzzyObjects.length : (list.page?.total ?? 0)}
      </span>
    </div>
  );
  if (selected) {
    return (
      <PageFrame
        className="knowledge-page-frame knowledge-detail-frame"
        title={t('对象详情', 'Object details')}
        description={t(
          '查看分类、标签、关系、摘录与来源；返回索引时保留当前筛选。',
          'Review categories, tags, relations, excerpts and source context; go back without losing the current filter.',
        )}
      >
        <section
          className="knowledge-detail-page"
          aria-label={t('知识对象详情', 'Knowledge object detail')}
        >
          <KnowledgeContextPanel
            identity={identity}
            item={selected}
            onOpenNote={onOpenNote}
            onOpenSource={onOpenSource}
            onManageCollections={onManageCollections}
            onChanged={() => void refreshAfterChange()}
            onClose={() => {
              restoredPosition.current = false;
              setSelectedId(null);
            }}
          />
        </section>
      </PageFrame>
    );
  }
  return (
    <PageFrame
      className="knowledge-page-frame"
      title={pageTitle}
      description={pageDescription}
      actions={pageActions}
      footer={
        <ListPagination
          list={activeList}
          total={fuzzyActive ? fuzzyObjects.length : null}
          disabled={!activeList.page || activeList.loading}
        />
      }
    >
      <section className="knowledge-index" aria-label={t('知识索引', 'Knowledge index')}>
        <aside className="knowledge-categories" aria-label={t('内容分类', 'Content categories')}>
          <header>
            <div>
              <h2>{t('范围与分类', 'Scope and categories')}</h2>
              <p>
                {t(
                  '在全部知识、收件箱和分类之间切换。',
                  'Switch between all knowledge, inbox and categories.',
                )}
              </p>
            </div>
            <button
              className="icon-button"
              onClick={onManageCollections}
              title={t('管理分类', 'Manage categories')}
              aria-label={t('管理分类', 'Manage categories')}
            >
              <Settings2 size={16} />
            </button>
          </header>
          <label className="knowledge-category-search">
            <Search size={13} aria-hidden="true" />
            <input
              value={collectionQuery}
              onChange={(event) => setCollectionQuery(event.target.value)}
              placeholder={t('查找分类…', 'Find a category…')}
              aria-label={t('查找分类', 'Find a category')}
            />
            {collectionQuery && (
              <button
                type="button"
                className="icon-button"
                aria-label={t('清除分类搜索', 'Clear category search')}
                onClick={() => setCollectionQuery('')}
              >
                <X size={12} />
              </button>
            )}
          </label>
          <button
            className={`knowledge-category ${mode === 'all' && !categoryId ? 'active' : ''}`}
            aria-pressed={mode === 'all' && !categoryId}
            onClick={() => {
              setCategoryId(null);
              chooseMode('all');
            }}
          >
            <LayoutList size={16} />
            <span>{t('全部知识', 'All knowledge')}</span>
          </button>
          <button
            className={`knowledge-category ${mode === 'inbox' ? 'active' : ''}`}
            aria-pressed={mode === 'inbox'}
            onClick={() => chooseMode('inbox')}
          >
            <Inbox size={16} />
            <span>{t('待整理', 'Inbox')}</span>
          </button>
          <button
            className={`knowledge-category ${mode === 'uncategorized' ? 'active' : ''}`}
            aria-pressed={mode === 'uncategorized'}
            onClick={() => {
              setCategoryId(null);
              chooseMode('uncategorized');
            }}
          >
            <Tags size={16} />
            <span>{t('未分类', 'Uncategorized')}</span>
          </button>
          <div className="knowledge-category-heading">
            <span>{t('内容分类', 'Categories')}</span>
            <button className="text-button" onClick={onManageCollections}>
              <Plus size={13} />
              {t('管理', 'Manage')}
            </button>
          </div>
          <div className="knowledge-category-list">
            {collectionsLoading && (
              <p className="knowledge-category-note" role="status">
                <LoaderCircle size={13} className="spin" />
                {t('正在读取分类…', 'Loading categories…')}
              </p>
            )}
            {collectionsError && (
              <div className="knowledge-category-note" role="alert">
                <span>{collectionsError}</span>
                <button
                  className="text-button"
                  onClick={() => setCollectionsGeneration((value) => value + 1)}
                >
                  {t('重试', 'Retry')}
                </button>
              </div>
            )}
            {!collectionsLoading && !collectionsError && collections.length === 0 && (
              <p className="knowledge-category-note">
                {collectionQuery.trim()
                  ? t(
                      '没有匹配的分类；当前选中的分类仍会保留。',
                      'No category matches; the active category is kept.',
                    )
                  : t(
                      '还没有分类。打开集合页创建第一个分类。',
                      'No categories yet. Create one in Collections.',
                    )}
              </p>
            )}
            {activeCollection &&
              !collections.some((collection) => collection.id === activeCollection.id) && (
                <button
                  className="knowledge-category active"
                  aria-pressed="true"
                  onClick={() => setCategoryId(activeCollection.id)}
                >
                  <FolderClosed size={16} />
                  <span title={activeCollection.name}>{activeCollection.name}</span>
                </button>
              )}
            {collections.map((collection) => (
              <button
                key={collection.id}
                className={`knowledge-category ${mode === 'all' && categoryId === collection.id ? 'active' : ''}`}
                aria-pressed={mode === 'all' && categoryId === collection.id}
                onClick={() => {
                  setMode('all');
                  setCategoryId(collection.id);
                }}
              >
                <FolderClosed size={16} />
                <span title={collection.name}>{collection.name}</span>
              </button>
            ))}
          </div>
          {collectionNext && (
            <button
              type="button"
              className="text-button knowledge-more-categories"
              disabled={collectionMore}
              onClick={() => void loadMoreCollections()}
            >
              {collectionMore ? t('正在读取…', 'Loading…') : t('加载更多分类', 'More categories')}
            </button>
          )}
        </aside>

        <div className="knowledge-list-panel">
          <label className="knowledge-search">
            <Search size={16} aria-hidden="true" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={
                searchMode === 'fuzzy'
                  ? t(
                      '输入关键词、主题或问题，查找相关对象…',
                      'Find related objects by keyword, topic or question…',
                    )
                  : t('按标题或原文短语精确查找…', 'Find an exact title or source phrase…')
              }
              aria-label={t(
                '按标题或原文短语查找笔记与资料',
                'Find notes and sources by title or exact phrase',
              )}
            />
            {query && (
              <button
                className="icon-button"
                aria-label={t('清除搜索', 'Clear search')}
                onClick={() => setQuery('')}
              >
                <X size={14} />
              </button>
            )}
          </label>
          <div
            className="knowledge-search-modes"
            role="group"
            aria-label={t('查找方式', 'Search matching')}
          >
            <button
              type="button"
              aria-pressed={searchMode === 'exact'}
              onClick={() => setSearchMode('exact')}
            >
              {t('精确匹配内容', 'Exact phrase')}
            </button>
            <button
              type="button"
              aria-pressed={searchMode === 'fuzzy'}
              onClick={() => setSearchMode('fuzzy')}
            >
              {t('模糊查找相关对象', 'Fuzzy related objects')}
            </button>
          </div>
          <button
            type="button"
            className="text-button knowledge-advanced-search"
            onClick={() => onOpenAdvancedSearch(mode === 'all' ? categoryId : null)}
          >
            <Search size={13} aria-hidden="true" />
            {t('高级检索与证据', 'Advanced search and evidence')}
          </button>
          {fuzzyActive && fuzzyLoading && (
            <p className="knowledge-search-status" role="status">
              <LoaderCircle size={13} className="spin" />
              {t(
                '正在按关键词和语义查找相关对象…',
                'Finding related objects by keyword and meaning…',
              )}
            </p>
          )}
          {fuzzyError && (
            <p className="knowledge-list-error" role="alert">
              {fuzzyError}
            </p>
          )}
          {fuzzyNotices.map((notice, index) => (
            <p
              key={`${notice.tone}-${index}`}
              className={
                notice.tone === 'alert' ? 'knowledge-list-error' : 'knowledge-search-status'
              }
              role={notice.tone === 'alert' ? 'alert' : 'status'}
            >
              {notice.text}
            </p>
          ))}
          {selectMode && (
            <p className="knowledge-selection-hint">
              {t(
                '可跨页选择，每批最多 100 项。',
                'Select across pages, up to 100 items per batch.',
              )}
            </p>
          )}
          {activeError && (
            <div className="knowledge-list-error" role="alert">
              <span>{activeError}</span>
              <button
                className="secondary-button"
                onClick={() => {
                  if (fuzzyActive && fuzzyObjectsError) setFuzzyObjectsReload((value) => value + 1);
                  else void activeList.refresh();
                }}
              >
                {t('重试', 'Retry')}
              </button>
            </div>
          )}
          {activeLoading && !activeList.page && (
            <p className="knowledge-list-loading" role="status">
              <LoaderCircle size={16} className="spin" />
              {t('正在读取知识索引…', 'Loading knowledge index…')}
            </p>
          )}
          {empty && (
            <div className="knowledge-empty">
              <BookOpen size={30} strokeWidth={1.5} />
              <h3>
                {query
                  ? t('没有匹配的内容', 'No matching content')
                  : mode === 'inbox'
                    ? t('收件箱已清空', 'Inbox is clear')
                    : t('这里还没有知识对象', 'No knowledge objects yet')}
              </h3>
              <p>
                {query
                  ? t(
                      '试试更短的关键词，或清除分类筛选。',
                      'Try a shorter query or clear the category filter.',
                    )
                  : mode === 'inbox'
                    ? t(
                        '导入或新建的内容完成整理后会离开这里。',
                        'Imported or new items leave here after organizing.',
                      )
                    : t(
                        '先导入一份资料，或在分类中写下第一条笔记。',
                        'Import a source or write the first note in a category.',
                      )}
              </p>
              {query ? (
                <button className="primary-button" onClick={() => setQuery('')}>
                  {t('清除搜索', 'Clear search')}
                </button>
              ) : mode === 'inbox' || mode === 'uncategorized' || categoryId ? (
                <button
                  className="secondary-button"
                  onClick={() => {
                    setCategoryId(null);
                    chooseMode('all');
                  }}
                >
                  {t('查看全部知识', 'View all knowledge')}
                </button>
              ) : (
                <button className="primary-button" onClick={() => setImportOpen(true)}>
                  <Plus size={15} />
                  {t('导入第一份资料', 'Import the first source')}
                </button>
              )}
            </div>
          )}
          <div
            className="knowledge-type-filter"
            role="group"
            aria-label={t('类型筛选', 'Type filter')}
          >
            <span>{t('知识类型', 'Knowledge type')}</span>
            <Select
              value={typeFilter}
              onValueChange={setTypeFilter}
              aria-label={t('筛选知识类型', 'Filter knowledge type')}
            >
              <option value="">{t('全部类型', 'All types')}</option>
              <option value="kind:note">{t('全部笔记', 'All notes')}</option>
              <option value="kind:source">{t('资料', 'Documents')}</option>
              <option value="kind:annotation">{t('摘录与批注', 'Excerpts & annotations')}</option>
              {objectTypes.map((type) => (
                <option key={type.id} value={`type:${type.id}`}>
                  {t(type.labelZh, type.labelEn)}
                </option>
              ))}
            </Select>
          </div>
          {selectedIds.length > 0 && selectionScope !== positionScope && (
            <div className="knowledge-selection-scope" role="status">
              <span>
                {t(
                  `筛选已改变，保留了 ${selectedIds.length} 个已选条目。`,
                  `Filters changed; ${selectedIds.length} selected items are kept.`,
                )}
              </span>
              <button className="text-button" onClick={() => setSelectionScope(positionScope)}>
                {t('保留选择', 'Keep selection')}
              </button>
              <button className="text-button" onClick={() => setSelectedIds([])}>
                {t('清空选择', 'Clear selection')}
              </button>
            </div>
          )}
          <div className="knowledge-list" role="list" ref={listViewport}>
            {visibleItems.map((item) => {
              const hit = fuzzyActive
                ? (fuzzy?.hits[item.object.id] ?? null)
                : query.trim()
                  ? item.hit
                  : null;
              const hitPage = hit?.page_index;
              return (
                <div
                  className="knowledge-row-group"
                  role="listitem"
                  key={`${item.object.kind}/${item.object.id}`}
                >
                  <button
                    type="button"
                    className="knowledge-row"
                    aria-current={selectedId === item.object.id}
                    aria-label={t(
                      `打开 ${item.title || '未命名'}`,
                      `Open ${item.title || 'Untitled'}`,
                    )}
                    aria-pressed={selectMode ? selectedIds.includes(item.object.id) : undefined}
                    disabled={
                      selectMode &&
                      selectedIds.length >= 100 &&
                      !selectedIds.includes(item.object.id)
                    }
                    onClick={() => {
                      if (selectMode) {
                        setSelectionScope(positionScope);
                        setSelectedIds((current) =>
                          current.includes(item.object.id)
                            ? current.filter((id) => id !== item.object.id)
                            : [...current, item.object.id],
                        );
                      } else openItem(item);
                    }}
                  >
                    <span className={`knowledge-kind is-${item.object.kind}`} aria-hidden="true">
                      {item.object.kind === 'source' ? (
                        <BookOpen size={18} />
                      ) : item.object.kind === 'annotation' ? (
                        <MessageSquareQuote size={18} />
                      ) : (
                        <FileText size={18} />
                      )}
                    </span>
                    <span className="knowledge-row-main">
                      <span className="knowledge-row-title">
                        {item.title || t('未命名', 'Untitled')}
                      </span>
                      <span className={`knowledge-row-preview ${hit ? 'is-hit' : ''}`}>
                        {hit?.snippet || item.preview || t('暂无摘要', 'No preview yet')}
                      </span>
                      {hit && (hitPage != null || hit.block_id) && (
                        <span className="knowledge-hit-locator">
                          {hitPage != null
                            ? t(`第 ${hitPage + 1} 页`, `Page ${hitPage + 1}`)
                            : t('命中原文块', 'Matched source block')}
                        </span>
                      )}
                      <span className="knowledge-row-meta">
                        {item.collections.map((collection) => (
                          <span className="knowledge-chip" key={collection.id}>
                            <FolderClosed size={11} />
                            {collection.name}
                          </span>
                        ))}
                        {item.object.tags.map((tag) => (
                          <span className="knowledge-chip is-tag" key={tag.id}>
                            <Tags size={11} />
                            {tag.display_name}
                          </span>
                        ))}
                        {item.object.kind === 'annotation' && (
                          <span className="knowledge-chip is-excerpt">
                            <MessageSquareQuote size={11} />
                            {t('摘录', 'Excerpt')}
                          </span>
                        )}
                        {item.object.inbox_state === 'pending' && (
                          <span className="knowledge-chip is-pending">{t('待整理', 'Inbox')}</span>
                        )}
                        {item.object.inbox_state === 'snoozed' && (
                          <span className="knowledge-chip is-snoozed">
                            {t('稍后整理', 'Later')}
                          </span>
                        )}
                        {item.related_count > 0 && (
                          <span className="knowledge-chip">
                            <Link2 size={11} />
                            {t(`${item.related_count} 个关联`, `${item.related_count} related`)}
                          </span>
                        )}
                      </span>
                    </span>
                    <time dateTime={item.updated_at}>{displayDate(item.updated_at, language)}</time>
                  </button>
                  {!selectMode && (
                    <button
                      type="button"
                      className="icon-button knowledge-row-manage"
                      aria-label={t(
                        `管理 ${item.title || '未命名'} 的分类与关系`,
                        `Manage categories and relations for ${item.title || 'Untitled'}`,
                      )}
                      onClick={() => {
                        rememberList();
                        setSelectedId(item.object.id);
                      }}
                    >
                      <Settings2 size={16} aria-hidden="true" />
                    </button>
                  )}
                </div>
              );
            })}
          </div>
          {selectMode && selectedIds.length > 0 && (
            <KnowledgeBatchBar
              beforeTask={rememberList}
              identity={identity}
              selected={selectedItems}
              onClear={() => setSelectedIds([])}
              onChanged={() => void refreshAfterChange()}
            />
          )}
        </div>
        {importOpen && (
          <SourceImportDialog
            identity={identity}
            importing={imports.importing}
            locked={imports.actions.locked}
            error={imports.error || imports.actions.error}
            onFiles={async (selection) => {
              const started = await imports.importFiles(selection);
              if (started) setActivityOpen(true);
              return started;
            }}
            onDirectory={async (selection) => {
              const started = await imports.importDirectory(selection);
              if (started) setActivityOpen(true);
              return started;
            }}
            onWebpage={async (address, selection) => {
              const started = await imports.importWebpage(address, selection);
              if (started) setActivityOpen(true);
              return started;
            }}
            initialCollection={
              activeCollection ? { id: activeCollection.id, name: activeCollection.name } : null
            }
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
              pending={!!imports.actions.pending}
              requestError={imports.actions.error}
              importing={imports.importing}
              locked={imports.actions.locked}
              scan={imports.scan}
              onRetry={imports.retryJobs}
              onRetryJob={imports.retryJob}
              onConfirmPending={() => void imports.retry()}
              onAcknowledge={() => setActivityOpen(false)}
              onContinue={() => void imports.continueRemaining()}
              onCancelRemaining={() => void imports.cancelRemaining()}
              onDismissScan={imports.clearScan}
              onGoInbox={() => {
                setActivityOpen(false);
                chooseMode('inbox');
              }}
              onOrganizeImported={(sourceIds) => {
                setActivityOpen(false);
                setOrganizeSourceIds(sourceIds);
              }}
              onOpenJob={(job) => {
                if (job.source_id) onOpenSource(job.source_id, job.source_version_id ?? undefined);
              }}
            />
          </SourceLibraryDialog>
        )}
        {organizeSourceIds && (
          <SourceImportOrganizerDialog
            identity={identity}
            sourceIds={organizeSourceIds}
            onClose={() => setOrganizeSourceIds(null)}
            onChanged={() => {
              setImportGeneration((value) => value + 1);
              setCollectionsGeneration((value) => value + 1);
            }}
          />
        )}
      </section>
    </PageFrame>
  );
}
