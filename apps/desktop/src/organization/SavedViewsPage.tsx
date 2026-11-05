import { Select } from '../ui/Select';
import { tagToneClass } from '../ui/tag-color';
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react';
import {
  ArrowLeft,
  CalendarDays,
  Check,
  Clock3,
  Download,
  LayoutGrid,
  LayoutList,
  MoreHorizontal,
  Pencil,
  Pin,
  Play,
  Plus,
  Printer,
  RefreshCw,
  Trash2,
  X,
} from 'lucide-react';
import type {
  BoardGroupBy,
  CollectionDetail,
  InboxState,
  NoteSort,
  NotesFilter,
  SavedView,
  SaveView,
  Tag,
  ViewConfig,
  ViewLayout,
  ViewNote,
  ViewProperty,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import {
  ActionNotice,
  PageLoadFailure,
  QueryFeedback,
  type OrganizationPageProps,
} from './OrganizationControls';
import { usePageActions } from './page-hooks';
import { PageActions, pageKey, type OrganizationCommand } from './page-actions';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import type { CursorPage } from '../layout/pagination.ts';
import { PageFrame } from '../layout/PageFrame';
import {
  ViewLayoutView,
  ViewMetadataEditor,
  ViewPrintHeader,
  collectCustomPropertyKeys,
  layoutLabel,
  propertyLabel,
} from './view-layouts';
import { downloadText, safeFilename, viewCsv } from './view-export';
import {
  objectTypeCatalog,
  objectTypeQueryTokens,
  type ObjectTypeDefinition,
} from './object-types';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { PopupMenu } from '../ui/PopupMenu';
import './organization-pages.css';

export function SavedViewsPage(
  props: OrganizationPageProps & { customTypes?: readonly ObjectTypeDefinition[] },
) {
  return <SavedViewsScope key={pageKey(props.identity, 'views')} {...props} />;
}
function SavedViewsScope(
  props: OrganizationPageProps & { customTypes?: readonly ObjectTypeDefinition[] },
) {
  const loaded = usePageActions(props.identity, 'views');
  return loaded.controller ? (
    <SavedViewsContent {...props} controller={loaded.controller} />
  ) : (
    <PageLoadFailure error={loaded.error} retry={loaded.retry} />
  );
}
/** The directory and each result query retain their own cursor page and scroll position. */
function useViewPaging<T>(
  scope: string,
  navigationKey: string,
  read: (cursor: string | null, size: number) => Promise<CursorPage<T>>,
  visible = true,
) {
  const [page, setPage] = usePageNavigationState(`${navigationKey}.page`, 1);
  const [size, setSize] = usePageNavigationState(`${navigationKey}.size`, 10);
  const [scroll, setScroll] = usePageNavigationState(`${navigationKey}.scroll`, 0);
  const scrollElement = useRef<HTMLDivElement>(null);
  const resume = useRef({ scope: '', pending: false });
  const [resuming, setResuming] = useState(false);
  const list = usePagedList<T>(scope, read, size);
  useEffect(() => {
    if (!list.page || list.loading || resume.current.scope === scope) return;
    resume.current = { scope, pending: page > 1 };
    if (page <= 1) {
      setResuming(false);
      return;
    }
    setResuming(true);
    void list.goTo(page).finally(() => {
      if (resume.current.scope === scope) {
        resume.current.pending = false;
        setResuming(false);
      }
    });
  }, [scope, list.page, list.loading]);
  useEffect(() => {
    if (
      list.page &&
      !list.loading &&
      !list.error &&
      !resume.current.pending &&
      resume.current.scope === scope
    )
      setPage(list.index);
  }, [scope, list.page, list.index, list.loading, list.error, resuming]);
  useLayoutEffect(() => {
    if (visible && scrollElement.current && !list.loading && !resuming)
      scrollElement.current.scrollTop = scroll;
  }, [visible, list.page, list.loading, resuming]);
  const reset = () => {
    setPage(1);
    setScroll(0);
  };
  const pager = {
    ...list,
    loading: list.loading || resuming,
    setSize: (value: number) => {
      reset();
      setSize(value);
      return list.setSize(value);
    },
    next: () => {
      setScroll(0);
      return list.next();
    },
    previous: () => {
      setScroll(0);
      return list.previous();
    },
    goTo: (value: number) => {
      setScroll(0);
      return list.goTo(value);
    },
  };
  return {
    pager,
    scrollElement,
    reset,
    rememberScroll: () => {
      if (scrollElement.current && !list.loading && !resuming)
        setScroll(scrollElement.current.scrollTop);
    },
  };
}

function SavedViewsContent({
  identity,
  onOpenNote,
  onChanged,
  controller,
  customTypes = [],
}: OrganizationPageProps & {
  controller: PageActions;
  customTypes?: readonly ObjectTypeDefinition[];
}) {
  const { t } = useI18n();
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const [selected, setSelected] = usePageNavigationState<string | null>('views.selected', null);
  const [view, setView] = useState<SavedView | null>(null);
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [confirmReload, setConfirmReload] = useState(false);
  const [query, setQuery] = usePageNavigationState('views.query', '');
  const [pinnedOnly, setPinnedOnly] = usePageNavigationState('views.pinned', false);
  const [refresh, setRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const noticeElement = useRef<HTMLDivElement>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const key = pageKey(identity, 'views');
  const locked = state.busy || !!state.pending;
  useEffect(() => {
    if ((state.error || state.pending) && noticeElement.current?.querySelector('[role="alert"]'))
      noticeElement.current.parentElement?.scrollTo({ top: 0 });
  }, [state.error, state.pending]);
  const showingList = !selected && !creating;
  const directory = useViewPaging<SavedView>(
    `${key}/${JSON.stringify(query)}/${pinnedOnly}/${refresh}/${state.generation}`,
    'views.list',
    (cursor, size) =>
      call(
        {
          kind: 'views_list',
          payload: { limit: size, cursor, pinned_only: pinnedOnly, query: query || null },
        },
        'views',
        identity,
      ),
    showingList,
  );
  const views = directory.pager;
  useEffect(() => {
    let current = true;
    setView(null);
    setError(null);
    setLoading(!!selected);
    if (selected)
      void call({ kind: 'views_get', payload: { view_id: selected } }, 'view', identity)
        .then((value) => {
          if (current) setView(value);
        })
        .catch((failure) => {
          if (current) setError(message(failure));
        })
        .finally(() => {
          if (current) setLoading(false);
        });
    return () => {
      current = false;
    };
  }, [selected, state.generation, refresh]);
  function select(id: string) {
    directory.rememberScroll();
    setSelected(id);
    setCreating(false);
    setEditing(false);
    setDeleting(false);
  }
  function returnToList() {
    setSelected(null);
    setCreating(false);
    setEditing(false);
    setDeleting(false);
  }
  async function act(command?: OrganizationCommand) {
    const pending = command ?? state.pending?.command;
    const result = await controller.run(command);
    if (result && live.current) {
      if ('view' in result) {
        if (pending?.kind === 'views_save' && pending.payload.request.view_id === null) {
          setSelected(result.view.id);
          setQuery('');
          setPinnedOnly(false);
          directory.reset();
        }
        if (result.removed) setSelected((id) => (id === result.view.id ? null : id));
      }
      setCreating(false);
      setEditing(false);
      setDeleting(false);
      onChanged?.();
    }
    return result;
  }
  function refreshDefinition() {
    controller.clearError();
    setEditing(false);
    setConfirmReload(false);
    setRefresh((value) => value + 1);
  }
  const notice = (
    <div className="saved-view-notice" ref={noticeElement}>
      <ActionNotice state={state} retry={() => void act()} />
    </div>
  );
  const title = creating
    ? t('新建视图', 'New view')
    : editing
      ? t('编辑视图', 'Edit view')
      : selected
        ? view?.name || t('保存的视图', 'Saved view')
        : t('保存的视图', 'Saved views');
  return (
    <PageFrame
      className={`org-page saved-views-page${selected ? ' is-view-open' : ''}${creating || editing ? ' is-view-form' : ''}`}
      title={title}
      description={
        showingList
          ? t(
              '把常用的筛选和阅读方式保存下来。',
              'Keep useful filters and reading layouts close at hand.',
            )
          : undefined
      }
      actions={
        showingList ? (
          <button className="primary-button" disabled={locked} onClick={() => setCreating(true)}>
            <Plus size={16} />
            {t('新建视图', 'New view')}
          </button>
        ) : (
          <>
            <button
              className="secondary-button saved-view-back"
              disabled={locked}
              aria-label={
                creating || editing
                  ? t('取消编辑', 'Cancel editing')
                  : t('返回视图列表', 'Back to views')
              }
              title={
                creating || editing
                  ? t('取消编辑', 'Cancel editing')
                  : t('返回视图列表', 'Back to views')
              }
              onClick={() => (editing ? setEditing(false) : returnToList())}
            >
              <ArrowLeft size={16} />
              <span className="saved-view-action-label">
                {creating || editing ? t('取消', 'Cancel') : t('返回视图列表', 'Back to views')}
              </span>
            </button>
            {view && !editing && !creating && (
              <>
                <button
                  className="secondary-button"
                  disabled={locked}
                  aria-label={t('编辑视图', 'Edit view')}
                  onClick={() => setEditing(true)}
                >
                  <Pencil size={15} />
                  <span className="saved-view-action-label">{t('编辑视图', 'Edit view')}</span>
                </button>
                <PopupMenu
                  label={t('视图操作', 'View actions')}
                  placement="bottom-start"
                  trigger={<MoreHorizontal size={18} />}
                  disabled={locked}
                  items={[
                    {
                      id: 'pin',
                      label: view.is_pinned
                        ? t('取消固定视图', 'Unpin view')
                        : t('固定视图', 'Pin view'),
                      icon: <Pin size={15} />,
                      checked: view.is_pinned,
                      onSelect: () =>
                        void act({
                          kind: 'views_save',
                          payload: {
                            command_id: crypto.randomUUID(),
                            request: viewRequest(view, {
                              is_pinned: !view.is_pinned,
                              pin_position: view.is_pinned ? null : 0,
                            }),
                          },
                        }),
                    },
                    {
                      id: 'reload',
                      label: t('重新读取视图', 'Reload view'),
                      icon: <RefreshCw size={15} />,
                      onSelect: refreshDefinition,
                    },
                    {
                      id: 'remove',
                      label: t('移除保存的视图', 'Remove saved view'),
                      icon: <Trash2 size={15} />,
                      separatorBefore: true,
                      onSelect: () => setDeleting(true),
                    },
                  ]}
                />
              </>
            )}
          </>
        )
      }
      toolbar={
        showingList ? (
          <div className="saved-views-list-tools">
            <input
              type="search"
              aria-label={t('查找视图', 'Find views')}
              value={query}
              placeholder={t('输入视图名称…', 'View name…')}
              onChange={(event) => {
                directory.reset();
                setQuery(event.target.value);
              }}
            />
            <label className="org-check">
              <input
                type="checkbox"
                checked={pinnedOnly}
                onChange={(event) => {
                  directory.reset();
                  setPinnedOnly(event.target.checked);
                }}
              />
              {t('仅显示固定', 'Pinned only')}
            </label>
            <button
              className="icon-button"
              disabled={views.loading}
              aria-label={t('刷新视图', 'Refresh views')}
              onClick={() => setRefresh((n) => n + 1)}
            >
              <RefreshCw size={16} />
            </button>
          </div>
        ) : undefined
      }
      footer={showingList ? <ListPagination list={views} /> : undefined}
    >
      {showingList ? (
        <div
          className="saved-views-list"
          ref={directory.scrollElement}
          onScroll={directory.rememberScroll}
          aria-label={t('视图列表', 'View list')}
        >
          {notice}
          <QueryFeedback
            state={views}
            reload={() => void views.refresh()}
            pager={null}
            empty={t(
              '还没有符合条件的视图。新建一个视图，或调整筛选条件。',
              'No saved views match. Create a view or adjust the filters.',
            )}
          />
          <div className="saved-views-list-items">
            {views.page?.items.map((item) => (
              <button
                className="org-index-item"
                key={item.id}
                data-view-id={item.id}
                disabled={views.loading}
                onClick={() => select(item.id)}
              >
                <LayoutGlyph layout={item.layout} />
                <span>
                  <strong>{item.name}</strong>
                  <small>{layoutLabel(item.layout, t)}</small>
                </span>
                {item.is_pinned && <Pin size={13} aria-label={t('已固定', 'Pinned')} />}
              </button>
            ))}
          </div>
        </div>
      ) : creating || (view && editing) ? (
        <div className="saved-view-form-scroll">
          {notice}
          <ViewForm
            key={creating ? 'new' : view!.id}
            identity={identity}
            customTypes={customTypes}
            initial={creating ? null : view}
            disabled={locked}
            onSave={(request) =>
              void act({
                kind: 'views_save',
                payload: { command_id: crypto.randomUUID(), request },
              })
            }
            onCancel={() => (creating ? returnToList() : setEditing(false))}
            onReload={() => setConfirmReload(true)}
          />
        </div>
      ) : loading ? (
        <div className="saved-view-message">
          {notice}
          <p className="org-loading" role="status">
            {t('正在读取视图…', 'Loading view…')}
          </p>
        </div>
      ) : error ? (
        <div className="saved-view-message">
          {notice}
          <div className="org-inline-error" role="alert">
            <p>{error}</p>
            <button className="secondary-button" onClick={refreshDefinition}>
              {t('重新读取', 'Reload')}
            </button>
          </div>
        </div>
      ) : view ? (
        <ViewResults
          key={view.id}
          identity={identity}
          view={view}
          refresh={refresh}
          onOpenNote={onOpenNote}
          notice={notice}
        />
      ) : null}
      {deleting && view && (
        <ActionConfirmDialog
          title={t('移除保存的视图？', 'Remove saved view?')}
          description={t(
            '只移除这个保存的视图，笔记和集合都会保留。',
            'Only this saved view will be removed. Your notes and collections stay intact.',
          )}
          confirmLabel={t('确认移除视图', 'Remove saved view')}
          tone="danger"
          onCancel={() => setDeleting(false)}
          onConfirm={() => {
            setDeleting(false);
            void act({
              kind: 'views_remove',
              payload: {
                command_id: crypto.randomUUID(),
                request: { view_id: view.id, base_row_version: view.row_version },
              },
            });
          }}
        />
      )}
      {confirmReload && (
        <ActionConfirmDialog
          title={t('载入当前版本？', 'Reload current version?')}
          description={t(
            '尚未保存的视图修改会被放弃，载入后可以按最新版本重新编辑。',
            'Unsaved view changes will be discarded. You can edit again from the latest saved version.',
          )}
          confirmLabel={t('载入当前版本', 'Reload current version')}
          onCancel={() => setConfirmReload(false)}
          onConfirm={refreshDefinition}
        />
      )}
    </PageFrame>
  );
}

function LayoutGlyph({ layout }: { layout: ViewLayout }) {
  if (layout === 'board') return <LayoutGrid size={17} />;
  if (layout === 'calendar') return <CalendarDays size={17} />;
  if (layout === 'timeline') return <Clock3 size={17} />;
  return <LayoutList size={17} />;
}
function viewRequest(view: SavedView, overrides: Partial<SaveView> = {}): SaveView {
  return {
    view_id: view.id,
    base_row_version: view.row_version,
    name: view.name,
    filter_schema: 1,
    filter: structuredClone(view.filter),
    sort: view.sort,
    layout: view.layout,
    config_schema: view.config_schema,
    config: structuredClone(view.config),
    is_pinned: view.is_pinned,
    pin_position: view.pin_position,
    ...overrides,
  };
}
function defaultViewConfig(): ViewConfig {
  return {
    visible_properties: ['inbox_state', 'tags', 'updated_at'],
    board_group_by: 'inbox_state',
    calendar_month: null,
    custom_properties: [],
    date_property: null,
  };
}

const MAX_CUSTOM_PROPERTIES = 12;
const MAX_VIEW_COLUMNS = 16;
const RESERVED_CUSTOM_PROPERTY_KEYS = new Set([
  'title',
  'tags',
  'aliases',
  'inbox_state',
  'archived',
  'search_enabled',
  'updated_at',
]);

function ViewForm({
  identity,
  initial,
  customTypes,
  disabled,
  onSave,
  onCancel,
  onReload,
}: {
  identity: OrganizationPageProps['identity'];
  initial: SavedView | null;
  customTypes: readonly ObjectTypeDefinition[];
  disabled: boolean;
  onSave: (request: SaveView) => void;
  onCancel: () => void;
  onReload: () => void;
}) {
  const { t } = useI18n();
  const [draft, setDraft] = useState<SaveView>(() =>
    initial
      ? viewRequest(initial)
      : {
          view_id: null,
          base_row_version: null,
          name: '',
          filter_schema: 1,
          filter: {
            tag_ids: [],
            object_types: [],
            inbox_state: null,
            collection_id: null,
            archived: false,
            search_enabled: null,
          },
          sort: 'updated_desc',
          layout: 'list',
          config_schema: 1,
          config: defaultViewConfig(),
          is_pinned: false,
          pin_position: null,
        },
  );
  const [customKeyDraft, setCustomKeyDraft] = useState('');
  const [customPropertyError, setCustomPropertyError] = useState<string | null>(null);
  const [discoveredKeys, setDiscoveredKeys] = useState<string[]>(
    initial?.config.custom_properties ?? [],
  );
  const [propertyDiscovery, setPropertyDiscovery] = useState<'idle' | 'loading' | 'failed'>('idle');
  useEffect(() => {
    if (!initial) return;
    let live = true;
    setPropertyDiscovery('loading');
    void call(
      { kind: 'views_run', payload: { view_id: initial.id, limit: 100, cursor: null } },
      'view_run',
      identity,
    )
      .then((result) => {
        if (!live) return;
        setDiscoveredKeys(
          [
            ...new Set([
              ...initial.config.custom_properties,
              ...collectCustomPropertyKeys(result.notes.items),
            ]),
          ].sort((left, right) => left.localeCompare(right)),
        );
        setPropertyDiscovery('idle');
      })
      .catch(() => live && setPropertyDiscovery('failed'));
    return () => {
      live = false;
    };
  }, [identity, initial]);
  const setFilter = (change: Partial<NotesFilter>) =>
    setDraft((old) => ({ ...old, filter: { ...old.filter, ...change } }));
  const setConfig = (change: Partial<ViewConfig>) =>
    setDraft((old) => ({ ...old, config: { ...old.config, ...change } }));
  const toggleProperty = (property: ViewProperty) =>
    setDraft((old) => {
      const selected = old.config.visible_properties.includes(property);
      if (selected && old.config.visible_properties.length === 1) return old;
      return {
        ...old,
        config: {
          ...old.config,
          visible_properties: selected
            ? old.config.visible_properties.filter((value) => value !== property)
            : [...old.config.visible_properties, property],
        },
      };
    });
  const availableCustomProperties = discoveredKeys.filter(
    (key) => !draft.config.custom_properties.includes(key),
  );
  function addCustomProperty(rawKey: string) {
    const key = rawKey.trim();
    if (!key) return;
    if ([...key].length > 64 || /[\u0000-\u001f\u007f-\u009f]/.test(key)) {
      setCustomPropertyError(
        t(
          '属性名需为 1–64 个字符，且不能包含控制字符。',
          'Use 1–64 characters without control characters.',
        ),
      );
      return;
    }
    if (RESERVED_CUSTOM_PROPERTY_KEYS.has(key.toLocaleLowerCase('en-US'))) {
      setCustomPropertyError(
        t(
          '该名称已用于内置整理属性，请换一个 frontmatter 键名。',
          'That name is reserved for built-in metadata. Choose another frontmatter key.',
        ),
      );
      return;
    }
    if (draft.config.custom_properties.includes(key)) {
      setCustomPropertyError(t('该属性已经添加。', 'That property is already selected.'));
      return;
    }
    if (draft.config.custom_properties.length >= MAX_CUSTOM_PROPERTIES) {
      setCustomPropertyError(
        t(
          '每个视图最多显示 12 个 frontmatter 属性。',
          'A view can show at most 12 frontmatter properties.',
        ),
      );
      return;
    }
    if (
      draft.config.visible_properties.length + draft.config.custom_properties.length >=
      MAX_VIEW_COLUMNS
    ) {
      setCustomPropertyError(
        t(
          '内置与自定义属性合计最多 16 列。',
          'Built-in and custom properties can total at most 16 columns.',
        ),
      );
      return;
    }
    setDraft((old) => ({
      ...old,
      config: {
        ...old.config,
        custom_properties: [...old.config.custom_properties, key],
      },
    }));
    setCustomKeyDraft('');
    setCustomPropertyError(null);
  }
  function removeCustomProperty(key: string) {
    setDraft((old) => ({
      ...old,
      config: {
        ...old.config,
        custom_properties: old.config.custom_properties.filter((candidate) => candidate !== key),
        date_property: old.config.date_property === key ? null : old.config.date_property,
      },
    }));
    setCustomPropertyError(null);
  }
  return (
    <form
      className="org-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (draft.name.trim())
          onSave({
            ...draft,
            name: draft.name.trim(),
            pin_position: draft.is_pinned ? (draft.pin_position ?? 0) : null,
          });
      }}
    >
      <fieldset disabled={disabled}>
        <h2 className="sr-only">
          {initial ? t('编辑视图', 'Edit view') : t('新建视图', 'New view')}
        </h2>
        <label>
          {t('视图名称', 'View name')}
          <input
            autoFocus
            maxLength={300}
            value={draft.name}
            onChange={(event) => setDraft({ ...draft, name: event.target.value })}
          />
        </label>
        <div className="org-form-grid">
          <div className="choice-field">
            {t('整理状态', 'Organization state')}
            <Select
              aria-label={t('整理状态', 'Organization state')}
              value={draft.filter.inbox_state ?? 'all'}
              onValueChange={(nextValue) =>
                setFilter({
                  inbox_state: nextValue === 'all' ? null : (nextValue as InboxState),
                })
              }
            >
              <option value="all">{t('所有状态', 'All states')}</option>
              <option value="pending">{t('收件箱 · 待整理', 'Inbox · Unprocessed')}</option>
              <option value="snoozed">{t('稍后整理', 'Later')}</option>
              <option value="processed">{t('已整理', 'Organized')}</option>
            </Select>
          </div>
          <div className="choice-field">
            {t('归档状态', 'Archive state')}
            <Select
              aria-label={t('归档状态', 'Archive state')}
              value={draft.filter.archived ? 'archived' : 'active'}
              onValueChange={(nextValue) => setFilter({ archived: nextValue === 'archived' })}
            >
              <option value="active">{t('未归档笔记', 'Unarchived notes')}</option>
              <option value="archived">{t('已归档笔记', 'Archived notes')}</option>
            </Select>
          </div>
          <div className="choice-field">
            {t('搜索收录', 'Search visibility')}
            <Select
              aria-label={t('搜索收录', 'Search visibility')}
              value={
                draft.filter.search_enabled == null ? 'all' : String(draft.filter.search_enabled)
              }
              onValueChange={(nextValue) =>
                setFilter({
                  search_enabled: nextValue === 'all' ? null : nextValue === 'true',
                })
              }
            >
              <option value="all">{t('不限', 'Any')}</option>
              <option value="true">{t('允许被搜索', 'Included in search')}</option>
              <option value="false">{t('不参与搜索', 'Excluded from search')}</option>
            </Select>
          </div>
          <div className="choice-field">
            {t('笔记排序', 'Note order')}
            <Select
              aria-label={t('笔记排序', 'Note order')}
              value={draft.sort}
              onValueChange={(nextValue) => setDraft({ ...draft, sort: nextValue as NoteSort })}
            >
              <option value="updated_desc">{t('最近编辑', 'Last edited')}</option>
              <option value="updated_asc">{t('最早编辑', 'Oldest edited')}</option>
              <option value="title_asc">{t('标题 A → Z', 'Title A → Z')}</option>
              <option value="title_desc">{t('标题 Z → A', 'Title Z → A')}</option>
            </Select>
          </div>
          <div className="choice-field">
            {t('显示方式', 'Layout')}
            <Select
              aria-label={t('显示方式', 'Layout')}
              value={draft.layout}
              onValueChange={(nextValue) => setDraft({ ...draft, layout: nextValue as ViewLayout })}
            >
              <option value="list">{t('列表', 'List')}</option>
              <option value="table">{t('表格', 'Table')}</option>
              <option value="board">{t('看板', 'Board')}</option>
              <option value="calendar">{t('日历', 'Calendar')}</option>
              <option value="timeline">{t('时间线', 'Timeline')}</option>
            </Select>
          </div>
          <div className="org-wide view-config-box">
            <h3>{t('视图属性', 'View properties')}</h3>
            <p className="org-small-note">
              {t(
                '属性只读取整理元数据，不会复制或修改正文。',
                'Properties read organization metadata and never copy or rewrite the note body.',
              )}
            </p>
            <div className="view-property-choices">
              {(
                [
                  'inbox_state',
                  'tags',
                  'archived',
                  'search_enabled',
                  'updated_at',
                ] as ViewProperty[]
              ).map((property) => (
                <label className="org-check" key={property}>
                  <input
                    type="checkbox"
                    checked={draft.config.visible_properties.includes(property)}
                    onChange={() => toggleProperty(property)}
                  />
                  {propertyLabel(property, t)}
                </label>
              ))}
            </div>
            <div className="view-custom-property-config">
              <div className="view-custom-property-heading">
                <strong>{t('Frontmatter 自定义属性', 'Frontmatter custom properties')}</strong>
                <span>
                  {draft.config.custom_properties.length}/{MAX_CUSTOM_PROPERTIES}
                </span>
              </div>
              <p className="org-small-note">
                {t(
                  '从笔记首个 YAML frontmatter 读取只读属性；正文和原 YAML 仍是唯一真值。保存后可自动发现当前匹配笔记中的键名。',
                  'Read-only properties come from each note’s first YAML frontmatter block. The body and YAML remain the source of truth; after saving, keys are discovered from matching notes.',
                )}
              </p>
              {draft.config.custom_properties.length > 0 && (
                <div
                  className="view-custom-property-selected"
                  aria-label={t('已选自定义属性', 'Selected custom properties')}
                >
                  {draft.config.custom_properties.map((key) => (
                    <span className="view-custom-property-chip" key={key}>
                      <span>{key}</span>
                      <button
                        type="button"
                        aria-label={t(`移除属性 ${key}`, `Remove property ${key}`)}
                        onClick={() => removeCustomProperty(key)}
                      >
                        <X size={12} />
                      </button>
                    </span>
                  ))}
                </div>
              )}
              <div className="view-custom-property-add">
                <input
                  list="saved-view-custom-property-options"
                  maxLength={64}
                  value={customKeyDraft}
                  placeholder={t('输入 frontmatter 键名…', 'Enter a frontmatter key…')}
                  onChange={(event) => {
                    setCustomKeyDraft(event.target.value);
                    setCustomPropertyError(null);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                      event.preventDefault();
                      addCustomProperty(customKeyDraft);
                    }
                  }}
                />
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!customKeyDraft.trim()}
                  onClick={() => addCustomProperty(customKeyDraft)}
                >
                  {t('添加属性', 'Add property')}
                </button>
                <datalist id="saved-view-custom-property-options">
                  {availableCustomProperties.map((key) => (
                    <option value={key} key={key} />
                  ))}
                </datalist>
              </div>
              {availableCustomProperties.length > 0 && (
                <div className="view-custom-property-suggestions">
                  <span>{t('发现的键名', 'Discovered keys')}</span>
                  {availableCustomProperties.map((key) => (
                    <button type="button" key={key} onClick={() => addCustomProperty(key)}>
                      {key}
                    </button>
                  ))}
                </div>
              )}
              {propertyDiscovery === 'loading' && (
                <p className="view-custom-property-status" role="status">
                  {t('正在从当前匹配笔记发现属性…', 'Discovering properties from matching notes…')}
                </p>
              )}
              {propertyDiscovery === 'failed' && (
                <p className="view-custom-property-status" role="status">
                  {t(
                    '暂时无法发现属性键，仍可手动输入。',
                    'Could not discover keys right now; manual entry still works.',
                  )}
                </p>
              )}
              {customPropertyError && (
                <p className="view-custom-property-error" role="alert">
                  {customPropertyError}
                </p>
              )}
            </div>
            {draft.layout === 'board' && (
              <div className="choice-field">
                {t('看板分组', 'Board grouping')}
                <Select
                  aria-label={t('看板分组', 'Board grouping')}
                  value={draft.config.board_group_by}
                  onValueChange={(value) => setConfig({ board_group_by: value as BoardGroupBy })}
                >
                  <option value="inbox_state">{t('按整理状态', 'By organization state')}</option>
                  <option value="tag">{t('按首个标签', 'By primary tag')}</option>
                </Select>
              </div>
            )}
            {draft.layout === 'calendar' && (
              <label>
                {t('日历起始月', 'Calendar month')}
                <input
                  type="month"
                  value={draft.config.calendar_month ?? ''}
                  onChange={(event) => setConfig({ calendar_month: event.target.value || null })}
                />
              </label>
            )}
            {(draft.layout === 'calendar' || draft.layout === 'timeline') && (
              <div className="choice-field">
                {t('日期属性', 'Date property')}
                <Select
                  aria-label={t('日期属性', 'Date property')}
                  value={draft.config.date_property ?? 'updated_at'}
                  onValueChange={(value) =>
                    setConfig({ date_property: value === 'updated_at' ? null : value })
                  }
                >
                  <option value="updated_at">{t('最后编辑日期', 'Last edited date')}</option>
                  {draft.config.custom_properties.map((key) => (
                    <option value={key} key={key}>
                      {key}
                    </option>
                  ))}
                </Select>
                <small className="org-small-note">
                  {t(
                    '自定义属性缺失或无法解析的笔记会进入“未排期”，不会伪装成最后编辑日期。',
                    'Notes whose custom date is missing or invalid appear as unscheduled instead of using the last-edited date.',
                  )}
                </small>
              </div>
            )}
          </div>
          <label className="org-check">
            <input
              type="checkbox"
              checked={draft.is_pinned}
              onChange={(event) => setDraft({ ...draft, is_pinned: event.target.checked })}
            />
            {t('固定视图', 'Pin view')}
          </label>
        </div>
        <CollectionFilter
          identity={identity}
          value={draft.filter.collection_id ?? null}
          onChange={(collection_id) => setFilter({ collection_id })}
        />
        <TagChoices
          identity={identity}
          selected={draft.filter.tag_ids}
          onChange={(tag_ids) => setFilter({ tag_ids })}
        />
        <ObjectTypeChoices
          customTypes={customTypes}
          selected={draft.filter.object_types}
          onChange={(object_types) => setFilter({ object_types })}
        />
        <p className="org-small-note">
          {t(
            '所有筛选条件同时满足；多个标签按“并且”匹配。视图不会包含回收站笔记。',
            'Notes must match every filter and every selected tag. Trashed notes are excluded.',
          )}
        </p>
        <div className="button-row">
          <button className="primary-button" disabled={!draft.name.trim()}>
            <Check size={15} />
            {t('保存并运行', 'Save and run')}
          </button>
          <button className="secondary-button" type="button" onClick={onCancel}>
            {t('取消', 'Cancel')}
          </button>
          {initial && (
            <button className="text-button" type="button" onClick={onReload}>
              {t('载入当前版本', 'Reload current version')}
            </button>
          )}
        </div>
      </fieldset>
    </form>
  );
}
function CollectionFilter({
  identity,
  value,
  onChange,
}: {
  identity: OrganizationPageProps['identity'];
  value: string | null;
  onChange: (id: string | null) => void;
}) {
  const { t } = useI18n();
  const [archived, setArchived] = useState(false);
  const [query, setQuery] = useState('');
  const collections = usePagedList<CollectionDetail>(
    `${pageKey(identity, 'views')}/collection-options/${archived}/${JSON.stringify(query)}`,
    (cursor, size) =>
      call(
        {
          kind: 'collections_list',
          payload: {
            limit: size,
            cursor,
            trashed: false,
            archived,
            pinned_only: false,
            query: query || null,
          },
        },
        'collections',
        identity,
      ),
  );
  return (
    <details className="org-picker">
      <summary>
        {t('限定集合', 'Limit to a collection')}
        {value ? ` · ${t('已选择', 'Selected')}` : ` · ${t('不限', 'Any')}`}
      </summary>
      <label>
        {t('查找集合', 'Find collections')}
        <input value={query} onChange={(event) => setQuery(event.target.value)} />
      </label>
      <label className="org-check">
        <input
          type="checkbox"
          checked={archived}
          onChange={(event) => setArchived(event.target.checked)}
        />
        {t('查看已归档集合', 'Show archived collections')}
      </label>
      <div className="choice-field">
        {t('集合', 'Collection')}
        <Select
          aria-label={t('集合', 'Collection')}
          value={value ?? ''}
          onValueChange={(nextValue) => onChange(nextValue || null)}
        >
          <option value="">{t('不限定集合', 'Any collection')}</option>
          {value && !collections.page?.items.some((item) => item.id === value) && (
            <option value={value}>{t('当前所选集合', 'Currently selected collection')}</option>
          )}
          {collections.page?.items.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </Select>
      </div>
      <QueryFeedback
        state={collections}
        reload={() => void collections.refresh()}
        pager={<ListPagination list={collections} />}
        empty={t('没有匹配的集合。', 'No matching collections.')}
      />
    </details>
  );
}
function TagChoices({
  identity,
  selected,
  onChange,
}: {
  identity: OrganizationPageProps['identity'];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const tags = usePagedList<Tag>(
    `${pageKey(identity, 'views')}/tag-options/${JSON.stringify(query)}`,
    (cursor, size) =>
      call(
        { kind: 'tags_list', payload: { limit: size, cursor, query: query || null } },
        'tags',
        identity,
      ),
  );
  return (
    <details className="org-picker">
      <summary>
        {t(`标签 · 已选 ${selected.length} 个`, `Tags · ${selected.length} selected`)}
      </summary>
      <label>
        {t('查找标签', 'Find tags')}
        <input value={query} onChange={(event) => setQuery(event.target.value)} />
      </label>
      <div className="org-tag-options">
        {tags.page?.items.map((tag) => (
          <button
            type="button"
            key={tag.id}
            className={`org-tag-option ${tagToneClass(tag.id)}`}
            aria-pressed={selected.includes(tag.id)}
            disabled={selected.length >= 100 && !selected.includes(tag.id)}
            onClick={() =>
              onChange(
                selected.includes(tag.id)
                  ? selected.filter((id) => id !== tag.id)
                  : [...selected, tag.id],
              )
            }
          >
            {tag.display_name}
          </button>
        ))}
      </div>
      <QueryFeedback
        state={tags}
        reload={() => void tags.refresh()}
        pager={<ListPagination list={tags} />}
        empty={t('没有匹配的标签。', 'No matching tags.')}
      />
      {!!selected.length && (
        <button type="button" className="text-button" onClick={() => onChange([])}>
          {t('清除标签条件', 'Clear tag filters')}
        </button>
      )}
    </details>
  );
}
function ObjectTypeChoices({
  customTypes,
  selected,
  onChange,
}: {
  customTypes: readonly ObjectTypeDefinition[];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  const { t } = useI18n();
  const types = objectTypeCatalog(customTypes);
  const tokensFor = (type: ObjectTypeDefinition) => objectTypeQueryTokens(type);
  const activeCount = types.filter((type) =>
    tokensFor(type).some((token) => selected.includes(token)),
  ).length;
  return (
    <details className="org-picker">
      <summary>
        {t(`对象类型 · 已选 ${activeCount} 类`, `Object types · ${activeCount} selected`)}
      </summary>
      <p className="org-small-note">
        {t(
          '不选择表示不限类型；选择会同时匹配该类型的旧别名。',
          'No selection means any type. A selection also matches legacy aliases.',
        )}
      </p>
      <div className="org-tag-options">
        {types.map((type) => {
          const tokens = tokensFor(type);
          const active = tokens.some((token) => selected.includes(token));
          return (
            <button
              type="button"
              key={type.id}
              className="org-tag-option"
              aria-pressed={active}
              onClick={() =>
                onChange(
                  active
                    ? selected.filter((token) => !tokens.includes(token))
                    : [...new Set([...selected, ...tokens])],
                )
              }
            >
              {t(type.pluralZh, type.pluralEn)}
            </button>
          );
        })}
      </div>
      {!!selected.length && (
        <button type="button" className="text-button" onClick={() => onChange([])}>
          {t('清除类型条件', 'Clear type filters')}
        </button>
      )}
    </details>
  );
}
function ViewResults({
  identity,
  view,
  refresh,
  onOpenNote,
  notice,
}: {
  identity: OrganizationPageProps['identity'];
  view: SavedView;
  refresh: number;
  onOpenNote: (id: string) => void;
  notice: ReactNode;
}) {
  const { t } = useI18n();
  const [selectedId, setSelectedId] = usePageNavigationState<string | null>(
    `views.run.${view.id}.selected`,
    null,
  );
  const editorRef = useRef<HTMLDivElement>(null);
  const previousSelection = useRef(selectedId);
  useEffect(() => {
    if (previousSelection.current === selectedId) return;
    previousSelection.current = selectedId;
    if (!selectedId) return;
    const frame = requestAnimationFrame(() =>
      editorRef.current?.scrollIntoView({ block: 'nearest' }),
    );
    return () => cancelAnimationFrame(frame);
  }, [selectedId]);
  const paging = useViewPaging<ViewNote>(
    `${pageKey(identity, 'views')}/run/${view.id}/${view.row_version}/${refresh}`,
    `views.run.${view.id}.${view.row_version}`,
    async (cursor, size) => {
      const result = await call(
        { kind: 'views_run', payload: { view_id: view.id, limit: size, cursor } },
        'view_run',
        identity,
      );
      if (result.view.row_version !== view.row_version)
        throw new Error(
          t(
            '视图定义已有变化，请刷新视图后继续。',
            'This view has changed. Refresh its definition to continue.',
          ),
        );
      return result.notes;
    },
  );
  const results = paging.pager;
  const items = results.page?.items ?? [];
  const selected = items.find((item) => item.note.id === selectedId) ?? null;
  function openNote(id: string) {
    paging.rememberScroll();
    onOpenNote(id);
  }
  function exportCurrent() {
    downloadText(`${safeFilename(view.name)}.csv`, viewCsv(view, items));
  }
  return (
    <section className="view-results-print" aria-label={t('视图结果', 'View results')}>
      <div className="org-section-title view-result-title">
        <h3>
          {t('匹配的笔记', 'Matching notes')}
          {results.page && ` · ${items.length}${results.page.next_cursor ? '+' : ''}`}
        </h3>
        <div className="view-result-actions">
          <button
            className="secondary-button"
            disabled={!items.length}
            aria-label={t('导出 CSV', 'Export CSV')}
            onClick={exportCurrent}
          >
            <Download size={14} />
            <span className="saved-view-action-label">{t('导出 CSV', 'Export CSV')}</span>
          </button>
          <button
            className="secondary-button"
            disabled={!items.length}
            aria-label={t('打印', 'Print')}
            onClick={() => window.print()}
          >
            <Printer size={14} />
            <span className="saved-view-action-label">{t('打印', 'Print')}</span>
          </button>
          <button
            className="secondary-button"
            disabled={results.loading}
            aria-label={t('重新运行', 'Run again')}
            onClick={() => void results.refresh()}
          >
            <Play size={14} />
            <span className="saved-view-action-label">{t('重新运行', 'Run again')}</span>
          </button>
        </div>
      </div>
      <div
        className="view-results-scroll"
        ref={paging.scrollElement}
        onScroll={paging.rememberScroll}
      >
        {notice}
        <QueryFeedback
          state={results}
          reload={() => void results.refresh()}
          pager={null}
          empty={t(
            '还没有笔记符合这个视图。调整筛选条件，或稍后再来看看。',
            'No notes match this view yet. Adjust its filters or check back later.',
          )}
        />
        <ViewPrintHeader view={view} />
        {selected && (
          <div className="view-metadata-anchor" ref={editorRef}>
            <ViewMetadataEditor
              item={selected}
              identity={identity}
              onChanged={() => void results.refresh()}
              onOpenNote={openNote}
              onClose={() => setSelectedId(null)}
            />
          </div>
        )}
        <ViewLayoutView
          view={view}
          items={items}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onOpenNote={openNote}
          onChanged={() => void results.refresh()}
        />
        <details className="view-export-info">
          <summary>{t('导出与打印范围', 'Export and print scope')}</summary>
          <p className="org-small-note">
            {t(
              `导出包含当前已加载的 ${items.length} 条笔记、整理属性、所选 frontmatter 投影与正文纯文本预览摘要；缺失或截断的属性会在 property_warnings 列标明。完整正文、富格式、附件和完整历史不在 CSV 中。打印会使用当前布局。`,
              `Export contains the ${items.length} loaded notes, organization metadata, selected frontmatter projections and a plain-text preview excerpt. Missing or truncated properties are called out in property_warnings. Full body content, rich formatting, attachments and full history are not in the CSV. Print uses the current layout.`,
            )}
          </p>
        </details>
      </div>
      <div className="view-results-footer">
        <ListPagination list={results} />
      </div>
    </section>
  );
}
