import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import {
  AlertCircle,
  CheckSquare2,
  FileText,
  Folder,
  Highlighter,
  Library,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Search,
  Trash2,
  X,
} from 'lucide-react';
import type {
  Note,
  ObjectKind,
  TrashItem,
  TrashPage as TrashPageData,
  TrashPreview as DeletePreview,
  TrashSelection,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { ReadDocument } from '../editor/ReadDocument';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Tooltip } from '../ui/Tooltip';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { PageFrame } from '../layout/PageFrame';
import { QueryFeedback } from './OrganizationControls';
import { nextTrashFocus } from './note-lifecycle';
import { summarizeTrashSelection } from './trash-restore-summary';
import type { TrashActionSnapshot } from './trash-actions';
import './organization-pages.css';
import './trash-page.css';

export type TrashPageProps = {
  identity: WorkspaceIdentity;
  generation: number;
  busy: boolean;
  state?: TrashActionSnapshot | null;
  onRestore: (items: TrashItem[]) => void | Promise<void>;
  onDelete: (preview: DeletePreview) => void | Promise<void>;
};
const itemIcons = { note: FileText, source: Library, annotation: Highlighter, collection: Folder };
function itemKind(kind: ObjectKind, t: (zh: string, en: string) => string) {
  return {
    note: t('笔记', 'Note'),
    source: t('资料', 'Source'),
    annotation: t('摘录', 'Excerpt'),
    collection: t('集合', 'Collection'),
  }[kind];
}
function itemTitle(item: TrashItem, t: (zh: string, en: string) => string) {
  return item.title || t('未命名项目', 'Untitled item');
}

export function TrashPage(props: TrashPageProps) {
  const scope = JSON.stringify([props.identity.local_instance_id, props.identity.workspace_id]);
  return <TrashContent key={scope} {...props} />;
}

function TrashContent({ identity, generation, busy, state, onRestore, onDelete }: TrashPageProps) {
  const { t, language } = useI18n();
  const [refresh, setRefresh] = useState(0);
  const [search, setSearch] = useState('');
  const [query, setQuery] = useState('');
  const [selecting, setSelecting] = useState(false);
  const [checked, setChecked] = useState<Map<string, TrashItem>>(() => new Map());
  const [activeId, setActiveId] = useState<string | null>(null);
  const [revealId, setRevealId] = useState<string | null>(null);
  const [deletion, setDeletion] = useState<TrashSelection | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [focusVersion, setFocusVersion] = useState(0);
  const refreshButton = useRef<HTMLButtonElement>(null);
  const allCheckbox = useRef<HTMLInputElement>(null);
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const focusAfterAction = useRef<{
    origin: HTMLElement | null;
    objectId: string | null;
    settled: boolean;
    preferOrigin: boolean;
  } | null>(null);
  const acknowledged = useRef<{ operationId: string; ids: Set<string> } | null>(null);
  const listId = useId();
  const helpId = useId();
  const scope = JSON.stringify([
    identity.local_instance_id,
    identity.workspace_id,
    generation,
    query,
    refresh,
  ]);
  const entries = usePagedList<TrashItem>(scope, (cursor, size) =>
    call(
      { kind: 'trash_list', payload: { limit: size, cursor, query: query || null } },
      'trash',
      identity,
    ),
  );
  // Only trash_list answers with a real total, so the page cast keeps it for the pager.
  const page = entries.page as TrashPageData | null;
  const items = page?.items ?? [];
  const active = items.find((item) => item.object_id === activeId) ?? null;
  const checkedItems = [...checked.values()];
  const selection = summarizeTrashSelection(checkedItems);
  const allLoadedChecked = items.length > 0 && items.every((item) => checked.has(item.object_id));
  const someLoadedChecked = items.some((item) => checked.has(item.object_id));
  const unavailableParents = checkedItems.filter(
    (item) =>
      item.kind === 'annotation' &&
      item.source_trashed &&
      (!item.parent_source_id || !checked.has(item.parent_source_id)),
  );
  const locked = busy || !!state?.busy || !!state?.pending;
  const empty = page !== null && !items.length && !entries.loading && !entries.error;
  useEffect(() => {
    const timer = window.setTimeout(() => setQuery(search.trim()), 160);
    return () => window.clearTimeout(timer);
  }, [search]);
  useLayoutEffect(() => {
    if (allCheckbox.current)
      allCheckbox.current.indeterminate = someLoadedChecked && !allLoadedChecked;
  }, [someLoadedChecked, allLoadedChecked]);
  useEffect(() => {
    const result = state?.result;
    if (!result) return;
    const previous = acknowledged.current;
    const known = previous?.operationId === result.operation_id ? previous.ids : new Set<string>();
    const newlyDone = result.items
      .filter((item) => item.status === 'done' && !known.has(item.object_id))
      .map((item) => item.object_id);
    if (!newlyDone.length) return;
    for (const id of newlyDone) known.add(id);
    acknowledged.current = { operationId: result.operation_id, ids: known };
    setChecked((old) => {
      const next = new Map(old);
      for (const id of newlyDone) next.delete(id);
      return next;
    });
    setActiveId((old) =>
      nextTrashFocus(
        items.map((item) => item.object_id),
        newlyDone,
        old,
      ),
    );
  }, [state?.result]);
  useLayoutEffect(() => {
    const restore = focusAfterAction.current;
    if (!restore?.settled || locked || entries.loading || (!page && !entries.error) || deletion)
      return;
    if (document.activeElement !== document.body && document.activeElement !== restore.origin) {
      // A user may move to another control while a slow operation is finishing.
      focusAfterAction.current = null;
      return;
    }
    focusAfterAction.current = null;
    const next = restore.objectId ? rowButtons.current.get(restore.objectId) : null;
    const origin =
      restore.preferOrigin && restore.origin?.isConnected && !restore.origin.matches(':disabled')
        ? restore.origin
        : null;
    const target = origin ?? next ?? refreshButton.current;
    if (target && !target.matches(':disabled') && !target.closest('[inert]'))
      target.focus({ preventScroll: true });
  }, [scope, locked, entries.loading, page, entries.error, deletion, focusVersion]);
  useEffect(() => {
    if (!revealId || entries.loading || !page || entries.error) return;
    if (items.some((item) => item.object_id === revealId)) {
      setActiveId(revealId);
      rowButtons.current.get(revealId)?.focus({ preventScroll: true });
      rowButtons.current.get(revealId)?.scrollIntoView({ block: 'nearest' });
      setRevealId(null);
    } else if (entries.canNext) {
      void entries.next();
    } else {
      setRevealId(null);
      setActionError(
        t(
          '所属资料已不在当前回收站中。请刷新后核对摘录状态。',
          'The source is no longer in this Trash. Refresh to check the excerpt’s current state.',
        ),
      );
    }
  }, [revealId, entries.loading, page, entries.error]);
  function onRefresh() {
    focusAfterAction.current = {
      origin: document.activeElement instanceof HTMLElement ? document.activeElement : null,
      objectId: activeId,
      settled: true,
      preferOrigin: true,
    };
    setActionError(null);
    setRefresh((value) => value + 1);
  }
  function toggle(item: TrashItem) {
    setChecked((old) => {
      const next = new Map(old);
      if (next.has(item.object_id)) next.delete(item.object_id);
      else next.set(item.object_id, item);
      return next;
    });
  }
  function rememberAction(ids: string[]) {
    focusAfterAction.current = {
      origin: document.activeElement instanceof HTMLElement ? document.activeElement : null,
      objectId: nextTrashFocus(
        items.map((item) => item.object_id),
        ids,
        activeId,
      ),
      settled: false,
      preferOrigin: false,
    };
  }
  function askDelete(selection: TrashSelection) {
    if (locked) return;
    setActionError(null);
    rememberAction(
      selection.kind === 'all' ? items.map((item) => item.object_id) : selection.object_ids,
    );
    setDeletion(selection);
  }
  async function restore(targets: TrashItem[]) {
    if (locked || !targets.length) return;
    rememberAction(targets.map((item) => item.object_id));
    setActionError(null);
    try {
      await onRestore(targets);
      if (focusAfterAction.current) focusAfterAction.current.settled = true;
      setFocusVersion((value) => value + 1);
    } catch (error) {
      focusAfterAction.current = null;
      setActionError(message(error));
    }
  }
  async function permanentlyDelete(preview: DeletePreview) {
    setDeletion(null);
    try {
      await onDelete(preview);
      if (focusAfterAction.current) focusAfterAction.current.settled = true;
      setFocusVersion((value) => value + 1);
    } catch (error) {
      focusAfterAction.current = null;
      setActionError(message(error));
    }
  }
  function revealParent(parentId: string) {
    setSearch('');
    setQuery('');
    setSelecting(false);
    setChecked(new Map());
    setRevealId(parentId);
    setActionError(null);
    setRefresh((value) => value + 1);
  }
  return (
    <PageFrame
      className="org-page trash-page"
      title={t('回收站', 'Trash')}
      description={t(
        '查看已删除内容，恢复需要的项目。',
        'Review deleted content and restore what you need.',
      )}
      actions={
        <div className="trash-heading-actions">
          <Tooltip label={t('刷新回收站', 'Refresh trash')}>
            <button
              ref={refreshButton}
              type="button"
              className="icon-button"
              disabled={locked || entries.loading}
              aria-label={t('刷新回收站', 'Refresh trash')}
              onClick={onRefresh}
            >
              <RefreshCw size={17} aria-hidden="true" />
            </button>
          </Tooltip>
          <button
            type="button"
            className="secondary-button trash-danger-button"
            disabled={locked || !page || (!items.length && !query)}
            onClick={() => askDelete({ kind: 'all' })}
          >
            <Trash2 size={16} aria-hidden="true" />
            {t('清空回收站', 'Empty Trash')}
          </button>
        </div>
      }
      toolbar={
        <div className="trash-toolbar">
          <label className="trash-search">
            <Search size={16} aria-hidden="true" />
            <input
              aria-label={t('搜索回收站', 'Search Trash')}
              placeholder={t('搜索已删除项目…', 'Search deleted items…')}
              value={search}
              disabled={locked}
              onChange={(event) => {
                setSearch(event.target.value);
                setChecked(new Map());
                setActiveId(null);
                setRevealId(null);
              }}
            />
            {search && (
              <button
                type="button"
                className="icon-button"
                aria-label={t('清除回收站搜索', 'Clear Trash search')}
                disabled={locked}
                onClick={() => {
                  setSearch('');
                  setChecked(new Map());
                  setActiveId(null);
                }}
              >
                <X size={14} aria-hidden="true" />
              </button>
            )}
          </label>
          <span className="trash-count" role="status">
            {page ? t(`${page.total} 项`, `${page.total} items`) : t('正在读取…', 'Loading…')}
          </span>
          <button
            type="button"
            className="secondary-button"
            aria-pressed={selecting}
            aria-controls={listId}
            disabled={locked || (!selecting && !items.length)}
            onClick={() => {
              setSelecting(!selecting);
              setChecked(new Map());
            }}
          >
            {selecting ? (
              <X size={16} aria-hidden="true" />
            ) : (
              <CheckSquare2 size={16} aria-hidden="true" />
            )}
            {selecting ? t('完成选择', 'Done selecting') : t('选择项目', 'Select items')}
          </button>
        </div>
      }
    >
      {selecting && (
        <div className="trash-selection-bar" aria-label={t('批量操作', 'Bulk actions')}>
          <label className="trash-check-all">
            <input
              ref={allCheckbox}
              type="checkbox"
              checked={allLoadedChecked}
              disabled={locked || !items.length}
              onChange={() =>
                setChecked((old) => {
                  const next = new Map(old);
                  for (const item of items) {
                    if (allLoadedChecked) next.delete(item.object_id);
                    else next.set(item.object_id, item);
                  }
                  return next;
                })
              }
            />
            <span>{t('选择本页项目', 'Select items on this page')}</span>
          </label>
          <span className="trash-selection-count" role="status">
            {t(`已选 ${checked.size} 项`, `${checked.size} selected`)}
          </span>
          {checked.size > 0 && (
            <span className="trash-selection-summary">
              {selection.counts
                .map(({ kind, count }) => `${itemKind(kind, t)} ${count}`)
                .join(' · ')}
            </span>
          )}
          <div className="trash-selection-actions">
            <button
              type="button"
              className="secondary-button"
              disabled={locked || !checked.size || unavailableParents.length > 0}
              aria-describedby={unavailableParents.length ? helpId : undefined}
              onClick={() => void restore(checkedItems)}
            >
              <RotateCcw size={16} aria-hidden="true" />
              {t('恢复所选', 'Restore selected')}
            </button>
            <button
              type="button"
              className="secondary-button trash-danger-button"
              disabled={locked || !checked.size}
              onClick={() => askDelete({ kind: 'objects', object_ids: [...checked.keys()] })}
            >
              <Trash2 size={16} aria-hidden="true" />
              {t('彻底删除所选', 'Delete selected permanently')}
            </button>
          </div>
          {checked.size > 0 && (
            <p className="trash-selection-scale">
              {selection.excerptsWithoutSource > 0 &&
                t(
                  `其中 ${selection.excerptsWithoutSource} 条摘录需要把所属资料一并选中。`,
                  `${selection.excerptsWithoutSource} of them need their source selected as well.`,
                )}
              {t(
                `恢复会按“资料优先”逐项确认，共 ${selection.total} 步。`,
                `Restore confirms every item one by one, sources first: ${selection.total} steps.`,
              )}
            </p>
          )}
          {unavailableParents.length > 0 && (
            <p id={helpId} className="trash-selection-hint">
              {t(
                '所选摘录的资料仍在回收站，请同时选择所属资料，或先恢复资料。',
                'A selected excerpt’s source is still in Trash. Select its source too, or restore the source first.',
              )}
            </p>
          )}
        </div>
      )}
      {actionError && (
        <div className="org-inline-error trash-local-error" role="alert">
          <AlertCircle size={16} aria-hidden="true" />
          <p>{actionError}</p>
        </div>
      )}
      {revealId && (
        <p className="org-working" role="status">
          <LoaderCircle size={15} className="spin" aria-hidden="true" />
          {t('正在定位所属资料…', 'Finding the parent source…')}
          <button type="button" className="text-button" onClick={() => setRevealId(null)}>
            {t('取消查找', 'Cancel lookup')}
          </button>
        </p>
      )}
      {empty ? (
        <div className="trash-placeholder">
          {query ? (
            <Search size={32} strokeWidth={1.5} aria-hidden="true" />
          ) : (
            <Trash2 size={32} strokeWidth={1.5} aria-hidden="true" />
          )}
          <h2>
            {query
              ? t('没有匹配的已删除项目', 'No matching deleted items')
              : t('回收站是空的', 'Trash is empty')}
          </h2>
          <p>
            {query
              ? t(
                  '换个关键词，或清除搜索查看全部项目。',
                  'Try another keyword, or clear the search to see all items.',
                )
              : t(
                  '移到这里的内容会保留，直到你恢复或彻底删除它们。',
                  'Items moved here stay until you restore or permanently delete them.',
                )}
          </p>
          {query && (
            <button type="button" className="secondary-button" onClick={() => setSearch('')}>
              {t('清除搜索', 'Clear search')}
            </button>
          )}
        </div>
      ) : (
        <div className="trash-columns">
          <aside className="trash-index" aria-label={t('已删除项目', 'Deleted items')}>
            <div className="trash-note-list" id={listId}>
              {items.map((item) => {
                const Icon = itemIcons[item.kind];
                const title = itemTitle(item, t);
                return (
                  <div
                    className="trash-row"
                    data-selected={checked.has(item.object_id) || undefined}
                    key={item.object_id}
                  >
                    {selecting && (
                      <label className="trash-item-check">
                        <input
                          type="checkbox"
                          checked={checked.has(item.object_id)}
                          disabled={locked}
                          onChange={() => toggle(item)}
                          aria-label={t(
                            `选择${itemKind(item.kind, t)}：${title}`,
                            `Select ${itemKind(item.kind, t).toLowerCase()}: ${title}`,
                          )}
                        />
                      </label>
                    )}
                    <button
                      type="button"
                      className="trash-note"
                      ref={(element) => {
                        if (element) rowButtons.current.set(item.object_id, element);
                        else rowButtons.current.delete(item.object_id);
                      }}
                      aria-pressed={activeId === item.object_id}
                      onFocus={(event) => {
                        const waiting = focusAfterAction.current;
                        if (waiting && waiting.origin !== event.currentTarget) {
                          // The refreshed list may unmount this row; retain the
                          // user's newer focus choice across that short gap.
                          focusAfterAction.current = {
                            ...waiting,
                            origin: event.currentTarget,
                            objectId: item.object_id,
                            preferOrigin: true,
                          };
                        }
                      }}
                      onClick={() => {
                        setActiveId(item.object_id);
                        setRevealId(null);
                      }}
                    >
                      <Icon size={18} strokeWidth={1.6} aria-hidden="true" />
                      <span>
                        <strong title={title}>{title}</strong>
                        <small className="trash-note-meta">
                          <span>{itemKind(item.kind, t)}</span>
                          {item.updated_at && (
                            <time dateTime={item.updated_at}>
                              {displayDate(item.updated_at, language)}
                            </time>
                          )}
                        </small>
                        {item.preview && <span className="trash-note-excerpt">{item.preview}</span>}
                      </span>
                    </button>
                  </div>
                );
              })}
            </div>
            <QueryFeedback
              state={entries}
              reload={onRefresh}
              pager={
                <ListPagination list={entries} total={page?.total ?? null} disabled={locked} />
              }
              empty={t('回收站是空的', 'Trash is empty')}
            />
            {selecting && (entries.canPrevious || entries.canNext) && (
              <p className="trash-page-hint">
                {t(
                  '选择会跨页保留，可翻页继续选；清空回收站会单独确认整个回收站。',
                  'Your selection is kept across pages. Empty Trash separately confirms the entire Trash.',
                )}
              </p>
            )}
          </aside>
          {active ? (
            <TrashItemPreview
              key={`${active.object_id}/${active.row_version}`}
              item={active}
              identity={identity}
              busy={locked}
              onRestore={(item) => void restore([item])}
              onDelete={(item) => askDelete({ kind: 'objects', object_ids: [item.object_id] })}
              onRevealParent={revealParent}
            />
          ) : (
            <div className="trash-placeholder">
              <FileText size={30} strokeWidth={1.5} aria-hidden="true" />
              <h2>{t('选择一项查看', 'Choose an item to preview')}</h2>
              <p>
                {t(
                  '先查看内容，再恢复或彻底删除。',
                  'Review the content before restoring or permanently deleting it.',
                )}
              </p>
            </div>
          )}
        </div>
      )}
      {deletion && (
        <DeleteTrashDialog
          identity={identity}
          selection={deletion}
          busy={locked}
          onCancel={() => {
            focusAfterAction.current = null;
            setDeletion(null);
          }}
          onConfirm={(preview) => void permanentlyDelete(preview)}
        />
      )}
    </PageFrame>
  );
}

function TrashItemPreview({
  identity,
  item,
  busy,
  onRestore,
  onDelete,
  onRevealParent,
}: {
  identity: WorkspaceIdentity;
  item: TrashItem;
  busy: boolean;
  onRestore: (item: TrashItem) => void;
  onDelete: (item: TrashItem) => void;
  onRevealParent: (parentId: string) => void;
}) {
  const { t, language } = useI18n();
  const [note, setNote] = useState<Note | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const preview = useRef<HTMLElement>(null);
  const parentHidden = item.kind === 'annotation' && item.source_trashed;
  const Icon = itemIcons[item.kind];
  useEffect(() => {
    if (item.kind !== 'note') return;
    let current = true;
    setNote(null);
    setError(null);
    void call({ kind: 'notes_get', payload: { note_id: item.object_id } }, 'note', identity)
      .then((value) => {
        if (current) setNote(value);
      })
      .catch((cause) => {
        if (current) setError(message(cause));
      });
    return () => {
      current = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, item.object_id, item.kind, attempt]);
  return (
    <article
      ref={preview}
      className="trash-preview"
      tabIndex={-1}
      aria-label={t('内容预览', 'Content preview')}
    >
      <header className="trash-preview-header">
        <span>{t('只读预览', 'Read-only preview')}</span>
        <div className="trash-preview-actions">
          <button
            type="button"
            className="secondary-button"
            disabled={busy || parentHidden || (!!note && !note.trashed)}
            onClick={() => onRestore(item)}
          >
            <RotateCcw size={16} aria-hidden="true" />
            {t('恢复', 'Restore')}
          </button>
          <button
            type="button"
            className="secondary-button trash-danger-button"
            disabled={busy || (!!note && !note.trashed)}
            onClick={() => onDelete(item)}
          >
            <Trash2 size={16} aria-hidden="true" />
            {t('彻底删除', 'Delete permanently')}
          </button>
        </div>
      </header>
      {item.kind === 'note' && error ? (
        <div className="trash-placeholder" role="alert">
          <AlertCircle size={26} aria-hidden="true" />
          <h2>{t('暂时无法读取笔记', 'Could not load this note')}</h2>
          <p>{error}</p>
          <button
            type="button"
            className="secondary-button"
            onClick={() => {
              preview.current?.focus({ preventScroll: true });
              setAttempt((value) => value + 1);
            }}
          >
            <RefreshCw size={15} aria-hidden="true" />
            {t('重新读取', 'Reload')}
          </button>
        </div>
      ) : item.kind === 'note' && !note ? (
        <div className="trash-placeholder" role="status">
          <LoaderCircle size={25} className="spin" aria-hidden="true" />
          <p>{t('正在读取笔记…', 'Loading note…')}</p>
        </div>
      ) : note && !note.trashed ? (
        <div className="trash-placeholder" role="status">
          <RotateCcw size={28} aria-hidden="true" />
          <h2>{t('这篇笔记已恢复', 'This note has been restored')}</h2>
          <p>
            {t('可在笔记列表中继续阅读和编辑。', 'Continue reading and editing it in your notes.')}
          </p>
        </div>
      ) : (
        <div className="trash-document">
          <span className="trash-preview-kind">
            <Icon size={16} aria-hidden="true" />
            {itemKind(item.kind, t)}
          </span>
          <h2>{itemTitle(item, t)}</h2>
          {item.updated_at && (
            <p className="trash-updated">
              {t('最后更新于 ', 'Last updated ')}
              {displayDate(item.updated_at, language)}
            </p>
          )}
          {note ? (
            <ReadDocument note={note} identity={identity} />
          ) : (
            <>
              {item.preview && <p className="trash-text-preview">{item.preview}</p>}
              {item.kind === 'source' && (
                <p className="trash-preview-hint">
                  {t(
                    '恢复资料后即可继续阅读原文。已单独删除的摘录不会随资料一起恢复。',
                    'Restore this source to continue reading. Excerpts deleted separately stay in Trash.',
                  )}
                </p>
              )}
              {item.kind === 'collection' && (
                <p className="trash-preview-hint">
                  {t(
                    '恢复集合会保留原有介绍与成员关系，不改变成员各自的回收状态。',
                    'Restoring the collection preserves its overview and members without changing each member’s Trash status.',
                  )}
                </p>
              )}
              {parentHidden && (
                <div className="trash-parent-hint">
                  <p>
                    {t(
                      '所属资料仍在回收站。请先恢复资料，再恢复这条摘录。',
                      'Its source is still in Trash. Restore the source before restoring this excerpt.',
                    )}
                  </p>
                  {item.parent_source_id && (
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={busy}
                      onClick={() => onRevealParent(item.parent_source_id!)}
                    >
                      <Library size={16} aria-hidden="true" />
                      {t('查看所属资料', 'Show parent source')}
                    </button>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </article>
  );
}

function DeleteTrashDialog({
  identity,
  selection,
  busy,
  onCancel,
  onConfirm,
}: {
  identity: WorkspaceIdentity;
  selection: TrashSelection;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (preview: DeletePreview) => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const [preview, setPreview] = useState<DeletePreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    cancel.current?.focus({ preventScroll: true });
    return () => {
      if (element.open) element.close();
      if (previous?.isConnected && !previous.matches(':disabled') && !previous.closest('[inert]'))
        previous.focus({ preventScroll: true });
    };
  }, []);
  useEffect(() => {
    let current = true;
    setPreview(null);
    setError(null);
    void call({ kind: 'trash_preview', payload: { selection } }, 'trash_preview', identity).then(
      (result) => {
        if (current) setPreview(result);
      },
      (cause) => {
        if (current) setError(message(cause));
      },
    );
    return () => {
      current = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, selection, attempt]);
  return (
    <dialog
      ref={dialog}
      role="alertdialog"
      className="trash-confirm-dialog"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onCancel();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
    >
      <header>
        <span className="trash-confirm-icon">
          <Trash2 size={22} strokeWidth={1.7} aria-hidden="true" />
        </span>
        <h2 id={titleId}>
          {selection.kind === 'all'
            ? t('清空整个回收站', 'Empty the entire Trash')
            : t('彻底删除所选内容', 'Permanently delete selected content')}
        </h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('取消删除', 'Cancel deletion')}
          disabled={busy}
          onClick={onCancel}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <div className="trash-confirm-content">
        <p id={descriptionId} className="trash-confirm-warning">
          {t(
            '彻底删除后，无法从回收站恢复。',
            'Permanently deleted items cannot be restored from Trash.',
          )}
        </p>
        {error ? (
          <div className="trash-confirm-error" role="alert">
            <p>
              {t(
                '暂时无法确认删除范围。尚未删除任何内容。',
                'The deletion scope could not be confirmed. Nothing has been deleted.',
              )}
            </p>
            <p>{error}</p>
            <button
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={() => setAttempt((value) => value + 1)}
            >
              <RefreshCw size={15} aria-hidden="true" />
              {t('重新读取范围', 'Reload scope')}
            </button>
          </div>
        ) : !preview ? (
          <p className="trash-confirm-loading" role="status">
            <LoaderCircle size={18} className="spin" aria-hidden="true" />
            {t('正在核对项目与关联内容…', 'Checking items and related content…')}
          </p>
        ) : (
          <>
            <p className="trash-confirm-count">
              {preview.dependent_count
                ? t(
                    `本次选择 ${preview.selected_count} 项，连同 ${preview.dependent_count} 条关联摘录，共删除 ${preview.total_count} 项。`,
                    `${preview.selected_count} selected ${preview.selected_count === 1 ? 'item' : 'items'} and ${preview.dependent_count} related ${preview.dependent_count === 1 ? 'excerpt' : 'excerpts'}: ${preview.total_count} ${preview.total_count === 1 ? 'item' : 'items'} in total.`,
                  )
                : t(
                    `本次将彻底删除以下 ${preview.total_count} 项。`,
                    `The following ${preview.total_count} ${preview.total_count === 1 ? 'item' : 'items'} will be permanently deleted.`,
                  )}
            </p>
            <ul
              className="trash-confirm-targets"
              aria-label={t('本次删除目标', 'Targets of this deletion')}
            >
              {preview.items.map((item) => {
                const Icon = itemIcons[item.kind];
                return (
                  <li key={item.object_id}>
                    <Icon size={16} aria-hidden="true" />
                    <span>
                      <strong>{itemTitle(item, t)}</strong>
                      <small>{itemKind(item.kind, t)}</small>
                    </span>
                  </li>
                );
              })}
            </ul>
            <p className="trash-confirm-scope">
              {t(
                '这些项目的正文、历史版本及相关整理记录将移除。其他内容中独立保存的引文快照、原件与附件文件、外部原文件及已有备份会保留。此操作不清理这些文件。',
                'Content, history and related organization records for these items will be removed. Independent quote snapshots, originals and attachments, external files and existing backups are kept. This operation does not remove those files.',
              )}
            </p>
            {selection.kind === 'all' && (
              <p className="trash-confirm-scope">
                {t(
                  '此操作针对整个回收站，不限于当前搜索结果或已加载列表。确认范围发生变化时，会停止并要求重新核对。',
                  'This includes the entire Trash, beyond the current search or loaded list. If the confirmed scope changes, deletion stops for another review.',
                )}
              </p>
            )}
          </>
        )}
      </div>
      <footer>
        <button
          ref={cancel}
          type="button"
          className="secondary-button"
          disabled={busy}
          onClick={onCancel}
        >
          {t('取消', 'Cancel')}
        </button>
        <button
          type="button"
          className="secondary-button trash-danger-button trash-confirm-submit"
          disabled={busy || !preview || preview.total_count === 0 || !!error}
          onClick={() => preview && onConfirm(preview)}
        >
          <Trash2 size={16} aria-hidden="true" />
          {preview
            ? t(
                `彻底删除 ${preview.total_count} 项`,
                `Delete ${preview.total_count} ${preview.total_count === 1 ? 'item' : 'items'} permanently`,
              )
            : t('彻底删除', 'Delete permanently')}
        </button>
      </footer>
    </dialog>
  );
}
