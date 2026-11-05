import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
} from 'react';
import { createPortal } from 'react-dom';
import {
  ArrowLeft,
  Download,
  ExternalLink,
  FileJson,
  FolderClosed,
  LayoutDashboard,
  Link2,
  LoaderCircle,
  Maximize2,
  Maximize,
  Minimize,
  Move,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Printer,
  RefreshCw,
  Save,
  Search,
  Trash2,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import type {
  Canvas,
  CanvasNode,
  CanvasNote,
  CanvasView,
  CollectionMembersPage,
  KnowledgeGraph,
  KnowledgeGraphEdge,
  Note,
  NoteSummary,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { usePageNavigationState } from '../layout/page-navigation-state';
import {
  ActionNotice,
  PageLoadFailure,
  QueryFeedback,
  type OrganizationPageProps,
} from './OrganizationControls';
import { PageActions, pageKey } from './page-actions';
import { usePageActions } from './page-hooks';
import { fitCanvasViewport, initialCanvasViewport, visibleCanvasNodes } from './canvas-geometry';
import {
  canvasViewportKey,
  canvasViewportPreference,
  readCanvasViewport,
  restoreCanvasViewport,
  type CanvasTransform,
  type CanvasViewportPreference,
} from './canvas-viewport';
import {
  canvasRelationLines,
  canvasNoteEdges,
  canvasRelationKey,
  relationKindLabel,
  unplacedRelationTargets,
} from './canvas-relations';
import { readObjectRelations, setFrontmatterRelations } from './object-types';
import { ViewMetadataEditor, viewStatusLabel } from './view-layouts';
import { TagPills } from '../ui/TagPills';
import { Select } from '../ui/Select';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { notify } from '../ui/ToastHost';
import { downloadText, safeFilename } from './view-export';
import { canvasSvg } from './canvas-export';
import { SourceCollectionPicker } from '../sources/SourceCollectionPicker';
import { PopupMenu } from '../ui/PopupMenu';
import { useContentFullscreen } from '../ui/useContentFullscreen';
import './canvas-page.css';

const NODE_WIDTH = 250;
const NODE_HEIGHT = 148;
const NODE_MIN_WIDTH = 160;
const NODE_MIN_HEIGHT = 96;
const RESIZE_DIRECTIONS = ['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw'] as const;
type ResizeDirection = (typeof RESIZE_DIRECTIONS)[number];
type CanvasPageProps = OrganizationPageProps & {
  onOpenTarget: NonNullable<OrganizationPageProps['onOpenTarget']>;
  withSavedNote: (noteId: string, action: () => Promise<void>) => Promise<void>;
  onOpenTrash: () => void;
  registerSave: (save: (() => Promise<void>) | null) => void;
};

export function CanvasPage(props: CanvasPageProps) {
  const loaded = usePageActions(props.identity, 'canvas');
  return loaded.controller ? (
    <CanvasScope
      key={pageKey(props.identity, 'canvas')}
      {...props}
      controller={loaded.controller}
    />
  ) : (
    <PageLoadFailure error={loaded.error} retry={loaded.retry} />
  );
}

function CanvasScope(props: CanvasPageProps & { controller: PageActions }) {
  const { t } = useI18n();
  const mutation = useSyncExternalStore(props.controller.subscribe, props.controller.getSnapshot);
  const [recovered, setRecovered] = useState(0);
  const saveBeforeLeave = useRef<(() => Promise<void>) | null>(null);
  const registerSave = useCallback(
    (save: (() => Promise<void>) | null) => {
      saveBeforeLeave.current = save;
      props.registerSave(save);
    },
    [props.registerSave],
  );
  const [selected, setSelected] = usePageNavigationState<string | null>('canvas.selected', null);
  const [collectionId, setCollectionId] = usePageNavigationState<string | null>(
    'canvas.collection',
    null,
  );
  const [collectionName, setCollectionName] = usePageNavigationState('canvas.collection-name', '');
  const [query, setQuery] = usePageNavigationState('canvas.query', '');
  const [effectiveQuery, setEffectiveQuery] = useState(query.trim());
  const [page, setPage] = usePageNavigationState('canvas.list-page', 1);
  const [size, setSize] = usePageNavigationState('canvas.list-size', 10);
  const [scroll, setScroll] = usePageNavigationState('canvas.list-scroll', 0);
  const listElement = useRef<HTMLDivElement>(null);
  const resume = useRef({ scope: '', pending: false });
  const [resuming, setResuming] = useState(false);
  const [openedName, setOpenedName] = useState('');
  const [creating, setCreating] = useState(false);
  const [name, setName] = usePageNavigationState('canvas.new-name', '');
  const [busy, setBusy] = useState(false);
  const locked = busy || mutation.busy || !!mutation.pending;
  const [error, setError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const showNotice = useCallback((value: string | null) => {
    if (value) notify({ text: value, tone: 'success' });
  }, []);
  useEffect(() => {
    const timer = setTimeout(() => setEffectiveQuery(query.trim()), 180);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    if (!collectionId) return;
    let live = true;
    void call(
      { kind: 'collections_get', payload: { collection_id: collectionId } },
      'collection',
      props.identity,
    )
      .then((value) => {
        if (live) setCollectionName(value.name);
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [collectionId, refresh, props.identity.local_instance_id, props.identity.workspace_id]);
  const scope = `${pageKey(props.identity, 'canvas')}/list/${JSON.stringify(effectiveQuery)}/${refresh}`;
  const canvases = usePagedList<Canvas>(
    scope,
    (cursor, limit) =>
      call(
        {
          kind: 'canvases_list',
          payload: { limit, cursor, pinned_only: false, query: effectiveQuery || null },
        },
        'canvases',
        props.identity,
      ),
    size,
  );
  useEffect(() => {
    if (!canvases.page || canvases.loading || resume.current.scope === scope) return;
    resume.current = { scope, pending: page > 1 };
    if (page <= 1) {
      setResuming(false);
      return;
    }
    setResuming(true);
    void canvases.goTo(page).finally(() => {
      if (resume.current.scope === scope) {
        resume.current.pending = false;
        setResuming(false);
      }
    });
  }, [scope, canvases.page, canvases.loading]);
  useEffect(() => {
    if (
      !canvases.loading &&
      !canvases.error &&
      canvases.page &&
      !resume.current.pending &&
      resume.current.scope === scope &&
      query.trim() === effectiveQuery
    )
      setPage(canvases.index);
  }, [scope, canvases.index, canvases.loading, canvases.page, resuming]);
  useLayoutEffect(() => {
    if (!selected && listElement.current && !canvases.loading && !resuming)
      listElement.current.scrollTop = scroll;
  }, [selected, canvases.page, canvases.loading, resuming]);
  const listBusy = canvases.loading || resuming || query.trim() !== effectiveQuery;
  const pager = {
    ...canvases,
    loading: listBusy,
    setSize: (value: number) => {
      setScroll(0);
      setSize(value);
      setPage(1);
      return canvases.setSize(value);
    },
    next: () => {
      setScroll(0);
      return canvases.next();
    },
    previous: () => {
      setScroll(0);
      return canvases.previous();
    },
    goTo: (value: number) => {
      setScroll(0);
      return canvases.goTo(value);
    },
  };
  function openCanvas(canvas: Canvas) {
    setError(null);
    setOpenedName(canvas.name);
    setSelected(canvas.id);
    setCreating(false);
  }
  async function createCanvas() {
    if (!name.trim() || locked) return;
    setBusy(true);
    setError(null);
    try {
      const result = await props.controller.run({
        kind: 'canvases_save',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            canvas_id: null,
            base_row_version: null,
            name: name.trim(),
            config_schema: 1,
            nodes: [],
            is_pinned: false,
            pin_position: null,
          },
        },
      });
      if (!result || !('canvas' in result)) return;
      openCanvas(result.canvas);
      setName('');
      setPage(1);
      setScroll(0);
      setQuery('');
      setRefresh((value) => value + 1);
      props.onChanged?.();
    } catch (failure) {
      setError(message(failure));
    } finally {
      setBusy(false);
    }
  }
  async function recoverAction() {
    const result = await props.controller.run();
    if (!result) return;
    if ('canvas' in result) {
      if (result.removed) {
        setSelected(null);
        setOpenedName('');
      } else {
        setSelected(result.canvas.id);
        setOpenedName(result.canvas.name);
      }
    }
    setRecovered((value) => value + 1);
    setRefresh((value) => value + 1);
    props.onChanged?.();
  }
  return (
    <PageFrame
      className={`org-page canvas-page${selected ? ' is-canvas-open' : ''}`}
      title={selected ? openedName || t('画布', 'Canvas') : t('画布', 'Canvases')}
      description={
        selected
          ? undefined
          : t(
              '用已有笔记组织想法，打开画布即可继续工作。',
              'Organize ideas with existing notes. Open a canvas to continue working.',
            )
      }
      actions={
        selected ? (
          <button
            className="secondary-button"
            onClick={() => {
              void (saveBeforeLeave.current?.() ?? Promise.resolve())
                .then(() => {
                  setSelected(null);
                  setOpenedName('');
                })
                .catch((failure) => setError(message(failure)));
            }}
          >
            <ArrowLeft size={16} />
            {t('返回画布列表', 'Back to canvases')}
          </button>
        ) : (
          <button className="primary-button" disabled={locked} onClick={() => setCreating(true)}>
            <Plus size={16} />
            {t('新建画布', 'New canvas')}
          </button>
        )
      }
      toolbar={
        selected ? (
          <div className="canvas-category-view">
            <FolderClosed size={15} aria-hidden="true" />
            <SourceCollectionPicker
              identity={props.identity}
              value={
                collectionId
                  ? { id: collectionId, name: collectionName || t('当前分类', 'Current category') }
                  : null
              }
              onChange={(value) => {
                setCollectionId(value?.id ?? null);
                setCollectionName(value?.name ?? '');
              }}
              disabled={false}
              label={t('分类视图', 'Category view')}
              emptyLabel={t('全部分类', 'All categories')}
              generation={refresh}
            />
            {collectionId && (
              <small>
                {t(
                  '其他节点暂时隐藏，完整布局仍保留。',
                  'Other nodes are hidden; the full layout is retained.',
                )}
              </small>
            )}
          </div>
        ) : (
          <div className="canvas-list-tools">
            <Search size={16} aria-hidden="true" />
            <input
              className="ui-input"
              type="search"
              aria-label={t('查找画布', 'Find canvases')}
              placeholder={t('按名称查找画布…', 'Find a canvas by name…')}
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setPage(1);
                setScroll(0);
              }}
            />
            <button
              className="secondary-button"
              disabled={listBusy}
              onClick={() => setRefresh((value) => value + 1)}
            >
              <RefreshCw size={15} />
              {t('刷新画布', 'Refresh canvases')}
            </button>
          </div>
        )
      }
      footer={selected ? undefined : <ListPagination list={pager} />}
    >
      <ActionNotice state={mutation} retry={() => void recoverAction()} />
      {error && (
        <div className="canvas-banner error" role="alert">
          <span>{error}</span>
          <button className="text-button" onClick={() => setError(null)}>
            {t('关闭', 'Dismiss')}
          </button>
        </div>
      )}
      {selected ? (
        <div className="canvas-detail">
          <CanvasEditor
            key={`${selected}/${recovered}`}
            identity={props.identity}
            canvasId={selected}
            collectionId={collectionId}
            onOpenNote={props.onOpenNote}
            onOpenTarget={props.onOpenTarget}
            onTitle={setOpenedName}
            controller={props.controller}
            onSavedAs={openCanvas}
            withSavedNote={props.withSavedNote}
            onOpenTrash={props.onOpenTrash}
            registerSave={registerSave}
            onChanged={() => {
              setRefresh((value) => value + 1);
              props.onChanged?.();
            }}
            onDeleted={() => {
              setSelected(null);
              setOpenedName('');
              setRefresh((value) => value + 1);
              props.onChanged?.();
            }}
            onNotice={showNotice}
            onError={setError}
          />
        </div>
      ) : (
        <div
          className="canvas-list-content"
          ref={listElement}
          onScroll={() => {
            if (!listBusy && !resume.current.pending)
              setScroll(listElement.current?.scrollTop ?? 0);
          }}
        >
          {creating && (
            <form
              className="canvas-create-form"
              onSubmit={(event) => {
                event.preventDefault();
                void createCanvas();
              }}
            >
              <label>
                {t('画布名称', 'Canvas name')}
                <input
                  className="ui-input"
                  aria-label={t('画布名称', 'Canvas name')}
                  autoFocus
                  maxLength={300}
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <div className="button-row">
                <button className="primary-button" disabled={locked || !name.trim()}>
                  {busy ? t('正在创建…', 'Creating…') : t('创建', 'Create')}
                </button>
                <button
                  className="secondary-button"
                  type="button"
                  disabled={locked}
                  onClick={() => setCreating(false)}
                >
                  {t('取消', 'Cancel')}
                </button>
              </div>
            </form>
          )}
          <div
            className="canvas-list-items"
            role="list"
            aria-label={t('画布列表', 'Canvas list')}
            aria-busy={listBusy}
          >
            {canvases.page?.items.map((canvas) => (
              <div role="listitem" key={canvas.id} data-canvas-id={canvas.id}>
                <button
                  className="org-index-item"
                  disabled={listBusy}
                  onClick={() => openCanvas(canvas)}
                >
                  <LayoutDashboard size={18} />
                  <span>
                    <strong>{canvas.name}</strong>
                    <small>
                      {t(`${canvas.nodes.length} 个节点`, `${canvas.nodes.length} nodes`)}
                    </small>
                  </span>
                  {canvas.is_pinned && (
                    <Pin size={14} role="img" aria-label={t('已置顶', 'Pinned')} />
                  )}
                  <ExternalLink size={15} />
                </button>
              </div>
            ))}
          </div>
          <QueryFeedback
            state={{ ...canvases, loading: listBusy }}
            reload={() => setRefresh((value) => value + 1)}
            pager={null}
            empty={
              query
                ? t('没有匹配的画布。', 'No canvases match.')
                : t(
                    '还没有画布，创建一个开始整理想法。',
                    'Create a canvas to start organizing ideas.',
                  )
            }
          />
        </div>
      )}
    </PageFrame>
  );
}

function CanvasEditor({
  identity,
  canvasId,
  collectionId,
  onOpenNote,
  onOpenTarget,
  onChanged,
  onDeleted,
  onNotice,
  onError,
  onTitle,
  controller,
  onSavedAs,
  withSavedNote,
  onOpenTrash,
  registerSave,
}: {
  identity: OrganizationPageProps['identity'];
  canvasId: string;
  collectionId: string | null;
  onOpenNote: (id: string) => void;
  onOpenTarget: CanvasPageProps['onOpenTarget'];
  onChanged: () => void;
  onDeleted: () => void;
  onNotice: (value: string | null) => void;
  onError: (value: string | null) => void;
  onTitle: (value: string) => void;
  controller: PageActions;
  onSavedAs: (canvas: Canvas) => void;
  withSavedNote: CanvasPageProps['withSavedNote'];
  onOpenTrash: () => void;
  registerSave: CanvasPageProps['registerSave'];
}) {
  const { t, language } = useI18n();
  const fullscreen = useContentFullscreen('.canvas-page');
  const [view, setView] = useState<CanvasView | null>(null);
  const [draft, setDraft] = useState<Canvas | null>(null);
  const draftRef = useRef<Canvas | null>(null);
  const savedCanvas = useRef<Canvas | null>(null);
  const pendingSave = useRef<Promise<unknown> | null>(null);
  const leaveAction = useRef<() => Promise<void>>(async () => undefined);
  useEffect(() => {
    registerSave(() => leaveAction.current());
    return () => registerSave(null);
  }, [registerSave]);
  const [loading, setLoading] = useState(true);
  const [saving, setBusy] = useState(false);
  const mutation = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const busy = saving || mutation.busy || !!mutation.pending;
  const [selectedNode, setSelectedNode] = usePageNavigationState<string | null>(
    `canvas.node/${canvasId}`,
    null,
  );
  const [query, setQuery] = useState('');
  const [searching, setSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<NoteSummary[] | null>(null);
  const [searchCursor, setSearchCursor] = useState<string | null>(null);
  const searchSequence = useRef(0);
  const [relationQuery, setRelationQuery] = useState('');
  const [relationTargets, setRelationTargets] = useState<NoteSummary[] | null>(null);
  const [relationCursor, setRelationCursor] = useState<string | null>(null);
  const [relationSearching, setRelationSearching] = useState(false);
  const relationSequence = useRef(0);
  const [relationBusy, setRelationBusy] = useState(false);
  const [relationOpen, setRelationOpen] = useState(false);
  const [removeConfirm, setRemoveConfirm] = useState(false);
  const [renameOpen, setRenameOpen] = useState(false);
  const [reloadConfirm, setReloadConfirm] = useState(false);
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [graphError, setGraphError] = useState<string | null>(null);
  const [graphRefresh, setGraphRefresh] = useState(0);
  const [categoryMembers, setCategoryMembers] = useState<Set<string> | null>(null);
  const [categoryLoading, setCategoryLoading] = useState(false);
  const [categoryError, setCategoryError] = useState('');
  const [categoryReload, setCategoryReload] = useState(0);
  const [selectedEdge, setSelectedEdge] = useState<KnowledgeGraphEdge | null>(null);
  const [relationFields, setRelationFields] = useState<string[]>([]);
  const [relationDetailError, setRelationDetailError] = useState('');
  const [removeRelation, setRemoveRelation] = useState<{
    note: Note;
    field: string;
    targetId: string;
    targetTitle: string;
  } | null>(null);
  const [stageSize, setStageSize] = useState({ width: 0, height: 0 });
  const [viewport, setViewport] = useState({ x: 0, y: 0, scale: 1 });
  const viewportRef = useRef(viewport);
  const wheelHandler = useRef<(event: WheelEvent) => void>(() => undefined);
  viewportRef.current = viewport;
  const restoredViewport = useRef<CanvasViewportPreference | null>(null);
  const viewportReadFailed = useRef(false);
  const [viewportError, setViewportError] = useState('');
  const viewportKey = canvasViewportKey(identity, canvasId);
  const stage = useRef<HTMLDivElement>(null);
  const initialFitDone = useRef(false);
  const drag = useRef<{
    nodeId: string;
    startX: number;
    startY: number;
    nodeX: number;
    nodeY: number;
  } | null>(null);
  const resize = useRef<{
    nodeId: string;
    direction: ResizeDirection;
    startX: number;
    startY: number;
    nodeX: number;
    nodeY: number;
    nodeWidth: number;
    nodeHeight: number;
  } | null>(null);
  const pan = useRef<{ startX: number; startY: number; x: number; y: number } | null>(null);
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);
  useEffect(() => {
    searchSequence.current++;
    relationSequence.current++;
    setSearchResults(null);
    setSearchCursor(null);
    setSearching(false);
    setRelationTargets(null);
    setRelationCursor(null);
    setRelationSearching(false);
    return () => {
      searchSequence.current++;
      relationSequence.current++;
    };
  }, [canvasId, collectionId, identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    let live = true;
    setCategoryMembers(null);
    setCategoryError('');
    setCategoryLoading(!!collectionId);
    if (collectionId)
      void (async () => {
        try {
          const members = new Set<string>();
          const cursors = new Set<string>();
          let cursor: string | null = null;
          let version: number | null = null;
          do {
            const page: CollectionMembersPage = await call(
              {
                kind: 'collections_members',
                payload: {
                  collection_id: collectionId,
                  limit: 100,
                  cursor,
                  include_trashed: true,
                  kind: 'note',
                },
              },
              'collection_members',
              identity,
            );
            if (!live) return;
            if (version !== null && version !== page.collection_row_version)
              throw new Error(t('分类已变化，请重新读取。', 'The category changed. Reload it.'));
            version = page.collection_row_version;
            for (const member of page.items) members.add(member.object.id);
            cursor = page.next_cursor;
            if (cursor && cursors.has(cursor))
              throw new Error(t('分类分页响应无效。', 'Invalid category page response.'));
            if (cursor) cursors.add(cursor);
          } while (cursor);
          setCategoryMembers(members);
        } catch (failure) {
          if (live) setCategoryError(message(failure));
        } finally {
          if (live) setCategoryLoading(false);
        }
      })();
    return () => {
      live = false;
    };
  }, [collectionId, categoryReload, identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    let live = true;
    setGraphError(null);
    void call({ kind: 'knowledge_graph' }, 'knowledge_graph', identity)
      .then((value) => live && setGraph(value))
      .catch((failure) => live && setGraphError(message(failure)));
    return () => {
      live = false;
    };
  }, [identity, canvasId, graphRefresh]);
  useEffect(() => {
    let live = true;
    initialFitDone.current = false;
    setLoading(true);
    setView(null);
    setDraft(null);
    setRelationOpen(false);
    setViewport({ x: 0, y: 0, scale: 1 });
    setViewportError('');
    restoredViewport.current = null;
    viewportReadFailed.current = false;
    try {
      restoredViewport.current = readCanvasViewport(localStorage, viewportKey);
    } catch (failure) {
      viewportReadFailed.current = true;
      setViewportError(message(failure));
    }
    void call({ kind: 'canvases_get', payload: { canvas_id: canvasId } }, 'canvas', identity)
      .then((loaded) => {
        if (!live) return;
        setView(loaded);
        onTitle(loaded.canvas.name);
        setDraft(loaded.canvas);
        draftRef.current = loaded.canvas;
        savedCanvas.current = loaded.canvas;
        setSelectedNode((id) =>
          loaded.canvas.nodes.some((node) => node.note_id === id) ? id : null,
        );
      })
      .catch((failure) => live && onError(message(failure)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [canvasId, identity, onError]);
  useEffect(() => {
    if (!graphRefresh) return;
    let live = true;
    void call({ kind: 'canvases_get', payload: { canvas_id: canvasId } }, 'canvas', identity)
      .then((loaded) => {
        if (live)
          setView((current) =>
            current
              ? {
                  ...current,
                  notes: loaded.notes,
                  missing_note_ids: loaded.missing_note_ids,
                }
              : current,
          );
      })
      .catch((failure) => live && onError(message(failure)));
    return () => {
      live = false;
    };
  }, [canvasId, identity, graphRefresh, onError]);
  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    const measure = () => {
      const box = element.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) setStageSize({ width: box.width, height: box.height });
    };
    measure();
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setStageSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [draft?.id]);
  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    const listener = (event: WheelEvent) => wheelHandler.current(event);
    element.addEventListener('wheel', listener, { passive: false });
    return () => element.removeEventListener('wheel', listener);
  }, [draft?.id]);
  useEffect(() => {
    if (!draft || stageSize.width <= 0 || stageSize.height <= 0) return;
    if (initialFitDone.current) {
      if (restoredViewport.current)
        setViewport(
          restoreCanvasViewport(restoredViewport.current, stageSize.width, stageSize.height),
        );
      return;
    }
    const initial = restoredViewport.current
      ? restoreCanvasViewport(restoredViewport.current, stageSize.width, stageSize.height)
      : initialCanvasViewport(draft.nodes, stageSize.width, stageSize.height);
    if (viewportReadFailed.current) setViewport(initial);
    else moveViewport(initial);
    initialFitDone.current = true;
  }, [draft?.id, stageSize.height, stageSize.width]);
  const categoryGraphNodes = useMemo(
    () =>
      (graph?.nodes ?? []).filter(
        (node) => !collectionId || !!categoryMembers?.has(node.object_id ?? node.id),
      ),
    [collectionId, graph, categoryMembers],
  );
  const categoryNodeIds = useMemo(
    () =>
      collectionId
        ? (categoryMembers ?? new Set<string>())
        : new Set(categoryGraphNodes.map((node) => node.id)),
    [collectionId, categoryMembers, categoryGraphNodes],
  );
  const noteEdges = useMemo(() => canvasNoteEdges(graph?.edges ?? [], graph?.nodes ?? []), [graph]);
  const categoryEdges = useMemo(() => {
    if (!collectionId) return noteEdges;
    return noteEdges.filter(
      (edge) => categoryNodeIds.has(edge.source_id) && categoryNodeIds.has(edge.target_id),
    );
  }, [categoryNodeIds, collectionId, noteEdges]);
  const displayedNodes = useMemo(
    () => (draft?.nodes ?? []).filter((node) => !collectionId || categoryNodeIds.has(node.note_id)),
    [categoryNodeIds, collectionId, draft?.nodes],
  );
  useEffect(() => {
    if (selectedNode && collectionId && !categoryNodeIds.has(selectedNode)) setSelectedNode(null);
  }, [categoryNodeIds, collectionId, selectedNode]);
  useEffect(() => {
    if (selectedEdge && !categoryEdges.some((edge) => edge === selectedEdge)) setSelectedEdge(null);
  }, [categoryEdges, selectedEdge]);
  useEffect(() => {
    let live = true;
    setRelationFields([]);
    setRelationDetailError('');
    if (selectedEdge?.kind === 'relation') {
      void call(
        { kind: 'notes_outgoing_relations', payload: { id: selectedEdge.source_id } },
        'outgoing_relations',
        identity,
      )
        .then((items) => {
          if (live)
            setRelationFields([
              ...new Set(
                items
                  .filter((item) => item.target_id === selectedEdge.target_id)
                  .map((item) => item.field_key),
              ),
            ]);
        })
        .catch((failure) => {
          if (live) setRelationDetailError(message(failure));
        });
    }
    return () => {
      live = false;
    };
  }, [selectedEdge, identity.local_instance_id, identity.workspace_id]);
  const notes = useMemo(
    () => new Map((view?.notes ?? []).map((note) => [note.note.id, note])),
    [view],
  );
  const missing = useMemo(() => new Set(view?.missing_note_ids ?? []), [view]);
  const edgeLines = useMemo(
    () => canvasRelationLines(displayedNodes, categoryEdges),
    [categoryEdges, displayedNodes],
  );
  const canvasEdges = useMemo(() => edgeLines.map((line) => line.edge), [edgeLines]);
  const unplacedTargets = useMemo(
    () => unplacedRelationTargets(displayedNodes, categoryEdges, categoryGraphNodes),
    [categoryEdges, categoryGraphNodes, displayedNodes],
  );
  const graphTitles = useMemo(
    () => new Map(categoryGraphNodes.map((node) => [node.id, node.title])),
    [categoryGraphNodes],
  );
  const visibleNodes = useMemo(
    () =>
      draft
        ? visibleCanvasNodes(displayedNodes, {
            ...viewport,
            width: stageSize.width,
            height: stageSize.height,
          })
        : [],
    [displayedNodes, draft, stageSize.height, stageSize.width, viewport],
  );
  const onScreenNodeCount = useMemo(
    () =>
      draft
        ? visibleCanvasNodes(
            displayedNodes,
            {
              ...viewport,
              width: stageSize.width,
              height: stageSize.height,
            },
            0,
          ).length
        : 0,
    [displayedNodes, draft, stageSize.height, stageSize.width, viewport],
  );
  const selected = selectedNode ? notes.get(selectedNode) : null;
  async function save(next: Canvas | null = draftRef.current) {
    if (!next || controller.getSnapshot().busy || controller.getSnapshot().pending) return false;
    setBusy(true);
    onError('');
    try {
      const operation = controller.run({
        kind: 'canvases_save',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            canvas_id: next.id,
            base_row_version: next.row_version,
            name: next.name,
            config_schema: next.config_schema,
            nodes: next.nodes,
            is_pinned: next.is_pinned,
            pin_position: next.pin_position,
          },
        },
      });
      pendingSave.current = operation;
      const result = await operation;
      if (!result || !('canvas' in result)) return false;
      setDraft(result.canvas);
      draftRef.current = result.canvas;
      savedCanvas.current = result.canvas;
      onTitle(result.canvas.name);
      setView((current) => (current ? { ...current, canvas: result.canvas } : current));
      setGraphRefresh((value) => value + 1);
      onNotice(t('布局已保存。', 'Layout saved.'));
      onChanged();
      return true;
    } catch (failure) {
      onError(message(failure));
      return false;
    } finally {
      pendingSave.current = null;
      setBusy(false);
    }
  }
  function removeCanvas() {
    if (!draft) return;
    setRemoveConfirm(true);
  }

  async function removeCanvasNow() {
    if (!draft || busy) return;
    setBusy(true);
    try {
      const result = await controller.run({
        kind: 'canvases_remove',
        payload: {
          command_id: crypto.randomUUID(),
          request: { canvas_id: draft.id, base_row_version: draft.row_version },
        },
      });
      if (!result) return;
      onDeleted();
    } catch (failure) {
      onError(message(failure));
    } finally {
      setBusy(false);
    }
  }
  async function reloadCanvas() {
    setBusy(true);
    try {
      const loaded = await call(
        { kind: 'canvases_get', payload: { canvas_id: canvasId } },
        'canvas',
        identity,
      );
      setView(loaded);
      setDraft(loaded.canvas);
      draftRef.current = loaded.canvas;
      savedCanvas.current = loaded.canvas;
      onTitle(loaded.canvas.name);
      controller.clearError();
      onError('');
      setGraphRefresh((value) => value + 1);
      setCategoryReload((value) => value + 1);
    } catch (failure) {
      onError(message(failure));
    } finally {
      setBusy(false);
    }
  }
  async function saveCopy() {
    const current = draftRef.current;
    if (!current || busy) return;
    const suffix = t(' 副本', ' copy');
    const result = await controller.run({
      kind: 'canvases_save',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          canvas_id: null,
          base_row_version: null,
          name: [...current.name].slice(0, 300 - suffix.length).join('') + suffix,
          config_schema: current.config_schema,
          nodes: current.nodes,
          is_pinned: false,
          pin_position: null,
        },
      },
    });
    if (result && 'canvas' in result) {
      onSavedAs(result.canvas);
      onChanged();
    }
  }
  async function openOriginal(noteId: string, targetBlockId: string | null = null) {
    if (busy) return;
    const current = draftRef.current;
    if (
      current &&
      view &&
      JSON.stringify(current) !== JSON.stringify(view.canvas) &&
      !(await save(current))
    )
      return;
    if (targetBlockId) {
      try {
        await onOpenTarget({
          target_id: noteId,
          target_block_id: targetBlockId,
          target_revision_id: null,
        });
      } catch (failure) {
        onError(message(failure));
      }
    } else onOpenNote(noteId);
  }

  async function updateCanvasDetails(
    change: Partial<Pick<Canvas, 'name' | 'is_pinned' | 'pin_position'>>,
  ) {
    const current = draftRef.current;
    if (!current || busy) return;
    const next = { ...current, ...change };
    setDraft(next);
    draftRef.current = next;
    await save(next);
  }
  leaveAction.current = async () => {
    if (pendingSave.current) await pendingSave.current;
    if (controller.getSnapshot().pending || controller.getSnapshot().busy)
      throw new Error(
        t(
          '画布操作仍需确认，请先在画布中确认原操作。',
          'The canvas operation needs confirmation. Confirm the original operation before leaving.',
        ),
      );
    const current = draftRef.current;
    if (
      current &&
      savedCanvas.current &&
      JSON.stringify(current) !== JSON.stringify(savedCanvas.current)
    ) {
      if (!(await save(current)))
        throw new Error(
          t(
            '布局尚未保存，请重新读取或另存画布副本后再离开。',
            'The layout is not saved. Reload it or save a canvas copy before leaving.',
          ),
        );
    }
  };
  async function searchNotes(cursor: string | null = null) {
    if (categoryLoading) return;
    const sequence = ++searchSequence.current;
    setSearching(true);
    onError('');
    try {
      const page = await call(
        {
          kind: 'notes_list',
          payload: {
            limit: 20,
            cursor,
            query: query.trim() || null,
            trashed: false,
            filter: collectionId
              ? { collection_id: collectionId, inbox_state: null, tag_ids: [], object_types: [] }
              : null,
            sort: null,
          },
        },
        'notes',
        identity,
      );
      if (sequence !== searchSequence.current) return;
      setSearchResults((old) => (cursor ? [...(old ?? []), ...page.items] : page.items));
      setSearchCursor(page.next_cursor);
    } catch (failure) {
      if (sequence === searchSequence.current) onError(message(failure));
    } finally {
      if (sequence === searchSequence.current) setSearching(false);
    }
  }
  async function searchRelationTargets(cursor: string | null = null) {
    if (!selectedNode || categoryLoading) return;
    const sequence = ++relationSequence.current;
    setRelationSearching(true);
    onError('');
    try {
      const page = await call(
        {
          kind: 'notes_list',
          payload: {
            limit: 20,
            cursor,
            query: relationQuery.trim() || null,
            trashed: false,
            filter: collectionId
              ? { collection_id: collectionId, inbox_state: null, tag_ids: [], object_types: [] }
              : null,
            sort: null,
          },
        },
        'notes',
        identity,
      );
      if (sequence !== relationSequence.current) return;
      const targets = page.items.filter((note) => note.id !== selectedNode);
      setRelationTargets((old) => (cursor ? [...(old ?? []), ...targets] : targets));
      setRelationCursor(page.next_cursor);
    } catch (failure) {
      if (sequence === relationSequence.current) onError(message(failure));
    } finally {
      if (sequence === relationSequence.current) setRelationSearching(false);
    }
  }
  async function linkRelation(target: NoteSummary) {
    if (!selectedNode || relationBusy || busy) return;
    setRelationBusy(true);
    onError('');
    try {
      await withSavedNote(selectedNode, async () => {
        const note = await call(
          { kind: 'notes_get', payload: { note_id: selectedNode } },
          'note',
          identity,
        );
        const current = readObjectRelations(note.document, 'related');
        if (current.error) throw new Error(current.error);
        if (current.values.some((value) => value.id === target.id)) {
          onNotice(t('这两篇笔记已经建立关系。', 'These notes are already related.'));
          return;
        }
        const document = setFrontmatterRelations(note.document, 'related', [
          ...current.values,
          { kind: 'note', id: target.id, title: target.title || t('未命名笔记', 'Untitled note') },
        ]);
        const result = await controller.run({
          kind: 'notes_save',
          payload: {
            command_id: crypto.randomUUID(),
            note: {
              note_id: note.id,
              base_revision_id: note.revision_id,
              title: note.title,
              document_schema: note.schema_version,
              document,
            },
          },
        });
        if (!result) return;
        setRelationTargets(null);
        setRelationQuery('');
        setGraphRefresh((value) => value + 1);
        onNotice(
          t(
            '已建立关系，可以在图谱和相关笔记中查看。',
            'Relation created. Find it in the graph and related notes.',
          ),
        );
        onChanged();
      });
    } catch (failure) {
      onError(message(failure));
    } finally {
      setRelationBusy(false);
    }
  }
  async function prepareRelationRemoval(field: string) {
    if (!selectedEdge || busy || relationBusy) return;
    const edge = selectedEdge;
    setRelationBusy(true);
    onError('');
    try {
      let source: Note | null = null;
      await withSavedNote(edge.source_id, async () => {
        source = await call(
          { kind: 'notes_get', payload: { note_id: edge.source_id } },
          'note',
          identity,
        );
      });
      if (source)
        setRemoveRelation({
          note: source,
          field,
          targetId: edge.target_id,
          targetTitle: graphTitles.get(edge.target_id) ?? t('目标笔记', 'Target note'),
        });
    } catch (failure) {
      onError(message(failure));
    } finally {
      setRelationBusy(false);
    }
  }
  async function confirmRelationRemoval() {
    const removal = removeRelation;
    if (!removal || busy || relationBusy) return;
    setRemoveRelation(null);
    setRelationBusy(true);
    onError('');
    try {
      await withSavedNote(removal.note.id, async () => {
        const relations = readObjectRelations(removal.note.document, removal.field);
        if (relations.error) throw new Error(relations.error);
        const document = setFrontmatterRelations(
          removal.note.document,
          removal.field,
          relations.values.filter((target) => target.id !== removal.targetId),
        );
        const result = await controller.run({
          kind: 'notes_save',
          payload: {
            command_id: crypto.randomUUID(),
            note: {
              note_id: removal.note.id,
              base_revision_id: removal.note.revision_id,
              title: removal.note.title,
              document_schema: removal.note.schema_version,
              document,
            },
          },
        });
        if (!result) return;
        setSelectedEdge(null);
        setGraphRefresh((value) => value + 1);
        onChanged();
        onNotice(
          t(
            '属性关系已移除，笔记内容仍保留。',
            'Property relation removed; both notes are retained.',
          ),
        );
      });
    } catch (failure) {
      onError(message(failure));
    } finally {
      setRelationBusy(false);
    }
  }
  async function addNote(note: NoteSummary) {
    if (!draft || busy) return;
    if (collectionId && !categoryNodeIds.has(note.id)) {
      onNotice(
        t(
          '当前分类视图只显示该分类的对象；清除分类筛选后才能加入其他笔记。',
          'This category view only shows its own objects; clear the category filter to add other notes.',
        ),
      );
      return;
    }
    if (draft.nodes.some((node) => node.note_id === note.id)) {
      onNotice(t('这条笔记已经在画布中。', 'That note is already on this canvas.'));
      return;
    }
    if (draft.nodes.length >= 1000) {
      onError(t('画布最多支持 1000 个节点。', 'A canvas supports at most 1,000 nodes.'));
      return;
    }
    const index = draft.nodes.length;
    const node: CanvasNode = {
      note_id: note.id,
      x: 80 + (index % 4) * 280,
      y: 80 + Math.floor(index / 4) * 190,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      z_index: index,
    };
    const next = { ...draft, nodes: [...draft.nodes, node] };
    setDraft(next);
    draftRef.current = next;
    setSearchResults(null);
    setSelectedNode(note.id);
    await save(next);
  }
  async function addUnplacedTargets() {
    if (!draft || !unplacedTargets.length || busy) return;
    const capacity = 1000 - draft.nodes.length;
    if (capacity <= 0) {
      onError(t('画布最多支持 1000 个节点。', 'A canvas supports at most 1,000 nodes.'));
      return;
    }
    const targets = unplacedTargets.slice(0, capacity);
    const start = draft.nodes.length;
    const nodes: CanvasNode[] = targets.map((target, index) => ({
      note_id: target.id,
      x: 80 + ((start + index) % 4) * 280,
      y: 80 + Math.floor((start + index) / 4) * 190,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
      z_index: start + index,
    }));
    const next = { ...draft, nodes: [...draft.nodes, ...nodes] };
    setDraft(next);
    draftRef.current = next;
    if (!(await save(next))) return;
    if (targets.length < unplacedTargets.length)
      onNotice(
        t(
          `已加入 ${targets.length} 个关联对象；画布节点已达上限。`,
          `Added ${targets.length} related objects; the canvas is now at its node limit.`,
        ),
      );
  }
  function removeNode(noteId: string) {
    if (!draft || busy) return;
    const next = { ...draft, nodes: draft.nodes.filter((node) => node.note_id !== noteId) };
    setDraft(next);
    draftRef.current = next;
    if (selectedNode === noteId) setSelectedNode(null);
    void save(next);
  }
  function updateNode(noteId: string, x: number, y: number) {
    updateNodeBox(noteId, { x, y });
  }
  function finishNodeGesture() {
    const changed = !!resize.current || !!drag.current;
    resize.current = null;
    drag.current = null;
    pan.current = null;
    const next = draftRef.current;
    if (
      changed &&
      next &&
      view &&
      next.nodes !== view.canvas.nodes &&
      JSON.stringify(next.nodes) !== JSON.stringify(view.canvas.nodes)
    )
      void save(next);
  }
  function updateNodeBox(
    noteId: string,
    box: Partial<Pick<CanvasNode, 'x' | 'y' | 'width' | 'height'>>,
  ) {
    setDraft((current) => {
      if (!current) return current;
      const next = {
        ...current,
        nodes: current.nodes.map((node) => (node.note_id === noteId ? { ...node, ...box } : node)),
      };
      draftRef.current = next;
      return next;
    });
  }
  function resizeNodeBox(
    noteId: string,
    direction: ResizeDirection,
    start: { x: number; y: number; width: number; height: number },
    dx: number,
    dy: number,
  ) {
    let { x, y, width, height } = start;
    if (direction.includes('e')) width = Math.max(NODE_MIN_WIDTH, start.width + dx);
    if (direction.includes('s')) height = Math.max(NODE_MIN_HEIGHT, start.height + dy);
    if (direction.includes('w')) {
      width = Math.max(NODE_MIN_WIDTH, start.width - dx);
      x = start.x + start.width - width;
    }
    if (direction.includes('n')) {
      height = Math.max(NODE_MIN_HEIGHT, start.height - dy);
      y = start.y + start.height - height;
    }
    updateNodeBox(noteId, { x, y, width, height });
  }
  function applyScale(nextScale: (current: number) => number) {
    moveViewport((current) => {
      const scale = Math.min(2, Math.max(0.25, nextScale(current.scale)));
      const centerX = stageSize.width / 2;
      const centerY = stageSize.height / 2;
      const worldX = (centerX - current.x) / current.scale;
      const worldY = (centerY - current.y) / current.scale;
      return {
        x: centerX - worldX * scale,
        y: centerY - worldY * scale,
        scale,
      };
    });
  }
  function zoom(delta: number) {
    applyScale((current) => current + delta);
  }
  function zoomTo(scale: number) {
    applyScale(() => scale);
  }
  function fit() {
    if (draft && stageSize.width > 0 && stageSize.height > 0) {
      moveViewport(fitCanvasViewport(displayedNodes, stageSize.width, stageSize.height));
      initialFitDone.current = true;
    }
  }
  function moveViewport(change: CanvasTransform | ((current: CanvasTransform) => CanvasTransform)) {
    const next = typeof change === 'function' ? change(viewportRef.current) : change;
    viewportRef.current = next;
    setViewport(next);
    if (stageSize.width <= 0 || stageSize.height <= 0) return;
    try {
      const preference = canvasViewportPreference(next, stageSize.width, stageSize.height);
      localStorage.setItem(viewportKey, JSON.stringify(preference));
      restoredViewport.current = preference;
      viewportReadFailed.current = false;
      setViewportError('');
    } catch (failure) {
      setViewportError(message(failure));
    }
  }
  wheelHandler.current = (event) => {
    event.preventDefault();
    event.stopPropagation();
    const box = stage.current?.getBoundingClientRect();
    if (!box) return;
    const x = event.clientX - box.left;
    const y = event.clientY - box.top;
    moveViewport((current) => {
      const scale = Math.min(2, Math.max(0.25, current.scale * Math.exp(-event.deltaY * 0.0012)));
      return {
        scale,
        x: x - ((x - current.x) / current.scale) * scale,
        y: y - ((y - current.y) / current.scale) * scale,
      };
    });
  };
  function exportJson() {
    if (!draft) return;
    const payload = {
      format: 'spanseek-canvas-v1',
      canvas: draft,
      notes: draft.nodes.map((node) => ({
        note_id: node.note_id,
        title: notes.get(node.note_id)?.note.title ?? null,
        exists: notes.has(node.note_id),
      })),
      relations: canvasRelationLines(draft.nodes, noteEdges).map((line) => line.edge),
      relations_complete: !!graph && !graph.truncated && !graphError,
      layout_saved: JSON.stringify(draft) === JSON.stringify(savedCanvas.current),
      lossy: ['note bodies', 'rich formatting', 'attachments', 'full history'],
      exported_at: new Date().toISOString(),
    };
    downloadText(
      `${safeFilename(draft.name)}.json`,
      JSON.stringify(payload, null, 2),
      'application/json;charset=utf-8',
    );
  }
  function exportSvg() {
    if (!draft) return;
    const svg = exportedSvg;
    downloadText(`${safeFilename(draft.name)}.svg`, svg, 'image/svg+xml;charset=utf-8');
  }
  async function printCanvas() {
    if (document.fullscreenElement) await document.exitFullscreen();
    const content = document.querySelector<HTMLImageElement>('body > .canvas-print-only img');
    if (!content) return;
    const source = content.src;
    await content.decode();
    if (!content.isConnected || content.src !== source) return;
    window.print();
  }
  const exportedSvg = useMemo(
    () =>
      draft
        ? canvasSvg(draft, notes, noteEdges, {
            labels: {
              missing: t('引用已失效', 'Missing reference'),
              pending: t('笔记预览待更新', 'Note preview pending'),
              untitled: t('未命名笔记', 'Untitled note'),
              trashed: t('笔记在回收站中', 'This note is in trash'),
              partial: t('部分关系未载入', 'Some relations are not loaded'),
            },
            missingIds: missing,
            relationsComplete: !!graph && !graph.truncated && !graphError,
          })
        : '',
    [draft, notes, noteEdges, missing, graph, graphError, t],
  );
  if (loading) return <PageLoadFailure error={null} retry={() => setSelectedNode(null)} />;
  if (!draft)
    return (
      <PageLoadFailure
        error={t('画布不存在。', 'Canvas not found.')}
        retry={() => window.location.reload()}
      />
    );
  const selectedView = selected ? { note: selected.note, metadata: selected.metadata } : null;
  return (
    <section
      className="canvas-editor"
      data-canvas-id={canvasId}
      aria-label={t('画布编辑', 'Canvas editor')}
    >
      {mutation.code === 'revision_conflict' && !mutation.pending && (
        <div className="canvas-banner" role="status">
          <span>
            {t(
              '来源笔记已有更新，请重新读取关系后再操作。',
              'The source note has changed. Reload its relations before continuing.',
            )}
          </span>
          <button
            className="secondary-button"
            onClick={() => {
              controller.clearError();
              setGraphRefresh((value) => value + 1);
            }}
          >
            {t('重新读取关系', 'Reload relations')}
          </button>
        </div>
      )}
      {mutation.code === 'metadata_conflict' && !mutation.pending && (
        <div className="canvas-banner" role="status">
          <span>
            {t(
              '当前布局仍保留，可以重新读取或另存副本。',
              'Your layout is retained. Reload the saved version or save a copy.',
            )}
          </span>
          <button
            className="secondary-button"
            disabled={busy}
            onClick={() => setReloadConfirm(true)}
          >
            {t('重新读取画布', 'Reload canvas')}
          </button>
          <button className="secondary-button" disabled={busy} onClick={() => void saveCopy()}>
            {t('另存画布副本', 'Save canvas copy')}
          </button>
        </div>
      )}
      {viewportError && (
        <div className="canvas-banner error" role="alert">
          <span>
            {t('画布位置暂未保存：', 'Canvas position was not saved: ')}
            {viewportError}
          </span>
          <button className="text-button" onClick={() => moveViewport(viewportRef.current)}>
            {t('重试保存位置', 'Retry saving position')}
          </button>
        </div>
      )}
      <header className="canvas-editor-heading">
        <div>
          <p>
            {collectionId
              ? t(
                  `显示 ${displayedNodes.length} / ${draft.nodes.length} 个节点`,
                  `Showing ${displayedNodes.length} of ${draft.nodes.length} nodes`,
                )
              : t(`${draft.nodes.length} 个节点`, `${draft.nodes.length} nodes`)}
          </p>
        </div>
        <div className="canvas-actions">
          <button className="secondary-button" disabled={busy} onClick={() => void save()}>
            <Save size={14} />
            {t('保存布局', 'Save layout')}
          </button>
          <PopupMenu
            label={t('画布操作', 'Canvas actions')}
            placement="bottom-start"
            disabled={busy}
            trigger={<MoreHorizontal size={17} />}
            items={[
              {
                id: 'rename',
                label: t('重命名画布', 'Rename canvas'),
                icon: <Pencil size={15} />,
                onSelect: () => setRenameOpen(true),
              },
              {
                id: 'pin',
                label: draft.is_pinned
                  ? t('取消置顶', 'Unpin canvas')
                  : t('置顶画布', 'Pin canvas'),
                icon: draft.is_pinned ? <PinOff size={15} /> : <Pin size={15} />,
                onSelect: () =>
                  void updateCanvasDetails({
                    is_pinned: !draft.is_pinned,
                    pin_position: draft.is_pinned ? null : 0,
                  }),
              },
              {
                id: 'json',
                disabled: !graph || !!graphError,
                separatorBefore: true,
                label: t('导出布局（JSON）', 'Export layout (JSON)'),
                icon: <FileJson size={15} />,
                detail: t(
                  '完整画布的布局、节点引用和已载入关系；不含正文',
                  'Full canvas layout, note references and loaded relations; no note bodies',
                ),
                onSelect: exportJson,
              },
              {
                id: 'svg',
                disabled: !graph || !!graphError,
                label: t('导出图像（SVG）', 'Export image (SVG)'),
                icon: <Download size={15} />,
                detail: t(
                  '完整画布的标题、预览与已载入关系，不含完整正文',
                  'Full canvas titles, previews and loaded relations; no full note bodies',
                ),
                onSelect: exportSvg,
              },
              {
                id: 'print',
                disabled: !graph || !!graphError,
                label: t('打印画布', 'Print canvas'),
                icon: <Printer size={15} />,
                detail: t(
                  '打印完整画布的标题、预览和已载入关系',
                  'Print full canvas titles, previews and loaded relations',
                ),
                onSelect: () => void printCanvas().catch((failure) => onError(message(failure))),
              },
              {
                id: 'delete',
                label: t('删除画布', 'Delete canvas'),
                icon: <Trash2 size={15} />,
                separatorBefore: true,
                detail: t('只删除画布，保留原笔记', 'Remove the canvas and keep original notes'),
                onSelect: removeCanvas,
              },
            ]}
          />
        </div>
      </header>
      <div className="canvas-toolbar">
        <div className="canvas-note-search">
          <Search size={15} />
          <input
            value={query}
            placeholder={
              collectionId
                ? t('在当前分类中搜索笔记…', 'Search notes in this category…')
                : t('搜索笔记并添加到画布…', 'Search notes to add…')
            }
            onChange={(event) => {
              searchSequence.current++;
              setQuery(event.target.value);
              setSearchResults(null);
              setSearchCursor(null);
              setSearching(false);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void searchNotes();
              }
            }}
          />
          <button
            className="secondary-button"
            disabled={searching || categoryLoading}
            onClick={() => void searchNotes()}
          >
            {searching ? <LoaderCircle size={14} className="spin" /> : <Search size={14} />}
            {t('查找', 'Find')}
          </button>
        </div>
        <div className="canvas-zoom">
          <button
            className="icon-button"
            aria-label={
              fullscreen.active
                ? t('退出全屏', 'Exit fullscreen')
                : t('全屏画布', 'Fullscreen canvas')
            }
            onClick={() => void fullscreen.toggle().catch((failure) => onError(message(failure)))}
          >
            {fullscreen.active ? <Minimize size={16} /> : <Maximize size={16} />}
          </button>
          <button
            className="icon-button"
            aria-label={t('缩小', 'Zoom out')}
            onClick={() => zoom(-0.1)}
          >
            <ZoomOut size={16} />
          </button>
          <span aria-live="polite">{Math.round(viewport.scale * 100)}%</span>
          <button
            className="icon-button"
            aria-label={t('放大', 'Zoom in')}
            onClick={() => zoom(0.1)}
          >
            <ZoomIn size={16} />
          </button>
          <button
            className="canvas-zoom-actual"
            aria-label={t('实际大小（100%）', 'Actual size (100%)')}
            onClick={() => zoomTo(1)}
          >
            100%
          </button>
          <button className="icon-button" aria-label={t('适应画布', 'Fit canvas')} onClick={fit}>
            <Maximize2 size={16} />
          </button>
        </div>
      </div>
      {searchResults && (
        <div
          className="canvas-search-results"
          role="listbox"
          aria-label={t('搜索结果', 'Search results')}
        >
          {searchResults.map((note) => (
            <button
              key={note.id}
              role="option"
              disabled={busy || searching}
              onClick={() => void addNote(note)}
            >
              <strong>{note.title || t('未命名笔记', 'Untitled note')}</strong>
              <small>{note.preview || t('没有正文', 'No content')}</small>
            </button>
          ))}
          {!searchResults.length && (
            <p className="org-small-note">{t('没有匹配的笔记。', 'No matching notes.')}</p>
          )}
          {searchCursor && (
            <button
              className="text-button"
              disabled={searching}
              onClick={() => void searchNotes(searchCursor)}
            >
              {t('更多笔记', 'More notes')}
            </button>
          )}
          <button className="text-button" onClick={() => setSearchResults(null)}>
            {t('关闭结果', 'Close results')}
          </button>
        </div>
      )}
      <div
        className="canvas-stage"
        ref={stage}
        onPointerDown={(event) => {
          if (
            event.target !== event.currentTarget &&
            !(event.target as HTMLElement).classList.contains('canvas-grid')
          )
            return;
          pan.current = {
            startX: event.clientX,
            startY: event.clientY,
            x: viewport.x,
            y: viewport.y,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (resize.current) {
            const active = resize.current;
            resizeNodeBox(
              active.nodeId,
              active.direction,
              {
                x: active.nodeX,
                y: active.nodeY,
                width: active.nodeWidth,
                height: active.nodeHeight,
              },
              (event.clientX - active.startX) / viewport.scale,
              (event.clientY - active.startY) / viewport.scale,
            );
            return;
          }
          if (drag.current) {
            const deltaX = (event.clientX - drag.current.startX) / viewport.scale;
            const deltaY = (event.clientY - drag.current.startY) / viewport.scale;
            updateNode(
              drag.current.nodeId,
              drag.current.nodeX + deltaX,
              drag.current.nodeY + deltaY,
            );
            return;
          }
          if (pan.current) {
            moveViewport((current) => ({
              ...current,
              x: pan.current!.x + event.clientX - pan.current!.startX,
              y: pan.current!.y + event.clientY - pan.current!.startY,
            }));
          }
        }}
        onPointerUp={finishNodeGesture}
        onPointerCancel={finishNodeGesture}
      >
        <div className="canvas-grid" />
        <div
          className="canvas-world"
          style={
            {
              transform: `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.scale})`,
              '--canvas-inverse-scale': 1 / viewport.scale,
            } as CSSProperties
          }
        >
          <svg
            className="canvas-edge-layer"
            role="img"
            aria-label={t(
              '画布中已建立的知识关系',
              'Established knowledge relations on this canvas',
            )}
          >
            <defs>
              <marker
                id="canvas-relation-arrow"
                markerWidth="7"
                markerHeight="7"
                refX="6"
                refY="3.5"
                orient="auto"
              >
                <path d="M0 0 L7 3.5 L0 7 Z" fill="currentColor" />
              </marker>
            </defs>
            {edgeLines.map(({ edge, x1, y1, x2, y2 }) => {
              const active = selectedNode === edge.source_id || selectedNode === edge.target_id;
              return (
                <line
                  key={canvasRelationKey(edge)}
                  className={`canvas-edge is-${edge.kind}${active ? ' is-active' : ''}${selectedEdge === edge ? ' is-selected' : ''}`}
                  x1={x1}
                  y1={y1}
                  x2={x2}
                  y2={y2}
                  markerEnd="url(#canvas-relation-arrow)"
                  onClick={(event) => {
                    event.stopPropagation();
                    setSelectedEdge(edge);
                  }}
                >
                  <title>{`${graphTitles.get(edge.source_id) ?? edge.source_id} → ${graphTitles.get(edge.target_id) ?? edge.target_id} · ${edge.context}`}</title>
                </line>
              );
            })}
          </svg>
          {visibleNodes.map((node) => {
            const note = notes.get(node.note_id);
            const missingNote = missing.has(node.note_id);
            const trashedNote = !!note?.metadata.trashed;
            return (
              <article
                data-note-id={node.note_id}
                className={`canvas-node${missingNote ? ' missing' : ''}${trashedNote ? ' is-trashed' : ''}${selectedNode === node.note_id ? ' selected' : ''}`}
                key={node.note_id}
                style={{
                  left: node.x,
                  top: node.y,
                  width: node.width,
                  height: node.height,
                  zIndex: node.z_index + 1,
                }}
                onPointerDown={(event) => {
                  if (busy) return;
                  if ((event.target as HTMLElement).closest('button, a, input, textarea, select'))
                    return;
                  event.stopPropagation();
                  setSelectedNode(node.note_id);
                  drag.current = {
                    nodeId: node.note_id,
                    startX: event.clientX,
                    startY: event.clientY,
                    nodeX: node.x,
                    nodeY: node.y,
                  };
                  event.currentTarget.setPointerCapture(event.pointerId);
                }}
                onClick={() => setSelectedNode(node.note_id)}
              >
                {missingNote ? (
                  <>
                    <strong>{t('引用已失效', 'Missing reference')}</strong>
                    <p>
                      {t(
                        '原笔记已不存在。可以移除此节点，其他内容不受影响。',
                        'The original note no longer exists. Removing this node keeps other content intact.',
                      )}
                    </p>
                    <button
                      className="text-button"
                      onClick={(event) => {
                        event.stopPropagation();
                        removeNode(node.note_id);
                      }}
                    >
                      {t('移除此节点', 'Remove node')}
                    </button>
                  </>
                ) : !note ? (
                  <>
                    <strong>
                      {graphTitles.get(node.note_id) ?? t('笔记预览待更新', 'Note preview pending')}
                    </strong>
                    <p>
                      {t(
                        '保存布局后可读取笔记预览。',
                        'Save the layout to load this note preview.',
                      )}
                    </p>
                    <button
                      className="text-button"
                      disabled={busy}
                      onClick={(event) => {
                        event.stopPropagation();
                        setGraphRefresh((value) => value + 1);
                      }}
                    >
                      {t('重新读取预览', 'Reload preview')}
                    </button>
                  </>
                ) : trashedNote ? (
                  <>
                    <strong>{note.note.title || t('未命名笔记', 'Untitled note')}</strong>
                    <p>
                      {t(
                        '笔记在回收站中，恢复后可继续使用。',
                        'This note is in trash. Restore it to continue using it.',
                      )}
                    </p>
                    <button
                      className="text-button"
                      onClick={(event) => {
                        event.stopPropagation();
                        onOpenTrash();
                      }}
                    >
                      {t('前往回收站', 'Open trash')}
                    </button>
                  </>
                ) : (
                  <>
                    <div className="canvas-node-heading">
                      <strong>{note.note.title || t('未命名笔记', 'Untitled note')}</strong>
                      <button
                        className="icon-button"
                        aria-label={t('打开原文', 'Open original')}
                        disabled={busy}
                        onClick={(event) => {
                          event.stopPropagation();
                          void openOriginal(note.note.id);
                        }}
                      >
                        <ExternalLink size={13} />
                      </button>
                    </div>
                    <p>{note.note.preview || t('没有正文。', 'No content yet.')}</p>
                    <small>
                      {viewStatusLabel(note.note.inbox_state, t)} ·{' '}
                      {displayDate(note.note.updated_at, language)}
                    </small>
                    {note.metadata.tags.length > 0 && (
                      <div className="canvas-node-tags">
                        <TagPills
                          tags={note.metadata.tags}
                          limit={3}
                          moreLabel={(count) => t(`另有 ${count} 个标签`, `${count} more tags`)}
                        />
                      </div>
                    )}
                  </>
                )}
                {selectedNode === node.note_id && (
                  <>
                    <span className="canvas-node-drag-affordance" aria-hidden="true">
                      <Move size={11} />
                      {t('拖动', 'Drag')}
                    </span>
                    {RESIZE_DIRECTIONS.map((direction) => (
                      <button
                        key={direction}
                        type="button"
                        disabled={busy}
                        className={`canvas-resize-handle is-${direction}`}
                        aria-label={t(`调整节点尺寸 ${direction}`, `Resize node ${direction}`)}
                        title={t('拖动调整尺寸', 'Drag to resize')}
                        onPointerDown={(event) => {
                          event.preventDefault();
                          event.stopPropagation();
                          resize.current = {
                            nodeId: node.note_id,
                            direction,
                            startX: event.clientX,
                            startY: event.clientY,
                            nodeX: node.x,
                            nodeY: node.y,
                            nodeWidth: node.width,
                            nodeHeight: node.height,
                          };
                          event.currentTarget.setPointerCapture(event.pointerId);
                        }}
                        onClick={(event) => event.stopPropagation()}
                        onKeyDown={(event) => {
                          const step = event.shiftKey ? 24 : 8;
                          const horizontal = direction.includes('e')
                            ? event.key === 'ArrowRight'
                              ? 1
                              : event.key === 'ArrowLeft'
                                ? -1
                                : 0
                            : direction.includes('w')
                              ? event.key === 'ArrowLeft'
                                ? 1
                                : event.key === 'ArrowRight'
                                  ? -1
                                  : 0
                              : 0;
                          const vertical = direction.includes('s')
                            ? event.key === 'ArrowDown'
                              ? 1
                              : event.key === 'ArrowUp'
                                ? -1
                                : 0
                            : direction.includes('n')
                              ? event.key === 'ArrowUp'
                                ? 1
                                : event.key === 'ArrowDown'
                                  ? -1
                                  : 0
                              : 0;
                          if (!horizontal && !vertical) return;
                          event.preventDefault();
                          resizeNodeBox(
                            node.note_id,
                            direction,
                            node,
                            horizontal * step,
                            vertical * step,
                          );
                          window.setTimeout(() => void save(draftRef.current), 0);
                        }}
                      />
                    ))}
                  </>
                )}
              </article>
            );
          })}
        </div>
        <div className="canvas-hud">
          <span>
            {t(
              `可见 ${onScreenNodeCount} / ${draft.nodes.length}`,
              `${onScreenNodeCount} visible / ${draft.nodes.length}`,
            )}
          </span>
          <span>
            {t(
              `缩放 ${Math.round(viewport.scale * 100)}%`,
              `Zoom ${Math.round(viewport.scale * 100)}%`,
            )}
          </span>
          {missing.size > 0 && (
            <span className="warning">
              {t(`${missing.size} 个引用已失效`, `${missing.size} missing references`)}
            </span>
          )}
        </div>
      </div>
      {graphError && (
        <p className="org-small-note canvas-graph-warning" role="status">
          {t('知识关系暂时无法读取：', 'Knowledge relations are temporarily unavailable: ')}
          {graphError}
        </p>
      )}
      {categoryLoading && (
        <p className="org-small-note" role="status">
          {t('正在读取分类范围…', 'Loading category members…')}
        </p>
      )}
      {categoryError && (
        <p role="alert">
          {categoryError}{' '}
          <button className="text-button" onClick={() => setCategoryReload((value) => value + 1)}>
            {t('重新读取分类', 'Reload category')}
          </button>
        </p>
      )}
      {graph?.truncated && (
        <p className="org-small-note" role="status">
          {t(
            '知识关系达到读取上限，这里显示已载入的部分关系。',
            'The relation query reached its limit. This canvas shows the relations that were loaded.',
          )}
        </p>
      )}
      <div className="canvas-lower-panels">
        {selectedNode && (
          <div className="canvas-selection-strip">
            <span>
              {t('已选', 'Selected')}{' '}
              <strong>
                {selected?.note.title ||
                  graphTitles.get(selectedNode) ||
                  t('画布节点', 'Canvas node')}
              </strong>
            </span>
            {selected?.note && !selected.metadata.trashed && !missing.has(selected.note.id) && (
              <button
                type="button"
                className="text-button"
                aria-expanded={relationOpen}
                onClick={() => setRelationOpen((current) => !current)}
              >
                <Link2 size={14} />
                {relationOpen
                  ? t('收起关系编辑', 'Close relation editor')
                  : t('建立关系', 'Create relation')}
              </button>
            )}
            <button
              type="button"
              className="text-button"
              disabled={busy}
              onClick={() => removeNode(selectedNode)}
            >
              <Trash2 size={14} />
              {t('从画布移除', 'Remove from canvas')}
            </button>
          </div>
        )}
        {relationOpen && selected?.note && !selected.metadata.trashed && (
          <section
            className="canvas-relation-creator"
            aria-label={t('建立知识关系', 'Create a knowledge relation')}
          >
            <p>
              {t(
                '选择另一篇笔记，建立可在图谱和反链中追溯的关系。',
                'Choose another note to create a relation you can trace through the graph and backlinks.',
              )}
            </p>
            <div className="canvas-relation-search">
              <Search size={15} aria-hidden="true" />
              <input
                value={relationQuery}
                placeholder={t('搜索要关联的笔记…', 'Search for a note to relate…')}
                aria-label={t('搜索要关联的笔记', 'Search for a note to relate')}
                onChange={(event) => {
                  relationSequence.current++;
                  setRelationQuery(event.target.value);
                  setRelationTargets(null);
                  setRelationCursor(null);
                  setRelationSearching(false);
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void searchRelationTargets();
                  }
                }}
              />
              <button
                type="button"
                className="secondary-button"
                disabled={relationBusy || relationSearching || categoryLoading}
                onClick={() => void searchRelationTargets()}
              >
                {relationBusy ? <LoaderCircle size={14} className="spin" /> : <Search size={14} />}
                {t('查找', 'Find')}
              </button>
            </div>
            {relationTargets && (
              <div
                className="canvas-relation-results"
                role="listbox"
                aria-label={t('建立关系候选', 'Relation candidates')}
              >
                {relationTargets.map((target) => (
                  <button
                    key={target.id}
                    type="button"
                    role="option"
                    disabled={busy || relationBusy || relationSearching}
                    onClick={() => void linkRelation(target)}
                  >
                    <strong>{target.title || t('未命名笔记', 'Untitled note')}</strong>
                    <small>{target.preview || t('没有正文', 'No content')}</small>
                  </button>
                ))}
                {!relationTargets.length && (
                  <p className="org-small-note">{t('没有匹配的笔记。', 'No matching notes.')}</p>
                )}
                {relationCursor && (
                  <button
                    className="text-button"
                    disabled={relationSearching}
                    onClick={() => void searchRelationTargets(relationCursor)}
                  >
                    {t('更多笔记', 'More notes')}
                  </button>
                )}
                <button
                  className="text-button"
                  type="button"
                  onClick={() => setRelationTargets(null)}
                >
                  {t('关闭结果', 'Close results')}
                </button>
              </div>
            )}
          </section>
        )}
        {unplacedTargets.length > 0 && (
          <div className="canvas-unplaced-relations">
            <span>
              <Link2 size={14} aria-hidden="true" />
              {t(
                `还有 ${unplacedTargets.length} 个关联笔记未加入画布`,
                `${unplacedTargets.length} related notes are not on this canvas`,
              )}
            </span>
            <button
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={() => void addUnplacedTargets()}
            >
              <Plus size={14} />
              {t('加入画布', 'Add to canvas')}
            </button>
          </div>
        )}
        {canvasEdges.length > 0 && (
          <section
            className="canvas-relations"
            aria-label={t('画布知识关系', 'Canvas knowledge relations')}
          >
            <header>
              <strong>{t('画布中的知识关系', 'Knowledge relations on this canvas')}</strong>
              <span>{canvasEdges.length}</span>
            </header>
            {edgeLines.map(({ edge }) => (
              <button
                type="button"
                key={canvasRelationKey(edge)}
                className={selectedEdge === edge ? 'active' : ''}
                onClick={() => setSelectedEdge(edge)}
              >
                <span>
                  {graphTitles.get(edge.source_id) ?? edge.source_id} →{' '}
                  {graphTitles.get(edge.target_id) ?? edge.target_id}
                </span>
                <small>
                  {relationKindLabel(edge.kind, t)} · {edge.context}
                </small>
              </button>
            ))}
            {selectedEdge && (
              <div className="canvas-relation-detail">
                <strong>
                  {graphTitles.get(selectedEdge.source_id) ?? selectedEdge.source_id}
                  {' → '}
                  {graphTitles.get(selectedEdge.target_id) ?? selectedEdge.target_id}
                </strong>
                <p>{selectedEdge.context || t('没有关系上下文。', 'No relation context.')}</p>
                <p>
                  {selectedEdge.kind === 'relation'
                    ? t(
                        '来源：笔记属性。打开来源笔记可查看或修改。',
                        'Source: note properties. Open the source note to inspect or edit them.',
                      )
                    : selectedEdge.kind === 'block_reference'
                      ? t(
                          '来源：正文块引用，可回到源笔记的对应块。',
                          'Source: body block reference; open the source note at that block.',
                        )
                      : t(
                          '来源：正文引用，可回到源笔记原文。',
                          'Source: body reference; open the source note to trace it.',
                        )}
                </p>
                <div className="canvas-relation-actions">
                  <button
                    className="text-button"
                    onClick={() => void openOriginal(selectedEdge.source_id)}
                  >
                    {t('打开来源笔记', 'Open source note')}
                  </button>
                  <button
                    className="text-button"
                    onClick={() =>
                      void openOriginal(selectedEdge.target_id, selectedEdge.target_block_id)
                    }
                  >
                    {selectedEdge.target_block_id
                      ? t('打开引用位置', 'Open referenced block')
                      : t('打开目标笔记', 'Open target note')}
                  </button>
                  {relationFields.map((field) => (
                    <button
                      key={field}
                      className="text-button"
                      disabled={busy || relationBusy}
                      onClick={() => void prepareRelationRemoval(field)}
                    >
                      {t(`移除属性关系「${field}」`, `Remove property relation “${field}”`)}
                    </button>
                  ))}
                </div>
                {relationDetailError && <p role="alert">{relationDetailError}</p>}
                <button type="button" className="text-button" onClick={() => setSelectedEdge(null)}>
                  {t('关闭关系详情', 'Close relation detail')}
                </button>
              </div>
            )}
          </section>
        )}
        {selectedView && !selectedView.metadata.trashed && (
          <ViewMetadataEditor
            item={selectedView}
            identity={identity}
            onChanged={() => {
              onChanged();
              void call(
                { kind: 'canvases_get', payload: { canvas_id: canvasId } },
                'canvas',
                identity,
              )
                .then((loaded) => {
                  const current = draftRef.current;
                  const dirty =
                    current &&
                    savedCanvas.current &&
                    JSON.stringify(current) !== JSON.stringify(savedCanvas.current);
                  setView(loaded);
                  savedCanvas.current = loaded.canvas;
                  if (!dirty) {
                    setDraft(loaded.canvas);
                    draftRef.current = loaded.canvas;
                  }
                })
                .catch((failure) => onError(message(failure)));
            }}
            onOpenNote={(id) => void openOriginal(id)}
          />
        )}
      </div>
      <CanvasPrintable title={draft.name} svg={exportedSvg} />
      {renameOpen && (
        <ActionConfirmDialog
          title={t('重命名画布', 'Rename canvas')}
          description={t('为画布设置一个便于识别的名称。', 'Give this canvas a recognizable name.')}
          confirmLabel={t('保存名称', 'Save name')}
          input={{
            label: t('画布名称', 'Canvas name'),
            initialValue: draft.name,
            validate: (value) =>
              !value.trim()
                ? t('请输入画布名称。', 'Enter a canvas name.')
                : [...value.trim()].length > 300
                  ? t('画布名称不能超过 300 个字符。', 'Use at most 300 characters.')
                  : null,
          }}
          onCancel={() => setRenameOpen(false)}
          onConfirm={(value) => {
            setRenameOpen(false);
            void updateCanvasDetails({ name: value.trim() });
          }}
        />
      )}
      {removeRelation && (
        <ActionConfirmDialog
          title={t('移除这条属性关系？', 'Remove this property relation?')}
          description={t(
            `从「${removeRelation.note.title}」的「${removeRelation.field}」属性移除到「${removeRelation.targetTitle}」的关系。两篇笔记和画布节点都会保留。`,
            `Remove the relation to “${removeRelation.targetTitle}” from “${removeRelation.note.title}”, property “${removeRelation.field}”. Both notes and canvas nodes will be retained.`,
          )}
          confirmLabel={t('移除关系', 'Remove relation')}
          onCancel={() => setRemoveRelation(null)}
          onConfirm={() => void confirmRelationRemoval()}
        />
      )}
      {reloadConfirm && (
        <ActionConfirmDialog
          title={t('重新读取已保存的画布？', 'Reload the saved canvas?')}
          description={t(
            '这会替换尚未保存的布局变动。需要保留当前布局时，请取消并另存画布副本。',
            'This replaces unsaved layout changes. Cancel and save a copy to keep your current layout.',
          )}
          confirmLabel={t('重新读取', 'Reload')}
          onCancel={() => setReloadConfirm(false)}
          onConfirm={() => {
            setReloadConfirm(false);
            void reloadCanvas();
          }}
        />
      )}
      {removeConfirm && draft && (
        <ActionConfirmDialog
          title={t('删除这个画布？', 'Delete this canvas?')}
          description={t(
            '只删除画布布局；笔记和正文不会删除。删除后无法从画布页恢复。',
            'Only the canvas layout is deleted; notes and bodies remain. This cannot be undone from the canvas page.',
          )}
          confirmLabel={t('确认删除画布', 'Delete canvas')}
          tone="danger"
          onCancel={() => setRemoveConfirm(false)}
          onConfirm={() => {
            setRemoveConfirm(false);
            void removeCanvasNow();
          }}
        />
      )}
    </section>
  );
}

function CanvasPrintable({ title, svg }: { title: string; svg: string }) {
  const { t } = useI18n();
  return createPortal(
    <div className="canvas-print-only">
      <header>
        <strong>{title}</strong>
        <span>
          {t(
            '完整画布 · 标题、预览和已载入关系',
            'Full canvas · titles, previews and loaded relations',
          )}
        </span>
      </header>
      <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`} alt={title} />
    </div>,
    document.body,
  );
}

export function canvasExportSummary(canvas: Canvas, notes: Map<string, CanvasNote>) {
  return {
    format: 'spanseek-canvas-v1',
    canvas,
    note_count: canvas.nodes.length,
    existing_notes: [...notes.keys()].length,
    lossy: ['note bodies', 'rich formatting', 'attachments', 'full history'],
  };
}
