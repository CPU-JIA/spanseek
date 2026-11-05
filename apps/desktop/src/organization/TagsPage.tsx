import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import {
  ArrowLeft,
  Archive,
  BookOpen,
  FileText,
  FolderClosed,
  Hash,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import type {
  CollectionDetail,
  MetadataCommit,
  TagBrowseItem,
  TagDetails,
  TaggedObject,
  TagManagementCommit,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { PageState } from '../layout/PageState';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import type { CursorPage } from '../layout/pagination.ts';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { useInferenceAction } from '../inference/useInferenceAction';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import { Combobox } from '../ui/Combobox';
import { PopupMenu } from '../ui/PopupMenu';
import { SectionSwitch } from '../ui/SectionSwitch';
import { tagToneClass } from '../ui/tag-color';
import { notify } from '../ui/ToastHost';
import { tagErrorMessage } from './tag-errors';
import { MemberPicker } from './MemberPicker';
import './tags-page.css';

type Position = { scope: string; page: number; size: number; scroll: number };
function useRememberedList<T>(
  key: string,
  scope: string,
  read: (cursor: string | null, size: number) => Promise<CursorPage<T>>,
) {
  const [saved, setSaved] = usePageNavigationState<Position>(key, {
    scope: '',
    page: 1,
    size: 10,
    scroll: 0,
  });
  const viewport = useRef<HTMLUListElement>(null);
  const list = usePagedList(scope, read, saved.size, true);
  const current = useRef(list);
  current.current = list;
  const restored = useRef('');
  const restoring = useRef(false);
  const lifetime = useRef({ live: true, scope });
  useEffect(() => {
    const token = { live: true, scope };
    lifetime.current = token;
    restored.current = '';
    restoring.current = false;
    return () => {
      token.live = false;
    };
  }, [scope]);
  function remember() {
    if (
      !lifetime.current.live ||
      lifetime.current.scope !== scope ||
      restoring.current ||
      current.current.loading
    )
      return;
    setSaved({
      scope,
      page: current.current.index,
      size: current.current.size,
      scroll: viewport.current?.scrollTop ?? 0,
    });
  }
  useEffect(() => {
    if (restored.current === scope || list.loading || !list.page || list.error) return;
    restored.current = scope;
    restoring.current = true;
    const token = lifetime.current;
    const restore = async () => {
      if (saved.scope === scope && saved.page > 1) await current.current.goTo(saved.page);
      if (!token.live || token.scope !== scope) return;
      requestAnimationFrame(() => {
        if (!token.live || token.scope !== scope) return;
        if (viewport.current) viewport.current.scrollTop = saved.scope === scope ? saved.scroll : 0;
        restoring.current = false;
      });
    };
    void restore();
    // The effect may re-run as goTo loads; only unmount/scope changes invalidate
    // its completion, so this effect does not cancel on a page snapshot change.
    return undefined;
  }, [scope, list.loading, Boolean(list.page), list.error]);
  useEffect(() => {
    if (restored.current === scope && !restoring.current && !list.loading && list.page) remember();
  }, [scope, list.index, list.size, list.loading]);
  return { list, viewport, remember };
}

type Props = {
  identity: WorkspaceIdentity;
  refreshVersion?: number;
  onOpenObject: (object: TaggedObject) => void;
  onOpenView: (id: string) => void;
  onOpenLibrary: () => void;
  onOpenTrash: () => void;
  onChanged: (result: TagManagementCommit) => void;
  onContentChanged: () => void;
};
export function TagsPage(props: Props) {
  const {
    identity,
    refreshVersion = 0,
    onOpenObject,
    onOpenView,
    onOpenLibrary,
    onOpenTrash,
    onChanged,
    onContentChanged,
  } = props;
  const { t } = useI18n();
  const [selected, setSelected] = usePageNavigationState<string | null>('tags.selected', null);
  const [collection, setCollection] = usePageNavigationState<{ id: string; name: string } | null>(
    'tags.collection',
    null,
  );
  const [query, setQuery] = usePageNavigationState('tags.query', '');
  const [refresh, setRefresh] = useState(0);
  const [editor, setEditor] = useState<{ tag: TagDetails | null } | null>(null);
  const [deleting, setDeleting] = useState<TagDetails | null>(null);
  const [blocked, setBlocked] = useState<TagDetails | null>(null);
  const [assigning, setAssigning] = useState<TagDetails | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const preparing = useRef(false);
  const [error, setError] = useState('');
  const [reading, setReading] = useState(false);
  const request = useRef(0);
  const mounted = useRef(true);
  const actions = useInferenceAction(identity, 'tags/manage', (failure) =>
    tagErrorMessage(failure, t),
  );
  const locked = actions.locked || reading;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      ++request.current;
    };
  }, []);
  async function inspect(id: string, action: 'edit' | 'delete' | 'assign') {
    const sequence = ++request.current;
    setReading(true);
    setError('');
    actions.clearError();
    try {
      const tag = await call(
        { kind: 'tags_get', payload: { tag_id: id } },
        'tag_details',
        identity,
      );
      if (sequence !== request.current) return;
      if (action === 'assign') {
        setAssigning(tag);
        setChosen([]);
      } else if (action === 'edit') setEditor({ tag });
      else if (tag.saved_views.length) setBlocked(tag);
      else setDeleting(tag);
    } catch (failure) {
      if (sequence === request.current) setError(tagErrorMessage(failure, t));
    } finally {
      if (sequence === request.current) setReading(false);
    }
  }
  function committed(result: TagManagementCommit) {
    if (!mounted.current) return;
    setEditor(null);
    setDeleting(null);
    setError('');
    notify({
      id: result.receipt.command_id,
      tone: 'success',
      text: result.receipt.replayed
        ? t(
            '操作结果已确认，列表显示当前状态。',
            'The operation was confirmed. The list shows the current state.',
          )
        : result.deleted
          ? t('标签已删除，相关内容保留。', 'Tag deleted. Its content is kept.')
          : t('标签已保存。', 'Tag saved.'),
    });
    if (result.deleted && selected === result.tag.tag.id) setSelected(null);
    onChanged(result);
    setRefresh((value) => value + 1);
  }
  async function save(name: string, description: string) {
    const tag = editor?.tag;
    const command_id = crypto.randomUUID();
    const result = await actions.execute(
      tag
        ? {
            kind: 'tags_update',
            payload: {
              command_id,
              request: { id: tag.tag.id, base_row_version: tag.row_version, name, description },
            },
          }
        : { kind: 'tags_create', payload: { command_id, request: { name, description } } },
      'tag_management_commit',
    );
    if (result && mounted.current) {
      if (!tag) {
        setSelected(null);
        setCollection(null);
        setQuery(result.tag.tag.display_name);
      }
      committed(result);
    }
  }
  async function remove() {
    if (!deleting) return;
    const tag = deleting;
    setDeleting(null);
    const result = await actions.execute(
      {
        kind: 'tags_delete',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            id: tag.tag.id,
            base_row_version: tag.row_version,
            impact_token: tag.impact_token,
          },
        },
      },
      'tag_management_commit',
    );
    if (result) committed(result);
  }
  function contentCommitted(result: MetadataCommit) {
    if (!mounted.current) return;
    setAssigning(null);
    setChosen([]);
    setError('');
    notify({
      id: result.receipt.command_id,
      tone: 'success',
      text: t('标签关联已更新。', 'Tag links updated.'),
    });
    onContentChanged();
    setRefresh((value) => value + 1);
  }
  async function changeLinks(ids: string[], tagId: string, remove: boolean) {
    if (preparing.current || actions.locked || !ids.length) return;
    preparing.current = true;
    setReading(true);
    setError('');
    try {
      const objects = await Promise.all(
        ids.map((object_id) =>
          call(
            { kind: 'objects_metadata_get', payload: { object_id } },
            'object_metadata',
            identity,
          ),
        ),
      );
      if (!mounted.current) return;
      const targets = objects.map((object) => ({
        object_id: object.id,
        base_row_version: object.row_version,
      }));
      const command_id = crypto.randomUUID();
      const result = await actions.execute(
        remove
          ? { kind: 'tags_remove', payload: { command_id, request: { targets, tag_id: tagId } } }
          : {
              kind: 'tags_assign',
              payload: { command_id, request: { targets, tag: { kind: 'id', id: tagId } } },
            },
        'metadata_commit',
      );
      if (result) contentCommitted(result);
    } catch (failure) {
      if (mounted.current) setError(tagErrorMessage(failure, t));
    } finally {
      preparing.current = false;
      if (mounted.current) setReading(false);
    }
  }
  async function retry() {
    if (actions.pending?.expected === 'metadata_commit') {
      const result = await actions.retry<'metadata_commit'>();
      if (result) contentCommitted(result);
    } else {
      const result = await actions.retry<'tag_management_commit'>();
      if (result) committed(result);
    }
  }
  const pendingFeedback = actions.pending && (
    <div className="tag-feedback" role="status">
      {t('有一项标签操作等待确认。', 'A tag change needs confirmation.')}
      <button
        type="button"
        className="secondary-button"
        disabled={actions.busy}
        onClick={() => void retry()}
      >
        {t('重新确认结果', 'Confirm result')}
      </button>
    </div>
  );
  const feedback = (
    <>
      {(error || actions.error) && (
        <p className="tag-feedback is-error" role="alert">
          {error || actions.error}
        </p>
      )}
      {pendingFeedback}
    </>
  );
  const scopePicker = (
    <TagCollectionPicker identity={identity} value={collection} onChange={setCollection} />
  );
  return (
    <>
      {selected ? (
        <TaggedContent
          key={selected}
          identity={identity}
          tagId={selected}
          collection={collection}
          refresh={refresh + refreshVersion}
          onBack={() => setSelected(null)}
          onOpen={onOpenObject}
          onAdd={() => void inspect(selected, 'assign')}
          onTrash={onOpenTrash}
          onRemove={(object) => void changeLinks([object.id], selected, true)}
          onEdit={() => void inspect(selected, 'edit')}
          onDelete={() => void inspect(selected, 'delete')}
          locked={locked}
          feedback={feedback}
          scopePicker={scopePicker}
        />
      ) : (
        <TagDirectory
          identity={identity}
          collectionId={collection?.id ?? null}
          query={query}
          onQuery={setQuery}
          refresh={refresh + refreshVersion}
          onOpen={setSelected}
          onCreate={() => {
            setError('');
            actions.clearError();
            setEditor({ tag: null });
          }}
          onEdit={(id) => void inspect(id, 'edit')}
          onDelete={(id) => void inspect(id, 'delete')}
          locked={locked}
          feedback={feedback}
          scopePicker={scopePicker}
        />
      )}
      {editor && (
        <TagEditor
          tag={editor.tag}
          busy={actions.busy}
          pending={Boolean(actions.pending)}
          pendingFeedback={pendingFeedback}
          error={actions.error}
          onSave={save}
          onClose={() => {
            setEditor(null);
            actions.clearError();
          }}
          onReload={editor.tag ? () => void inspect(editor.tag!.tag.id, 'edit') : undefined}
        />
      )}
      {deleting && (
        <ActionConfirmDialog
          title={t(
            `删除标签“${deleting.tag.display_name}”？`,
            `Delete tag “${deleting.tag.display_name}”?`,
          )}
          description={t(
            `将移除 ${deleting.object_count} 个对象的标签关联，其中 ${deleting.trashed_object_count} 个在回收站中。相关正文、资料和摘录保留，待处理的旧标签建议将跳过。`,
            `Remove this tag from ${deleting.object_count} objects, including ${deleting.trashed_object_count} in trash. Notes, sources and excerpts are kept. Pending suggestions for this tag will be skipped.`,
          )}
          confirmLabel={t('删除标签', 'Delete tag')}
          tone="danger"
          onCancel={() => setDeleting(null)}
          onConfirm={() => void remove()}
        />
      )}
      {assigning && (
        <TagAssignmentDialog
          identity={identity}
          tag={assigning}
          selected={chosen}
          busy={actions.busy || reading}
          pending={Boolean(actions.pending)}
          error={error || actions.error}
          pendingFeedback={pendingFeedback}
          onChange={(ids) => {
            if (ids.length > 100) {
              setError(t('每次最多选择 100 项内容。', 'Select at most 100 items at a time.'));
              return;
            }
            setError('');
            setChosen(ids);
          }}
          onClose={() => setAssigning(null)}
          onCreate={() => {
            setAssigning(null);
            onOpenLibrary();
          }}
          onApply={() => void changeLinks(chosen, assigning.tag.id, false)}
        />
      )}
      {blocked && (
        <TagDialog
          title={t('先更新使用此标签的视图', 'Update views using this tag')}
          onClose={() => setBlocked(null)}
          footer={
            <button className="secondary-button" onClick={() => setBlocked(null)}>
              {t('返回', 'Back')}
            </button>
          }
        >
          <p>
            {t(
              '删除标签不能自动扩大保存视图的范围。请先打开以下视图，修改标签筛选后再删除。',
              'Deleting a tag must not silently broaden saved views. Open these views and update their tag filters first.',
            )}
          </p>
          <ul className="tag-view-usage">
            {blocked.saved_views.map((view) => (
              <li key={view.id}>
                <button
                  className="text-button"
                  onClick={() => {
                    setBlocked(null);
                    onOpenView(view.id);
                  }}
                >
                  {view.name}
                </button>
              </li>
            ))}
          </ul>
        </TagDialog>
      )}
    </>
  );
}

function TagDirectory({
  identity,
  collectionId,
  query,
  onQuery,
  refresh,
  onOpen,
  onCreate,
  onEdit,
  onDelete,
  locked,
  feedback,
  scopePicker,
}: {
  identity: WorkspaceIdentity;
  collectionId: string | null;
  query: string;
  onQuery: (value: string) => void;
  refresh: number;
  onOpen: (id: string) => void;
  onCreate: () => void;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
  locked: boolean;
  feedback: ReactNode;
  scopePicker: ReactNode;
}) {
  const { t, language } = useI18n();
  const [settled, setSettled] = useState(query);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(query), 150);
    return () => clearTimeout(timer);
  }, [query]);
  const scope = JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    settled.trim(),
    collectionId,
  ]);
  const { list, viewport, remember } = useRememberedList<TagBrowseItem>(
    'tags.directory.position',
    scope,
    async (cursor, limit) => {
      const page = await call(
        {
          kind: 'tags_browse',
          payload: {
            page: cursor ? Number(cursor) : 1,
            limit,
            query: settled.trim() || null,
            collection_id: collectionId,
          },
        },
        'tags_browse_page',
        identity,
      );
      return {
        items: page.items,
        total: page.total,
        page_index: page.page,
        next_cursor: page.page * limit < page.total ? String(page.page + 1) : null,
      };
    },
  );
  const latest = useRef(list);
  latest.current = list;
  useEffect(() => {
    if (refresh) void latest.current.refresh();
  }, [refresh]);
  return (
    <PageFrame
      className="tags-directory-page"
      title={t('标签管理', 'Tags')}
      description={t(
        '跨分类组织内容，查看标签的使用范围。',
        'Organize content across collections and inspect tag usage.',
      )}
      actions={
        <button
          className="primary-button"
          aria-label={t('新建标签', 'New tag')}
          title={t('新建标签', 'New tag')}
          disabled={locked}
          onClick={onCreate}
        >
          <Plus size={16} />
          <span className="tag-action-label">{t('新建标签', 'New tag')}</span>
        </button>
      }
      toolbar={
        <>
          <div className="tag-search">
            <Search size={16} />
            <input
              aria-label={t('搜索标签', 'Search tags')}
              placeholder={t('搜索名称或说明', 'Search names or descriptions')}
              value={query}
              onChange={(event) => onQuery(event.target.value)}
            />
            {query && (
              <button
                className="icon-button"
                aria-label={t('清空搜索', 'Clear search')}
                onClick={() => onQuery('')}
              >
                <X size={14} />
              </button>
            )}
          </div>
          {scopePicker}
        </>
      }
      footer={<ListPagination list={list} />}
    >
      {feedback}
      <p className="tag-count-hint">
        {t(
          '使用数量包含归档内容；已回收内容在标签内单独查看。',
          'Usage includes archived content. Trashed items can be viewed inside each tag.',
        )}
      </p>
      {list.error ? (
        <PageState
          kind="error"
          title={t('无法读取标签', 'Could not load tags')}
          detail={list.error}
          onRetry={() => void list.refresh()}
          retryLabel={t('重试', 'Retry')}
        />
      ) : !list.page ? (
        <PageState kind="loading" title={t('正在读取标签…', 'Loading tags…')} />
      ) : !list.page.items.length ? (
        <div className="tag-empty">
          <Hash size={24} />
          <h2>
            {query || collectionId
              ? t('没有匹配的标签', 'No matching tags')
              : t('创建第一个标签', 'Create your first tag')}
          </h2>
          <p>
            {t(
              '标签可以用于笔记、资料、摘录和分类。',
              'Tags work with notes, sources, excerpts and collections.',
            )}
          </p>
          <button className="secondary-button" disabled={locked} onClick={onCreate}>
            {t('新建标签', 'New tag')}
          </button>
        </div>
      ) : (
        <ul
          ref={viewport}
          className="tag-rows"
          onScroll={remember}
          aria-label={t('标签列表', 'Tag list')}
          aria-busy={list.loading}
        >
          {list.page.items.map((item) => (
            <li className={`tag-row ${tagToneClass(item.tag.id)}`} key={item.tag.id}>
              <button
                className="tag-row-main"
                onClick={() => {
                  remember();
                  onOpen(item.tag.id);
                }}
              >
                <Hash size={18} />
                <span className="tag-row-copy">
                  <strong>{item.tag.display_name}</strong>
                  {item.description && (
                    <span className="tag-description" title={item.description}>
                      {item.description}
                    </span>
                  )}
                  <small>
                    {item.updated_at
                      ? t(
                          `更新于 ${displayDate(item.updated_at, language)}`,
                          `Updated ${displayDate(item.updated_at, language)}`,
                        )
                      : t('历史更新时间未记录', 'Historical update time was not recorded')}
                  </small>
                </span>
                <span className="tag-usage">
                  {t(`${item.object_count} 项`, `${item.object_count} items`)}
                </span>
              </button>
              <PopupMenu
                label={t(
                  `标签“${item.tag.display_name}”的操作`,
                  `Actions for tag “${item.tag.display_name}”`,
                )}
                trigger={<MoreHorizontal size={18} />}
                disabled={locked}
                items={[
                  {
                    id: 'edit',
                    label: t('编辑标签', 'Edit tag'),
                    icon: <Pencil />,
                    onSelect: () => onEdit(item.tag.id),
                  },
                  {
                    id: 'delete',
                    label: t('删除标签', 'Delete tag'),
                    icon: <Trash2 />,
                    onSelect: () => onDelete(item.tag.id),
                  },
                ]}
              />
            </li>
          ))}
        </ul>
      )}
    </PageFrame>
  );
}

function TaggedContent({
  identity,
  tagId,
  collection,
  refresh,
  onBack,
  onOpen,
  onAdd,
  onTrash,
  onEdit,
  onDelete,
  onRemove,
  locked,
  feedback,
  scopePicker,
}: {
  identity: WorkspaceIdentity;
  tagId: string;
  collection: { id: string; name: string } | null;
  refresh: number;
  onBack: () => void;
  onOpen: (object: TaggedObject) => void;
  onAdd: () => void;
  onTrash: () => void;
  onEdit: () => void;
  onDelete: () => void;
  locked: boolean;
  feedback: ReactNode;
  scopePicker: ReactNode;
  onRemove: (object: TaggedObject) => void;
}) {
  const { t } = useI18n();
  const [mode, setMode] = usePageNavigationState<'active' | 'trash'>(
    `tags.content.mode/${tagId}`,
    'active',
  );
  const [tag, setTag] = useState<TagDetails | null>(null);
  const [error, setError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const workspaceKey = JSON.stringify([identity.local_instance_id, identity.workspace_id]);
  useEffect(() => {
    let live = true;
    setError('');
    void call({ kind: 'tags_get', payload: { tag_id: tagId } }, 'tag_details', identity)
      .then((value) => {
        if (live) setTag(value);
      })
      .catch((failure) => {
        if (live) setError(tagErrorMessage(failure, t));
      });
    return () => {
      live = false;
    };
  }, [workspaceKey, tagId, refresh, attempt]);
  const scope = JSON.stringify([workspaceKey, tagId, mode, collection?.id ?? null]);
  const { list, viewport, remember } = useRememberedList<TaggedObject>(
    `tags.content.position/${tagId}`,
    scope,
    async (cursor, limit) => {
      const page = await call(
        {
          kind: 'tagged_objects',
          payload: {
            tag_id: tagId,
            page: cursor ? Number(cursor) : 1,
            limit,
            trashed: mode === 'trash',
            collection_id: collection?.id ?? null,
          },
        },
        'tagged_objects_page',
        identity,
      );
      return {
        items: page.items,
        total: page.total,
        page_index: page.page,
        next_cursor: page.page * limit < page.total ? String(page.page + 1) : null,
      };
    },
  );
  const latest = useRef(list);
  latest.current = list;
  useEffect(() => {
    if (refresh) void latest.current.refresh();
  }, [refresh]);
  return (
    <PageFrame
      className="tags-directory-page"
      title={tag?.tag.display_name ?? t('关联内容', 'Tagged content')}
      description={
        tag?.description ||
        t('查看这个标签连接的真实内容。', 'Explore the content connected by this tag.')
      }
      actions={
        <>
          <button
            className="secondary-button"
            aria-label={t('返回标签', 'Back to tags')}
            title={t('返回标签', 'Back to tags')}
            onClick={onBack}
          >
            <ArrowLeft size={16} />
            <span className="tag-action-label">{t('返回标签', 'Back to tags')}</span>
          </button>
          <button
            className="secondary-button"
            aria-label={t('编辑标签', 'Edit tag')}
            title={t('编辑标签', 'Edit tag')}
            disabled={locked || !tag}
            onClick={onEdit}
          >
            <Pencil size={15} />
            <span className="tag-action-label">{t('编辑标签', 'Edit tag')}</span>
          </button>
          <PopupMenu
            label={t('更多标签操作', 'More tag actions')}
            trigger={<MoreHorizontal size={18} />}
            disabled={locked || !tag}
            items={[
              {
                id: 'delete',
                label: t('删除标签', 'Delete tag'),
                icon: <Trash2 />,
                onSelect: onDelete,
              },
            ]}
          />
        </>
      }
      toolbar={
        <>
          <SectionSwitch
            label={t('内容状态', 'Content state')}
            value={mode}
            onChange={setMode}
            options={[
              { value: 'active', label: t('使用中', 'In use') },
              { value: 'trash', label: t('已回收', 'Trashed') },
            ]}
          />
          {scopePicker}
          <span className="spacer" />
          <button
            className="secondary-button"
            aria-label={
              mode === 'trash'
                ? t('前往回收站', 'Open trash')
                : t('添加关联内容', 'Add linked content')
            }
            title={
              mode === 'trash'
                ? t('前往回收站', 'Open trash')
                : t('添加关联内容', 'Add linked content')
            }
            disabled={locked}
            onClick={mode === 'trash' ? onTrash : onAdd}
          >
            {mode === 'trash' ? <Trash2 size={16} /> : <Plus size={16} />}
            <span className="tag-action-label">
              {mode === 'trash'
                ? t('前往回收站', 'Open trash')
                : t('添加关联内容', 'Add linked content')}
            </span>
          </button>
        </>
      }
      footer={<ListPagination list={list} />}
    >
      {feedback}
      {error || list.error ? (
        <PageState
          kind="error"
          title={t('无法读取关联内容', 'Could not load tagged content')}
          detail={error || list.error}
          retryLabel={t('重新读取', 'Reload')}
          onRetry={() => {
            setAttempt((value) => value + 1);
            void list.refresh();
          }}
        />
      ) : !tag || !list.page ? (
        <PageState kind="loading" title={t('正在读取关联内容…', 'Loading tagged content…')} />
      ) : !list.page.items.length ? (
        <div className="tag-empty">
          <Hash size={24} />
          <h2>{t('这里还没有内容', 'No content here yet')}</h2>
          <p>
            {collection
              ? t(
                  '当前分类内没有符合条件的关联内容。',
                  'No matching tagged content in this collection.',
                )
              : t('选择已有内容，为它们添加这个标签。', 'Choose existing content to add this tag.')}
          </p>
          <button
            className="secondary-button"
            disabled={locked}
            onClick={mode === 'trash' ? onTrash : onAdd}
          >
            {mode === 'trash'
              ? t('前往回收站', 'Open trash')
              : t('添加关联内容', 'Add linked content')}
          </button>
        </div>
      ) : (
        <ul
          className="tag-rows"
          ref={viewport}
          onScroll={remember}
          aria-label={t('关联内容列表', 'Tagged content list')}
          aria-busy={list.loading}
        >
          {list.page.items.map((item) => {
            const Icon =
              item.kind === 'collection'
                ? FolderClosed
                : item.kind === 'source'
                  ? BookOpen
                  : FileText;
            const kind =
              item.kind === 'collection'
                ? t('分类', 'Collection')
                : item.kind === 'source'
                  ? t('资料', 'Source')
                  : item.kind === 'annotation'
                    ? t('摘录', 'Excerpt')
                    : t('笔记', 'Note');
            return (
              <li className="tag-row" key={item.id}>
                <button
                  className="tag-row-main"
                  disabled={item.trashed}
                  onClick={() => {
                    remember();
                    onOpen(item);
                  }}
                >
                  <Icon size={18} />
                  <span className="tag-row-copy">
                    <strong>{item.title || kind}</strong>
                    <small>
                      {kind}
                      {item.source_trashed
                        ? ` · ${t('原资料已回收', 'Source is in trash')}`
                        : item.trashed
                          ? ` · ${t('已回收', 'Trashed')}`
                          : ''}
                    </small>
                  </span>
                  {item.archived && (
                    <span className="tag-object-state">
                      <Archive size={13} />
                      {t('已归档', 'Archived')}
                    </span>
                  )}
                </button>
                {item.trashed ? (
                  <button className="text-button" onClick={onTrash}>
                    {t('前往恢复', 'Restore from trash')}
                  </button>
                ) : (
                  <PopupMenu
                    label={t(`“${item.title}”的标签操作`, `Tag actions for “${item.title}”`)}
                    trigger={<MoreHorizontal size={18} />}
                    disabled={locked}
                    items={[
                      {
                        id: 'remove',
                        label: t('移除此标签关联', 'Remove tag link'),
                        icon: <X />,
                        onSelect: () => onRemove(item),
                      },
                    ]}
                  />
                )}
              </li>
            );
          })}
        </ul>
      )}
    </PageFrame>
  );
}

function TagCollectionPicker({
  identity,
  value,
  onChange,
}: {
  identity: WorkspaceIdentity;
  value: { id: string; name: string } | null;
  onChange: (value: { id: string; name: string } | null) => void;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [settled, setSettled] = useState('');
  useEffect(() => {
    const timer = setTimeout(() => setSettled(query), 150);
    return () => clearTimeout(timer);
  }, [query]);
  const list = usePagedList<CollectionDetail>(
    JSON.stringify([identity.local_instance_id, identity.workspace_id, settled]),
    (cursor, limit) =>
      call(
        {
          kind: 'collections_list',
          payload: {
            limit,
            cursor,
            query: settled.trim() || null,
            trashed: false,
            archived: false,
            pinned_only: false,
          },
        },
        'collections',
        identity,
      ),
    20,
  );
  const options = [
    { value: '', label: t('全部分类', 'All collections') },
    ...(value && !list.page?.items.some((item) => item.id === value.id)
      ? [{ value: value.id, label: value.name }]
      : []),
    ...(list.page?.items ?? []).map((item) => ({ value: item.id, label: item.name })),
  ];
  return (
    <div className="tag-scope">
      <Combobox
        aria-label={t('分类范围', 'Collection scope')}
        value={value?.id ?? ''}
        options={options}
        filterOptions={false}
        onQueryChange={setQuery}
        loading={list.loading}
        onValueChange={(id) => {
          const found = options.find((item) => item.value === id);
          if (found) onChange(id ? { id, name: found.label } : null);
        }}
        footer={
          <>
            {list.error ? (
              <div role="alert">
                {list.error}
                <button className="text-button" onClick={() => void list.refresh()}>
                  {t('重试', 'Retry')}
                </button>
              </div>
            ) : (
              <ListPagination list={list} variant="inline" />
            )}
          </>
        }
      />
    </div>
  );
}

function TagDialog({
  title,
  children,
  footer,
  onClose,
  busy = false,
  className = '',
}: {
  title: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  busy?: boolean;
  className?: string;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const heading = useId();
  useEffect(() => {
    const element = dialog.current!;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    return () => {
      if (element.open) element.close();
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className={`tag-edit-dialog ${className}`}
      aria-labelledby={heading}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
    >
      <header>
        <h2 id={heading} title={title}>
          {title}
        </h2>
        <button
          className="icon-button"
          disabled={busy}
          aria-label={t('关闭标签对话框', 'Close tag dialog')}
          onClick={onClose}
        >
          <X size={18} />
        </button>
      </header>
      <div className="tag-edit-body">{children}</div>
      <footer>{footer}</footer>
    </dialog>
  );
}
function TagEditor({
  tag,
  busy,
  pending,
  pendingFeedback,
  error,
  onSave,
  onClose,
  onReload,
}: {
  tag: TagDetails | null;
  busy: boolean;
  pending: boolean;
  pendingFeedback: ReactNode;
  error: string | null;
  onSave: (name: string, description: string) => Promise<void>;
  onClose: () => void;
  onReload?: () => void;
}) {
  const { t } = useI18n();
  const formId = useId();
  const [name, setName] = useState(tag?.tag.display_name ?? '');
  const [description, setDescription] = useState(tag?.description ?? '');
  const previous = useRef(tag);
  useEffect(() => {
    const before = previous.current;
    if (before?.tag.id === tag?.tag.id) {
      setName((current) =>
        current === (before?.tag.display_name ?? '') ? (tag?.tag.display_name ?? '') : current,
      );
      setDescription((current) =>
        current === (before?.description ?? '') ? (tag?.description ?? '') : current,
      );
    } else {
      setName(tag?.tag.display_name ?? '');
      setDescription(tag?.description ?? '');
    }
    previous.current = tag;
  }, [tag]);
  const valid =
    [...name.trim()].length > 0 &&
    [...name.trim()].length <= 100 &&
    [...description].length <= 4000;
  return (
    <TagDialog
      title={tag ? t('编辑标签', 'Edit tag') : t('新建标签', 'New tag')}
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <button className="secondary-button" disabled={busy} onClick={onClose}>
            {t('取消', 'Cancel')}
          </button>
          <button
            type="submit"
            form={formId}
            className="primary-button"
            disabled={busy || pending || !valid}
          >
            {busy ? t('正在保存…', 'Saving…') : t('保存标签', 'Save tag')}
          </button>
        </>
      }
    >
      <form
        id={formId}
        onSubmit={(event) => {
          event.preventDefault();
          if (valid && !busy && !pending) void onSave(name.trim(), description.trim());
        }}
      >
        <label>
          <span>{t('标签名称', 'Tag name')}</span>
          <input
            className="ui-input"
            autoFocus
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={200}
            disabled={busy}
            required
            aria-describedby={`${formId}-name-limit`}
          />
        </label>
        <small id={`${formId}-name-limit`} className="tag-edit-impact">
          {t(
            `1–100 字，已输入 ${[...name.trim()].length} 字。`,
            `1–100 characters; ${[...name.trim()].length} entered.`,
          )}
        </small>
        <label>
          <span>{t('说明（可选）', 'Description (optional)')}</span>
          <textarea
            className="ui-input"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            maxLength={8000}
            disabled={busy}
            rows={4}
            aria-describedby={`${formId}-description-limit`}
          />
        </label>
        <small id={`${formId}-description-limit`} className="tag-edit-impact">
          {t(
            `最多 4000 字，已输入 ${[...description].length} 字。`,
            `Up to 4,000 characters; ${[...description].length} entered.`,
          )}
        </small>
        {tag && (
          <p className="tag-edit-impact">
            {t(
              `这是全局标签，改名会更新 ${tag.object_count} 个关联对象中的显示名称，待处理的旧标签建议将跳过。`,
              `This is a global tag. Renaming updates its name on ${tag.object_count} linked objects and skips pending suggestions based on its old name.`,
            )}
          </p>
        )}
        {error && (
          <div className="tag-feedback is-error" role="alert">
            {error}
            {onReload && (
              <button
                type="button"
                className="text-button"
                disabled={busy || pending}
                onClick={onReload}
              >
                {t('重新读取标签', 'Reload tag')}
              </button>
            )}
          </div>
        )}
        {pendingFeedback}
      </form>
    </TagDialog>
  );
}

const noExcludedMembers: ReadonlySet<string> = new Set();
function TagAssignmentDialog({
  identity,
  tag,
  selected,
  busy,
  pending,
  error,
  pendingFeedback,
  onChange,
  onClose,
  onCreate,
  onApply,
}: {
  identity: WorkspaceIdentity;
  tag: TagDetails;
  selected: string[];
  busy: boolean;
  pending: boolean;
  error: string | null;
  pendingFeedback: ReactNode;
  onChange: (ids: string[]) => void;
  onClose: () => void;
  onCreate: () => void;
  onApply: () => void;
}) {
  const { t } = useI18n();
  return (
    <TagDialog
      className="tag-assignment-dialog"
      title={t(`添加到“${tag.tag.display_name}”`, `Add to “${tag.tag.display_name}”`)}
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <span className="tag-assignment-count">
            {t(`已选择 ${selected.length} 项`, `Selected ${selected.length}`)}
            {selected.length > 0 && (
              <button
                type="button"
                className="text-button"
                disabled={busy || pending}
                onClick={() => onChange([])}
              >
                {t('清空', 'Clear')}
              </button>
            )}
          </span>
          <button className="secondary-button" disabled={busy} onClick={onClose}>
            {t('取消', 'Cancel')}
          </button>
          <button
            className="primary-button"
            disabled={busy || pending || !selected.length}
            onClick={onApply}
          >
            {busy ? t('正在添加…', 'Adding…') : t('添加关联', 'Add links')}
          </button>
        </>
      }
    >
      <p className="tag-assignment-help">
        {t(
          '可跨类型、跨页选择，已有标签关联不会重复创建。',
          'Select across types and pages. Existing tag links will not be duplicated.',
        )}
        <button type="button" className="text-button" disabled={busy || pending} onClick={onCreate}>
          {t('前往新建内容', 'Create content first')}
        </button>
      </p>
      {error && (
        <p className="tag-feedback is-error" role="alert">
          {error}
        </p>
      )}
      {pendingFeedback}
      <MemberPicker
        context="tag"
        identity={identity}
        selected={selected}
        onChange={onChange}
        inCollection={noExcludedMembers}
        disabled={busy || pending}
        onClose={() => {
          if (!busy) onClose();
        }}
      />
    </TagDialog>
  );
}
