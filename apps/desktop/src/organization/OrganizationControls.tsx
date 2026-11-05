import { useEffect, useState, type ReactNode } from 'react';
import { AlertCircle, Check, FileText, LoaderCircle, RefreshCw, X } from 'lucide-react';
import type { NoteSummary, ResolveLink, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import { addableCount, memberCandidateRows } from './member-candidates';
import type { QuerySnapshot } from './scoped-query';

export type OrganizationPageProps = {
  identity: WorkspaceIdentity;
  onOpenNote: (id: string) => void;
  onOpenTarget?: (target: ResolveLink) => Promise<void>;
  onSearchCollection?: (id: string) => void;
  onAskCollection?: (id: string) => void;
  onCreateInCollection?: (collectionId: string, baseRowVersion: number) => void;
  onChanged?: () => void;
};
export function PageLoadFailure({ error, retry }: { error: string | null; retry: () => void }) {
  const { t } = useI18n();
  return (
    <div className="org-empty" role={error ? 'alert' : 'status'}>
      {error ? <AlertCircle size={24} /> : <LoaderCircle size={24} className="spin" />}
      <p>{error ?? t('正在读取操作记录…', 'Loading operation history…')}</p>
      {error && (
        <button className="secondary-button" onClick={retry}>
          {t('重新读取', 'Reload')}
        </button>
      )}
    </div>
  );
}
export function ActionNotice({
  state,
  retry,
}: {
  state: { busy: boolean; pending: unknown; error: string | null; code: string | null };
  retry: () => void;
}) {
  const { t } = useI18n();
  if (state.busy)
    return (
      <p className="org-working" role="status">
        <LoaderCircle size={15} className="spin" />
        {t('正在保存并确认结果…', 'Saving and confirming the result…')}
      </p>
    );
  if (!state.pending && !state.error) return null;
  return (
    <div className="org-notice" role="alert">
      <AlertCircle size={17} />
      <div>
        {state.pending ? (
          <>
            <strong>{t('上次操作仍需确认', 'The previous operation needs confirmation')}</strong>
            <p>
              {t(
                '已保留原操作，确认前暂停新的修改。重试会核对同一次操作的结果。',
                'The original operation is kept. New changes are paused until its result is confirmed. Retrying uses that same operation.',
              )}
            </p>
          </>
        ) : state.code === 'metadata_conflict' ? (
          <>
            <strong>
              {t(
                '内容已有更新，本次操作未应用',
                'This item changed. Your operation was not applied.',
              )}
            </strong>
            <p>
              {t(
                '请重新读取当前版本，检查后再操作。不会自动覆盖。',
                'Reload the current version and review it before trying again.',
              )}
            </p>
          </>
        ) : null}
        {state.error && <p>{state.error}</p>}
        {!!state.pending && (
          <button className="secondary-button" disabled={state.busy} onClick={retry}>
            <RefreshCw size={15} />
            {state.busy
              ? t('正在确认…', 'Confirming…')
              : t('重试原操作', 'Retry original operation')}
          </button>
        )}
      </div>
    </div>
  );
}
export function QueryFeedback<T>({
  state,
  reload,
  empty,
  pager,
}: {
  state: QuerySnapshot<T>;
  reload: () => void;
  empty: string;
  /** Every list in this area is page-numbered, so its pager is part of the feedback. */
  pager: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <>
      {state.error && (
        <div className="org-inline-error" role="alert">
          <p>{state.error}</p>
          <button type="button" className="text-button" onClick={reload}>
            {t('重新读取', 'Reload')}
          </button>
        </div>
      )}
      {state.loading && (
        <p className="org-loading" role="status">
          <LoaderCircle size={14} className="spin" />
          {t('正在读取…', 'Loading…')}
        </p>
      )}
      {!state.loading && !state.error && state.page && !state.page.items.length && (
        <p className="org-empty-copy">{empty}</p>
      )}
      {pager}
    </>
  );
}
export function NotePicker({
  identity,
  selected,
  onChange,
  multiple = false,
  inCollection,
  disabled = false,
  onClose,
}: {
  identity: WorkspaceIdentity;
  selected: string[];
  onChange: (ids: string[], note?: NoteSummary) => void;
  multiple?: boolean;
  /**
   * Members of the collection being edited. They keep their place on the page,
   * are marked and cannot be picked; the core silently drops ids that are
   * already members, so offering them would promise an add that changes nothing.
   */
  inCollection?: ReadonlySet<string>;
  disabled?: boolean;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [text, setText] = useState('');
  const [query, setQuery] = useState('');
  const [archived, setArchived] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 200);
    return () => clearTimeout(timer);
  }, [text]);
  const scope = `${identity.local_instance_id}/${identity.workspace_id}/notes/${archived}/${JSON.stringify(query)}`;
  const notes = usePagedList<NoteSummary>(scope, (cursor, size) =>
    call(
      {
        kind: 'notes_list',
        payload: {
          limit: size,
          cursor,
          query: query || null,
          trashed: false,
          sort: 'title_asc',
          filter: { tag_ids: [], object_types: [], inbox_state: null, archived },
        },
      },
      'notes',
      identity,
    ),
  );
  const rows = memberCandidateRows(notes.page?.items ?? [], inCollection ?? []);
  return (
    <section
      className="org-picker"
      aria-label={t('选择笔记', 'Choose notes')}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <header>
        <strong>
          {multiple
            ? t('添加已有笔记', 'Add existing notes')
            : t('选择介绍笔记', 'Choose an overview note')}
        </strong>
        <button
          type="button"
          className="icon-button"
          aria-label={t('关闭笔记选择', 'Close note picker')}
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </header>
      <label>
        {t('按标题查找', 'Find by title')}
        <input
          autoFocus
          className="ui-input"
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing && event.keyCode !== 229)
              event.preventDefault();
          }}
          placeholder={t('按标题查找…', 'Find by title…')}
        />
      </label>
      <label className="org-check">
        <input
          type="checkbox"
          checked={archived}
          onChange={(event) => setArchived(event.target.checked)}
        />
        {t('查看已归档笔记', 'Show archived notes')}
      </label>
      <div className="org-picker-options">
        {rows.map(({ item: note, inCollection: present }) => (
          <button
            key={note.id}
            type="button"
            disabled={
              disabled ||
              present ||
              (multiple && selected.length >= 100 && !selected.includes(note.id))
            }
            aria-pressed={selected.includes(note.id)}
            onClick={() =>
              onChange(
                multiple
                  ? selected.includes(note.id)
                    ? selected.filter((id) => id !== note.id)
                    : [...selected, note.id]
                  : [note.id],
                note,
              )
            }
          >
            <FileText size={15} />
            <span>{note.title || t('未命名笔记', 'Untitled note')}</span>
            {present && (
              <small className="org-member-state">{t('已在集合内', 'In this collection')}</small>
            )}
            {selected.includes(note.id) && <Check size={15} />}
          </button>
        ))}
      </div>
      {inCollection && (
        <small className="org-picker-note">
          {t(
            `本页 ${rows.length} 项，其中 ${rows.length - addableCount(rows)} 项已在集合内`,
            `${rows.length} on this page, ${rows.length - addableCount(rows)} already in the collection`,
          )}
        </small>
      )}
      <QueryFeedback
        state={notes}
        reload={() => void notes.refresh()}
        pager={<ListPagination list={notes} disabled={disabled} variant="inline" />}
        empty={
          query
            ? t('没有匹配的笔记。', 'No matching notes.')
            : t('这里还没有笔记。', 'There are no notes here yet.')
        }
      />
      {multiple && (
        <small>
          {t(
            `已选 ${selected.length} 项；每次最多 100 项`,
            `${selected.length} selected; up to 100 per operation`,
          )}
        </small>
      )}
    </section>
  );
}
