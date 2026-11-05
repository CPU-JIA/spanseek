import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { ArrowDownLeft, ArrowUpRight, ChevronDown, Link2, RefreshCw } from 'lucide-react';
import type {
  BodyReference,
  NoteDocument,
  RelationBacklink,
  ResolveLink,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useReferenceContext } from './reference-context';
import { ReferenceChip } from './reference-chip';
import { ListPagination, usePagedList } from '../layout/Pagination.tsx';
import type { CursorPage } from '../layout/pagination.ts';
import { referencePanelScope, type ReferencePanelTab } from './reference-panel-scope';
import { referenceSkeletonRows } from './reference-panel-model';

/**
 * The property list is the one page model that also reports whether the bounded
 * candidate scan reached its limit. `usePagedList` keeps the page object it is
 * handed, so that flag travels with the rows instead of a second request.
 */
type RelationBacklinkCursorPage = CursorPage<RelationBacklink> & { truncated?: boolean };

/**
 * Placeholders for a first load, so the section keeps its height instead of
 * collapsing to one line and jumping when the rows arrive. The reserved rows are
 * decorative; a status line carries the same information to screen readers.
 */
export function ReferenceSkeleton({ rows }: { rows: number }) {
  const { t } = useI18n();
  if (rows <= 0) return null;
  return (
    <>
      <p role="status" className="reference-visually-hidden">
        {t('正在读取引用…', 'Loading references…')}
      </p>
      <div className="reference-skeleton" aria-hidden="true">
        {Array.from({ length: rows }, (_, index) => (
          <span key={index} className="reference-skeleton-row" />
        ))}
      </div>
    </>
  );
}

export function ReferencePanel({
  noteId,
  revisionId,
  identity,
  generation,
  label,
  document,
}: {
  noteId: string;
  revisionId?: string;
  identity: WorkspaceIdentity;
  generation: number;
  label?: string;
  document?: NoteDocument;
}) {
  const { t } = useI18n();
  const context = useReferenceContext();
  const panelId = useId();
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<ReferencePanelTab>('backlinks');
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  // Outgoing occurrences need a saved revision; body backlinks and property
  // relations only need the note, so they stay available without one.
  const activeTab: ReferencePanelTab = tab === 'outgoing' && !revisionId ? 'backlinks' : tab;
  // A collapsed panel reads nothing; every other input is part of the key, so a
  // change starts a new list at page 1 instead of replaying an old cursor stack.
  const bodyKey = referencePanelScope(
    identity,
    open,
    noteId,
    revisionId ?? null,
    activeTab,
    generation,
  );
  const relationKey = referencePanelScope(identity, open, noteId, null, 'relation', generation);
  const queryKey = activeTab === 'relation' ? relationKey : bodyKey;
  const scope = queryKey ?? '';
  const activeScope = useRef(scope);
  activeScope.current = scope;
  const aliases = useMemo(() => {
    const values = new Map<string, string>();
    const visit = (nodes: NoteDocument['content']) => {
      for (const node of nodes) {
        if (
          (node.type === 'wikiLink' || node.type === 'wikiEmbed') &&
          node.attrs?.label &&
          node.attrs.occurrence_id
        )
          values.set(node.attrs.occurrence_id, node.attrs.label);
        if (node.content) visit(node.content);
      }
    };
    visit(document?.content ?? []);
    return values;
  }, [document]);
  // Only the visible tab reads: the inactive list is handed a null scope, which
  // `usePagedList` treats as "read nothing at all".
  const page = usePagedList<BodyReference>(
    activeTab === 'relation' ? null : bodyKey,
    (cursor, size) =>
      call(
        activeTab === 'backlinks'
          ? { kind: 'notes_backlinks', payload: { id: noteId, limit: size, cursor } }
          : {
              kind: 'notes_references',
              payload: { id: noteId, revision_id: revisionId!, limit: size, cursor },
            },
        'body_reference_page',
        identity,
      ),
  );
  const relations = usePagedList<RelationBacklink>(
    activeTab === 'relation' ? relationKey : null,
    (cursor, size) =>
      call(
        { kind: 'notes_relation_backlinks', payload: { id: noteId, limit: size, cursor } },
        'relation_backlink_page',
        identity,
      ),
  );
  const backlinks = page.page?.items ?? [];
  const linked = relations.page?.items ?? [];
  const list = activeTab === 'relation' ? relations : page;
  const rows = activeTab === 'relation' ? linked.length : backlinks.length;
  const truncated = (relations.page as RelationBacklinkCursorPage | null)?.truncated === true;
  const loaded = list.page !== null;
  const skeletons = referenceSkeletonRows({ loading: list.loading, rows });
  useEffect(() => {
    setError(null);
    setOpening(false);
  }, [queryKey]);
  async function navigate(target: ResolveLink, occurrenceId?: string) {
    if (!context?.onOpenReference) return;
    setOpening(true);
    setError(null);
    try {
      await context.onOpenReference(target, occurrenceId);
    } catch (error) {
      if (activeScope.current === scope) setError(message(error));
    } finally {
      if (activeScope.current === scope) setOpening(false);
    }
  }
  const navigateToSource = (reference: BodyReference) =>
    navigate(
      {
        target_id: reference.source_id,
        target_block_id: reference.source_block_id,
        target_revision_id: reference.source_revision_id,
      },
      reference.occurrence_id,
    );
  return (
    <section
      className="reference-panel"
      aria-label={label || t('引用与反链', 'References and backlinks')}
    >
      <button
        className="reference-panel-heading"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((value) => !value)}
      >
        <ChevronDown size={15} className={open ? '' : 'is-collapsed'} />
        <span>{label || t('引用与反链', 'References and backlinks')}</span>
      </button>
      {open && (
        <div id={panelId}>
          <div className="reference-panel-tabs">
            <button aria-pressed={activeTab === 'backlinks'} onClick={() => setTab('backlinks')}>
              <ArrowDownLeft size={14} />
              {t('引用了这里', 'Linked here')}
            </button>
            {revisionId && (
              <button aria-pressed={activeTab === 'outgoing'} onClick={() => setTab('outgoing')}>
                <ArrowUpRight size={14} />
                {t('本文引用', 'Links in this note')}
              </button>
            )}
            <button aria-pressed={activeTab === 'relation'} onClick={() => setTab('relation')}>
              <Link2 size={14} />
              {t('关联到这里', 'Linked by property')}
            </button>
            <span className="spacer" />
            <button
              className="icon-button"
              aria-label={t('刷新引用', 'Refresh references')}
              disabled={list.loading}
              onClick={() => void list.refresh()}
            >
              <RefreshCw size={14} />
            </button>
          </div>
          {(error || list.error) && (
            <p className="inline-error" role="alert">
              {error || list.error}
            </p>
          )}
          {skeletons > 0 && <ReferenceSkeleton rows={skeletons} />}
          {list.loading && skeletons === 0 && (
            <p role="status" className="muted">
              {t('正在读取更多引用…', 'Loading more references…')}
            </p>
          )}
          {loaded && !list.loading && !list.error && rows === 0 && (
            <p className="muted">
              {activeTab === 'backlinks'
                ? t('还没有笔记引用这里。', 'No notes link here yet.')
                : activeTab === 'outgoing'
                  ? t('当前已保存版本没有正文引用。', 'The saved version has no references.')
                  : t(
                      '还没有笔记通过属性关联到这里。',
                      'No notes link here through properties yet.',
                    )}
            </p>
          )}
          {activeTab === 'relation' && truncated && (
            <p className="reference-truncated" role="status">
              {t(
                '已扫描的笔记达到上限，这里可能不是全部结果。',
                'The scanned notes reached their limit, so this list may be incomplete.',
              )}
            </p>
          )}
          <div className="reference-occurrences">
            {activeTab === 'relation'
              ? linked.map((relation, index) => (
                  <article key={`${relation.source_id}/${relation.field_key}/${index}`}>
                    <button
                      className="reference-source"
                      disabled={opening || relation.source_trashed || !context?.onOpenReference}
                      onClick={() =>
                        void navigate({
                          target_id: relation.source_id,
                          target_block_id: null,
                          target_revision_id: null,
                        })
                      }
                    >
                      <Link2 size={14} />
                      <strong>{relation.source_title}</strong>
                      {relation.source_trashed && <span>{t('已回收', 'In Trash')}</span>}
                    </button>
                    <p className="reference-field">{relation.field_key}</p>
                    <p>{relation.context || t('无属性上下文', 'No property context')}</p>
                  </article>
                ))
              : backlinks.map((reference) => (
                  <article key={`${reference.source_id}/${reference.occurrence_id}`}>
                    {activeTab === 'backlinks' ? (
                      <button
                        className="reference-source"
                        disabled={opening || reference.source_trashed || !context?.onOpenReference}
                        onClick={() => void navigateToSource(reference)}
                      >
                        <ArrowDownLeft size={14} />
                        <strong>{reference.source_title}</strong>
                        {reference.source_trashed && <span>{t('已回收', 'In Trash')}</span>}
                      </button>
                    ) : (
                      <ReferenceChip
                        identity={identity}
                        displayLabel={aliases.get(reference.occurrence_id)}
                        attrs={{
                          target_id: reference.target_id,
                          target_block_id: reference.target_block_id,
                          target_revision_id: reference.target_revision_id,
                          label: reference.label,
                        }}
                      />
                    )}
                    <p>{reference.context || t('无文字上下文', 'No text context')}</p>
                    {activeTab === 'outgoing' && (
                      <button
                        className="text-button"
                        disabled={opening || !context?.onOpenReference}
                        onClick={() => void navigateToSource(reference)}
                      >
                        {t('定位这一处引用', 'Locate this occurrence')}
                      </button>
                    )}
                  </article>
                ))}
          </div>
          {activeTab === 'relation' ? (
            <ListPagination
              list={relations}
              className="reference-panel-pagination"
              variant="inline"
            />
          ) : (
            <ListPagination list={page} className="reference-panel-pagination" variant="inline" />
          )}
        </div>
      )}
    </section>
  );
}
