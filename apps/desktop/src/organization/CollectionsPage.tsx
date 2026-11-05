import { Select } from '../ui/Select';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  Archive,
  ArrowDown,
  ArrowUp,
  BookOpen,
  FileText,
  FolderClosed,
  ChevronLeft,
  ChevronRight,
  MoreHorizontal,
  Pencil,
  Pin,
  Plus,
  RefreshCw,
  Search,
  MessageSquare,
  Quote,
  RotateCcw,
  Trash2,
  X,
} from 'lucide-react';
import type {
  CollectionDetail,
  CollectionMember,
  ObjectKind,
  UpdateCollection,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { displayDate } from '../format';
import {
  ActionNotice,
  NotePicker,
  PageLoadFailure,
  QueryFeedback,
  type OrganizationPageProps,
} from './OrganizationControls';
import { usePageActions } from './page-hooks';
import { PageActions, pageKey, type OrganizationCommand } from './page-actions';
import { reorderedMembers } from './collection-order';
import { collectionMemberTarget } from './collection-member-model';
import { MemberPicker } from './MemberPicker';
import { collectionMemberIds } from './member-candidates';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { PageFrame, PageFooter } from '../layout/PageFrame';
import { PopupMenu } from '../ui/PopupMenu';
import './organization-pages.css';

type Edit = {
  name: string;
  overview: string | null;
  overviewTitle: string;
  pinned: boolean;
  base: number;
};
export function CollectionsPage(props: OrganizationPageProps) {
  return <CollectionsScope key={pageKey(props.identity, 'collections')} {...props} />;
}
function CollectionsScope(props: OrganizationPageProps) {
  const loaded = usePageActions(props.identity, 'collections');
  return loaded.controller ? (
    <CollectionsContent {...props} controller={loaded.controller} />
  ) : (
    <PageLoadFailure error={loaded.error} retry={loaded.retry} />
  );
}
function CollectionsContent({
  identity,
  onOpenNote,
  onOpenTarget,
  onSearchCollection,
  onAskCollection,
  onCreateInCollection,
  onChanged,
  controller,
}: OrganizationPageProps & { controller: PageActions }) {
  const { t, language } = useI18n();
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const [mode, setMode] = usePageNavigationState<'active' | 'archived' | 'trash'>(
    'collections.mode',
    'active',
  );
  const [query, setQuery] = usePageNavigationState('collections.query', '');
  const [pinnedOnly, setPinnedOnly] = usePageNavigationState('collections.pinned', false);
  const [selected, setSelected] = usePageNavigationState<string | null>(
    'collections.selected',
    null,
  );
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');
  const [detail, setDetail] = useState<CollectionDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [edit, setEdit] = useState<Edit | null>(null);
  const [pickingOverview, setPickingOverview] = useState(false);
  const overviewPickerButton = useRef<HTMLButtonElement>(null);
  const [pickingMembers, setPickingMembers] = useState(false);
  const [chosen, setChosen] = useState<string[]>([]);
  const [preparing, setPreparing] = useState(false);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const key = pageKey(identity, 'collections');
  const locked = state.busy || !!state.pending || preparing;
  function closeOverviewPicker() {
    setPickingOverview(false);
    overviewPickerButton.current?.focus({ preventScroll: true });
  }
  const collections = usePagedList(
    `${key}/${mode}/${pinnedOnly}/${JSON.stringify(query)}/${state.generation}/${refresh}`,
    (cursor, size) =>
      call(
        {
          kind: 'collections_list',
          payload: {
            limit: size,
            cursor,
            trashed: mode === 'trash',
            archived: mode === 'archived',
            pinned_only: pinnedOnly,
            query: query || null,
          },
        },
        'collections',
        identity,
      ),
  );
  useEffect(() => {
    let current = true;
    setDetail(null);
    setDetailError(null);
    setLoadingDetail(!!selected);
    if (selected)
      void call(
        { kind: 'collections_get', payload: { collection_id: selected } },
        'collection',
        identity,
      )
        .then((value) => {
          if (current) setDetail(value);
        })
        .catch((error) => {
          if (current) setDetailError(message(error));
        })
        .finally(() => {
          if (current) setLoadingDetail(false);
        });
    return () => {
      current = false;
    };
  }, [selected, state.generation, refresh]);
  function select(id: string) {
    setPreparing(false);
    setSelected(id);
    setCreating(false);
    setEdit(null);
    setPickingOverview(false);
    setPickingMembers(false);
    setChosen([]);
  }
  async function act(command?: OrganizationCommand) {
    const kind = command?.kind ?? state.pending?.command.kind;
    const result = await controller.run(command);
    if (result && live.current) {
      if ('collection' in result && kind === 'collections_create') {
        setSelected(result.collection.id);
        setCreating(false);
      }
      setEdit(null);
      setPickingOverview(false);
      setPickingMembers(false);
      setChosen([]);
      if (kind === 'objects_set_archived' || kind === 'objects_set_trashed') {
        setRefresh((value) => value + 1);
      }
      onChanged?.();
    }
    return result;
  }
  function update(overrides: Partial<UpdateCollection>) {
    if (!detail) return;
    void act({
      kind: 'collections_update',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          collection_id: detail.id,
          base_row_version: detail.metadata.row_version,
          name: detail.name,
          overview_note_id: detail.overview_note_id,
          is_pinned: detail.is_pinned,
          pin_position: detail.pin_position,
          ...overrides,
        },
      },
    });
  }
  function flag(kind: 'objects_set_trashed' | 'objects_set_archived', value: boolean) {
    if (detail)
      void act({
        kind,
        payload: {
          command_id: crypto.randomUUID(),
          request: { object_id: detail.id, base_row_version: detail.metadata.row_version, value },
        },
      });
  }
  const showingDetail = creating || Boolean(selected);
  function backToCollections() {
    setSelected(null);
    setCreating(false);
    setEdit(null);
    setPickingOverview(false);
    setPickingMembers(false);
    setChosen([]);
    setNewName('');
  }
  return (
    <PageFrame
      className={`org-page ${showingDetail ? 'collection-detail-page' : 'collections-page'}`}
      nestedFooter={showingDetail && !creating}
      footer={!showingDetail ? <ListPagination list={collections} /> : undefined}
      title={t('集合', 'Collections')}
      description={
        showingDetail
          ? t(
              '管理主题成员、顺序、介绍与集合操作；返回时保留当前筛选。',
              'Manage topic members, order, overview, and collection actions; return without losing the filter.',
            )
          : t(
              '把笔记、资料与摘录放在一起，继续自己的主题研究。',
              'Bring notes, sources, and excerpts together to continue your research.',
            )
      }
      actions={
        showingDetail ? (
          <button type="button" className="text-button collection-back" onClick={backToCollections}>
            <ChevronLeft size={16} />
            {t('所有集合', 'All collections')}
          </button>
        ) : (
          <button
            className="primary-button"
            disabled={locked}
            onClick={() => {
              setCreating(true);
              setSelected(null);
              setEdit(null);
              setNewName('');
            }}
          >
            <Plus size={16} />
            {t('新建集合', 'New collection')}
          </button>
        )
      }
    >
      {showingDetail ? (
        <>
          <ActionNotice state={state} retry={() => void act()} />
          {preparing && (
            <p className="org-working" role="status">
              {t('正在读取完整成员顺序…', 'Reading the complete member order…')}
            </p>
          )}
          <div className="org-detail collection-detail-content">
            {creating ? (
              <form
                className="org-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  if (newName.trim())
                    void act({
                      kind: 'collections_create',
                      payload: {
                        command_id: crypto.randomUUID(),
                        request: {
                          name: newName.trim(),
                          overview_note_id: null,
                          is_pinned: false,
                          pin_position: null,
                        },
                      },
                    }).then((result) => {
                      if (result && live.current) {
                        setMode('active');
                        setQuery('');
                        setPinnedOnly(false);
                      }
                    });
                }}
              >
                <h2>{t('新建集合', 'New collection')}</h2>
                <label>
                  {t('集合名称', 'Collection name')}
                  <input
                    autoFocus
                    maxLength={300}
                    disabled={locked}
                    value={newName}
                    onChange={(event) => setNewName(event.target.value)}
                  />
                </label>
                <div className="button-row">
                  <button className="primary-button" disabled={locked || !newName.trim()}>
                    {t('创建集合', 'Create collection')}
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
            ) : loadingDetail ? (
              <p className="org-loading" role="status">
                {t('正在读取集合…', 'Loading collection…')}
              </p>
            ) : detailError ? (
              <div role="alert" className="org-inline-error">
                <p>{detailError}</p>
                <button className="secondary-button" onClick={() => setRefresh((n) => n + 1)}>
                  {t('重新读取', 'Reload')}
                </button>
              </div>
            ) : detail ? (
              <>
                <header className="org-detail-heading">
                  <div>
                    <h2>{detail.name}</h2>
                    <p>
                      {t(`${detail.member_count} 个成员`, `${detail.member_count} members`)}
                      {detail.updated_at && ` · ${displayDate(detail.updated_at, language)}`}
                    </p>
                  </div>
                  {!detail.metadata.trashed && (
                    <>
                      <button
                        className="icon-button"
                        disabled={locked || !!edit}
                        aria-pressed={detail.is_pinned}
                        aria-label={
                          detail.is_pinned
                            ? t('取消固定集合', 'Unpin collection')
                            : t('固定集合', 'Pin collection')
                        }
                        title={detail.is_pinned ? t('取消固定', 'Unpin') : t('固定', 'Pin')}
                        onClick={() =>
                          update({
                            is_pinned: !detail.is_pinned,
                            pin_position: detail.is_pinned ? null : 0,
                          })
                        }
                      >
                        <Pin size={17} />
                      </button>
                      <button
                        className="secondary-button"
                        disabled={locked || !!edit}
                        onClick={() =>
                          setEdit({
                            name: detail.name,
                            overview: detail.overview_note_id,
                            overviewTitle: '',
                            pinned: detail.is_pinned,
                            base: detail.metadata.row_version,
                          })
                        }
                      >
                        <Pencil size={15} />
                        {t('编辑集合', 'Edit collection')}
                      </button>
                      <PopupMenu
                        label={t('集合操作', 'Collection actions')}
                        placement="bottom-start"
                        className="icon-button"
                        disabled={locked || !!edit}
                        trigger={<MoreHorizontal size={17} />}
                        items={[
                          {
                            id: 'archive',
                            label: detail.metadata.archived
                              ? t('取消归档', 'Unarchive')
                              : t('归档集合', 'Archive collection'),
                            icon: <Archive size={15} />,
                            detail: t('不会删除成员内容', 'Keeps all members intact'),
                            onSelect: () => flag('objects_set_archived', !detail.metadata.archived),
                          },
                          {
                            id: 'trash',
                            label: t('移入回收站', 'Move to trash'),
                            icon: <Trash2 size={15} />,
                            detail: t('可从回收站恢复', 'Can be restored from trash'),
                            separatorBefore: true,
                            onSelect: () => flag('objects_set_trashed', true),
                          },
                        ]}
                      />
                    </>
                  )}
                </header>
                {detail.metadata.trashed ? (
                  <div className="org-notice">
                    <Trash2 size={18} />
                    <div>
                      <p>
                        {t(
                          '集合位于回收站。成员内容仍在知识库中。',
                          'This collection is in the trash. Its members remain in your library.',
                        )}
                      </p>
                      <button
                        className="secondary-button"
                        disabled={locked}
                        onClick={() => flag('objects_set_trashed', false)}
                      >
                        <RotateCcw size={15} />
                        {t('恢复集合', 'Restore collection')}
                      </button>
                    </div>
                  </div>
                ) : null}
                {edit ? (
                  <form
                    className="org-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      update({
                        base_row_version: edit.base,
                        name: edit.name.trim(),
                        overview_note_id: edit.overview,
                        is_pinned: edit.pinned,
                        pin_position: edit.pinned ? (detail.pin_position ?? 0) : null,
                      });
                    }}
                  >
                    <fieldset disabled={locked}>
                      <label>
                        {t('集合名称', 'Collection name')}
                        <input
                          maxLength={300}
                          value={edit.name}
                          onChange={(event) => setEdit({ ...edit, name: event.target.value })}
                        />
                      </label>
                      <label className="org-check">
                        <input
                          type="checkbox"
                          checked={edit.pinned}
                          onChange={(event) => setEdit({ ...edit, pinned: event.target.checked })}
                        />
                        {t('固定集合', 'Pin collection')}
                      </label>
                      <div className="org-section-title">
                        <h3>{t('介绍笔记', 'Overview note')}</h3>
                        <button
                          ref={overviewPickerButton}
                          type="button"
                          className="text-button"
                          aria-expanded={pickingOverview}
                          onClick={() =>
                            pickingOverview ? closeOverviewPicker() : setPickingOverview(true)
                          }
                        >
                          {t('选择笔记', 'Choose note')}
                        </button>
                        {edit.overview && (
                          <button
                            type="button"
                            className="icon-button"
                            aria-label={t('移除介绍笔记', 'Remove overview note')}
                            onClick={() => setEdit({ ...edit, overview: null, overviewTitle: '' })}
                          >
                            <X size={15} />
                          </button>
                        )}
                      </div>
                      {edit.overview && (
                        <p className="muted">
                          {edit.overviewTitle || t('已选介绍笔记', 'Overview note selected')}
                        </p>
                      )}
                      {pickingOverview && (
                        <NotePicker
                          identity={identity}
                          selected={edit.overview ? [edit.overview] : []}
                          onChange={(ids, note) => {
                            setEdit({
                              ...edit,
                              overview: ids[0] ?? null,
                              overviewTitle: note?.title ?? '',
                            });
                            closeOverviewPicker();
                          }}
                          onClose={closeOverviewPicker}
                        />
                      )}
                      <div className="button-row">
                        <button className="primary-button" disabled={!edit.name.trim()}>
                          {t('保存集合', 'Save collection')}
                        </button>
                        <button
                          className="secondary-button"
                          type="button"
                          onClick={() => {
                            setEdit(null);
                            setPickingOverview(false);
                          }}
                        >
                          {t('取消编辑', 'Cancel editing')}
                        </button>
                        <button
                          className="text-button"
                          type="button"
                          onClick={() => {
                            setEdit(null);
                            setRefresh((n) => n + 1);
                          }}
                        >
                          {t('载入当前版本', 'Reload current version')}
                        </button>
                      </div>
                    </fieldset>
                  </form>
                ) : detail.overview_note_id ? (
                  <OverviewNote
                    identity={identity}
                    id={detail.overview_note_id}
                    onOpen={onOpenNote}
                  />
                ) : (
                  <p className="muted">
                    {t(
                      '可以选择一篇笔记，作为这个主题的介绍。',
                      'Choose a note to introduce this topic.',
                    )}
                  </p>
                )}
                {!detail.metadata.trashed &&
                  (onSearchCollection || onAskCollection || onCreateInCollection) && (
                    <div className="org-topic-actions">
                      <div className="button-row">
                        {onCreateInCollection && (
                          <button
                            className="primary-button"
                            disabled={locked || !!edit}
                            onClick={() =>
                              onCreateInCollection(detail.id, detail.metadata.row_version)
                            }
                          >
                            <Plus size={16} />
                            {t('在此记录', 'Write in this topic')}
                          </button>
                        )}
                        {onSearchCollection && (
                          <button
                            className="secondary-button"
                            onClick={() => onSearchCollection(detail.id)}
                          >
                            <Search size={16} />
                            {t('在主题内搜索', 'Search this topic')}
                          </button>
                        )}
                        {onAskCollection && (
                          <button
                            className="secondary-button"
                            onClick={() => onAskCollection(detail.id)}
                          >
                            <MessageSquare size={16} />
                            {t('向主题提问', 'Ask this topic')}
                          </button>
                        )}
                      </div>
                      <p className="org-small-note">
                        {t(
                          '检索与问答仅使用此集合中已启用的笔记、资料和摘录；单独添加摘录不会扩大到它的整篇来源。',
                          'Search and answers use enabled notes, sources, and excerpts in this collection. Adding an excerpt alone does not include its whole source.',
                        )}
                      </p>
                    </div>
                  )}
                <CollectionMembers
                  key={detail.id}
                  identity={identity}
                  collection={detail}
                  generation={`${state.generation}/${refresh}`}
                  disabled={locked || !!edit || detail.metadata.trashed}
                  picking={pickingMembers}
                  setPicking={setPickingMembers}
                  chosen={chosen}
                  setChosen={setChosen}
                  act={act}
                  onOpenNote={onOpenNote}
                  onOpenTarget={onOpenTarget}
                  preparing={setPreparing}
                  live={live}
                />
              </>
            ) : (
              <div className="org-empty">
                <FolderClosed size={32} strokeWidth={1.5} />
                <h2>{t('把相关内容连成一个主题', 'Bring related content into a topic')}</h2>
                <p>
                  {t('选择一个集合，或创建新的集合。', 'Choose a collection or create a new one.')}
                </p>
              </div>
            )}{' '}
          </div>
        </>
      ) : (
        <>
          <ActionNotice state={state} retry={() => void act()} />
          {preparing && (
            <p className="org-working" role="status">
              {t('正在读取完整成员顺序…', 'Reading the complete member order…')}
            </p>
          )}
          <div className="collection-overview">
            <div className="collection-filter-bar">
              <label className="collection-filter-field collection-search-field">
                <span>{t('查找集合', 'Find collections')}</span>
                <span className="collection-search-input">
                  <Search size={15} aria-hidden="true" />
                  <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder={t('输入集合名称…', 'Collection name…')}
                    aria-label={t('查找集合', 'Find collections')}
                  />
                  {query && (
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t('清除集合搜索', 'Clear collection search')}
                      onClick={() => setQuery('')}
                    >
                      <X size={13} />
                    </button>
                  )}
                </span>
              </label>
              <div className="choice-field collection-filter-field collection-mode-field">
                <span>{t('显示', 'Show')}</span>
                <Select
                  aria-label={t('显示', 'Show')}
                  value={mode}
                  onValueChange={(nextValue) => {
                    setMode(nextValue as typeof mode);
                    setSelected(null);
                    setEdit(null);
                  }}
                >
                  <option value="active">{t('当前集合', 'Active collections')}</option>
                  <option value="archived">{t('已归档', 'Archived')}</option>
                  <option value="trash">{t('回收站', 'Trash')}</option>
                </Select>
              </div>
              <label className="org-check collection-pinned-check">
                <input
                  type="checkbox"
                  checked={pinnedOnly}
                  onChange={(event) => setPinnedOnly(event.target.checked)}
                />
                {t('仅显示固定', 'Pinned only')}
              </label>
              <button
                className="secondary-button collection-refresh"
                disabled={collections.loading}
                onClick={() => setRefresh((n) => n + 1)}
              >
                <RefreshCw size={15} className={collections.loading ? 'spin' : undefined} />
                {t('刷新', 'Refresh')}
              </button>
            </div>
            <div className="collection-overview-heading">
              <div>
                <h2>{t('全部集合', 'All collections')}</h2>
                <p>
                  {t(
                    '每个集合是一块主题工作台，成员顺序与介绍笔记都单独保存。',
                    'Each collection is a topic workspace with its own member order and overview.',
                  )}
                </p>
              </div>
              <span className="knowledge-count">{collections.page?.items.length ?? 0}</span>
            </div>
            <div className="collection-grid">
              {collections.page?.items.map((collection) => (
                <button
                  key={collection.id}
                  type="button"
                  className="collection-card"
                  onClick={() => select(collection.id)}
                >
                  <span className="collection-card-icon" aria-hidden="true">
                    <FolderClosed size={20} />
                  </span>
                  <span className="collection-card-copy">
                    <span className="collection-card-title-row">
                      <strong>{collection.name}</strong>
                      {collection.is_pinned && <Pin size={14} aria-label={t('已固定', 'Pinned')} />}
                    </span>
                    <small>
                      {t(`${collection.member_count} 个成员`, `${collection.member_count} members`)}
                      {collection.updated_at
                        ? ` · ${displayDate(collection.updated_at, language)}`
                        : ''}
                    </small>
                  </span>
                  <ChevronRight className="collection-card-arrow" size={17} aria-hidden="true" />
                </button>
              ))}
            </div>
            <QueryFeedback
              state={collections}
              reload={collections.refresh}
              pager={null}
              empty={t('没有符合条件的集合。', 'No collections match these filters.')}
            />
          </div>
        </>
      )}
    </PageFrame>
  );
}

function OverviewNote({
  identity,
  id,
  onOpen,
}: {
  identity: OrganizationPageProps['identity'];
  id: string;
  onOpen: (id: string) => void;
}) {
  const { t } = useI18n();
  const [result, setResult] = useState<{ id: string; title: string; error: boolean } | null>(null);
  useEffect(() => {
    let live = true;
    void call({ kind: 'notes_get', payload: { note_id: id } }, 'note', identity)
      .then((note) => {
        if (live) setResult({ id, title: note.title, error: false });
      })
      .catch((error) => {
        if (live) setResult({ id, title: message(error), error: true });
      });
    return () => {
      live = false;
    };
  }, [identity, id]);
  const value = result?.id === id ? result : null;
  return (
    <button className="org-overview" disabled={!value || value.error} onClick={() => onOpen(id)}>
      <BookOpen size={18} />
      <span>{value?.title ?? t('正在读取介绍笔记…', 'Loading overview note…')}</span>
    </button>
  );
}
function CollectionMembers({
  identity,
  collection,
  generation,
  disabled,
  picking,
  setPicking,
  chosen,
  setChosen,
  act,
  onOpenNote,
  onOpenTarget,
  preparing,
  live,
}: {
  identity: OrganizationPageProps['identity'];
  collection: CollectionDetail;
  generation: string;
  disabled: boolean;
  picking: boolean;
  setPicking: (value: boolean) => void;
  chosen: string[];
  setChosen: (ids: string[]) => void;
  act: (command?: OrganizationCommand) => Promise<unknown>;
  onOpenNote: (id: string) => void;
  onOpenTarget: OrganizationPageProps['onOpenTarget'];
  preparing: (value: boolean) => void;
  live: { current: boolean };
}) {
  const { t } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<ObjectKind | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const memberPickerButton = useRef<HTMLButtonElement>(null);
  function closeMemberPicker() {
    setPicking(false);
    memberPickerButton.current?.focus({ preventScroll: true });
  }
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      if (live.current) preparing(false);
    };
  }, []);
  const read = (cursor: string | null, size: number) =>
    call(
      {
        kind: 'collections_members',
        payload: {
          collection_id: collection.id,
          limit: size,
          cursor,
          include_trashed: true,
          kind,
        },
      },
      'collection_members',
      identity,
    );
  // Reordering permutes every member, so it still walks whole 100-row pages.
  const readAll = (cursor: string | null) => read(cursor, 100);
  const members = usePagedList(
    `${pageKey(identity, 'collections')}/${collection.id}/${collection.metadata.row_version}/${generation}/${kind ?? 'all'}`,
    read,
  );
  // The picker marks content that already belongs to the collection, so it has to
  // know the whole collection, not only the member page on screen: a member on
  // page 2 used to look addable, and the core drops ids that are already members,
  // so that "add" reported success and changed nothing.
  const [inCollection, setInCollection] = useState<ReadonlySet<string>>(() => new Set<string>());
  const [memberSetError, setMemberSetError] = useState<string | null>(null);
  const [loadingMemberSet, setLoadingMemberSet] = useState(false);
  const [memberSetAttempt, setMemberSetAttempt] = useState(0);
  useEffect(() => {
    if (!picking) {
      setInCollection(new Set<string>());
      setMemberSetError(null);
      setLoadingMemberSet(false);
      return;
    }
    let current = true;
    setLoadingMemberSet(true);
    setMemberSetError(null);
    void collectionMemberIds(
      (cursor) =>
        call(
          {
            kind: 'collections_members',
            payload: {
              collection_id: collection.id,
              limit: 100,
              cursor,
              include_trashed: true,
              kind: null,
            },
          },
          'collection_members',
          identity,
        ),
      collection.metadata.row_version,
    )
      .then((ids) => {
        if (current) setInCollection(ids);
      })
      .catch((failure) => {
        if (current) {
          setInCollection(new Set<string>());
          setMemberSetError(message(failure));
        }
      })
      .finally(() => {
        if (current) setLoadingMemberSet(false);
      });
    return () => {
      current = false;
    };
  }, [
    picking,
    collection.id,
    collection.metadata.row_version,
    identity.local_instance_id,
    identity.workspace_id,
    memberSetAttempt,
  ]);
  function change(
    kind: 'collections_add_members' | 'collections_remove_members',
    member_ids: string[],
  ) {
    void act({
      kind,
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          collection_id: collection.id,
          base_row_version: collection.metadata.row_version,
          member_ids,
        },
      },
    });
  }
  async function move(id: string, direction: -1 | 1) {
    preparing(true);
    setError(null);
    try {
      const ids = await reorderedMembers(readAll, collection.metadata.row_version, id, direction);
      if (live.current && active.current)
        await act({
          kind: 'collections_reorder',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              collection_id: collection.id,
              base_row_version: collection.metadata.row_version,
              member_ids: ids,
            },
          },
        });
    } catch (error) {
      if (live.current && active.current) setError(message(error));
    } finally {
      if (live.current && active.current) preparing(false);
    }
  }
  async function open(member: CollectionMember) {
    const target = collectionMemberTarget(member);
    if (!target || opening) return;
    setOpening(member.object.id);
    setError(null);
    try {
      if (member.object.kind === 'note') onOpenNote(member.object.id);
      else if (onOpenTarget) await onOpenTarget(target);
    } catch (error) {
      if (active.current) setError(message(error));
    } finally {
      if (active.current) setOpening(null);
    }
  }
  return (
    <>
      <div className="org-section-title">
        <h3>{t('主题内容', 'Topic contents')}</h3>
        <button
          ref={memberPickerButton}
          className="secondary-button"
          disabled={disabled}
          aria-expanded={picking}
          onClick={() => {
            if (picking) closeMemberPicker();
            else {
              setChosen([]);
              setPicking(true);
            }
          }}
        >
          <Plus size={15} />
          {t('添加已有内容', 'Add existing content')}
        </button>
      </div>
      {picking && (
        <>
          <MemberPicker
            identity={identity}
            selected={chosen}
            inCollection={inCollection}
            disabled={disabled}
            onChange={setChosen}
            onClose={closeMemberPicker}
          />
          {loadingMemberSet && (
            <p className="org-working" role="status">
              {t('正在核对集合成员…', 'Checking the collection members…')}
            </p>
          )}
          {memberSetError && (
            <div className="org-inline-error" role="alert">
              <p>
                {t(
                  '读取集合成员失败，暂时无法确认哪些内容已在集合内。',
                  'Could not read the collection members, so items already in the collection cannot be marked.',
                )}
              </p>
              <p>{memberSetError}</p>
              <button
                type="button"
                className="text-button"
                disabled={disabled}
                onClick={() => setMemberSetAttempt((value) => value + 1)}
              >
                {t('重新读取', 'Reload')}
              </button>
            </div>
          )}
          <div className="button-row">
            <button
              className="primary-button"
              disabled={disabled || !chosen.length || loadingMemberSet || !!memberSetError}
              onClick={() => change('collections_add_members', chosen)}
            >
              {t(`添加 ${chosen.length} 项内容`, `Add ${chosen.length} items`)}
            </button>
          </div>
        </>
      )}
      {error && (
        <p className="org-inline-error" role="alert">
          {error}
        </p>
      )}
      <div
        className="org-kind-tabs"
        role="group"
        aria-label={t('按成员类型显示', 'Filter member type')}
      >
        {([null, 'note', 'source', 'annotation'] as const).map((value) => (
          <button
            type="button"
            key={value ?? 'all'}
            aria-pressed={kind === value}
            onClick={() => setKind(value)}
          >
            {value === null
              ? t('全部', 'All')
              : value === 'note'
                ? t('笔记', 'Notes')
                : value === 'source'
                  ? t('资料', 'Sources')
                  : t('摘录', 'Excerpts')}
          </button>
        ))}
      </div>
      {kind !== null && (
        <p className="org-small-note">
          {t(
            '切换到“全部”可调整完整成员顺序。',
            'Switch to All to reorder the complete collection.',
          )}
        </p>
      )}
      {members.page?.items.map((member, index) => (
        <div className="org-member" key={member.object.id}>
          {member.object.kind === 'source' ? (
            <BookOpen size={16} />
          ) : member.object.kind === 'annotation' ? (
            <Quote size={16} />
          ) : (
            <FileText size={16} />
          )}
          <button
            className="org-member-open"
            disabled={
              !!opening ||
              !collectionMemberTarget(member) ||
              (member.object.kind !== 'note' && !onOpenTarget)
            }
            onClick={() => void open(member)}
          >
            <strong>
              {member.note
                ? member.note.title || t('未命名笔记', 'Untitled note')
                : member.source
                  ? member.source.display_title || t('未命名资料', 'Untitled source')
                  : member.annotation
                    ? member.annotation.quote_preview ||
                      member.annotation.comment_preview ||
                      t('摘录', 'Excerpt')
                    : t('内容不可用', 'Content unavailable')}
            </strong>
            <small>
              {member.object.trashed
                ? t('已在回收站', 'In trash')
                : member.annotation?.source_trashed
                  ? t('摘录来源在回收站', 'Excerpt source is in trash')
                  : member.note?.preview.slice(0, 90) ||
                    member.annotation?.source_title ||
                    member.source?.format.toUpperCase() ||
                    t('没有可用摘要', 'No summary available')}
            </small>
            {!member.object.trashed &&
              (!member.object.search_enabled ||
                member.annotation?.source_search_enabled === false) && (
                <small className="org-member-state">
                  {t('未参与检索，仍可阅读', 'Excluded from search; available to read')}
                </small>
              )}
            {member.object.archived && (
              <small className="org-member-state">{t('已归档', 'Archived')}</small>
            )}
            {member.source && !member.source.text_ready && (
              <small className="org-member-state">{t('正文尚未就绪', 'Text not ready')}</small>
            )}
          </button>
          <button
            className="icon-button"
            disabled={disabled || kind !== null || index === 0}
            title={t('上移', 'Move up')}
            aria-label={t('上移成员', 'Move member up')}
            onClick={() => void move(member.object.id, -1)}
          >
            <ArrowUp size={15} />
          </button>
          <button
            className="icon-button"
            disabled={
              disabled ||
              kind !== null ||
              (index === (members.page?.items.length ?? 0) - 1 && !members.page?.next_cursor)
            }
            title={t('下移', 'Move down')}
            aria-label={t('下移成员', 'Move member down')}
            onClick={() => void move(member.object.id, 1)}
          >
            <ArrowDown size={15} />
          </button>
          <button
            className="icon-button"
            disabled={disabled}
            title={t('移出集合', 'Remove from collection')}
            aria-label={t('移出集合', 'Remove from collection')}
            onClick={() => change('collections_remove_members', [member.object.id])}
          >
            <X size={15} />
          </button>
        </div>
      ))}
      <QueryFeedback
        state={members}
        reload={members.refresh}
        pager={
          <PageFooter>
            <ListPagination list={members} />
          </PageFooter>
        }
        empty={t(
          '此处还没有内容，可以添加相关笔记、资料或摘录。',
          'Add related notes, sources, or excerpts to this collection.',
        )}
      />
    </>
  );
}
