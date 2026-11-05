import { useEffect, useId, useRef, useState } from 'react';
import { FileText, Search, X } from 'lucide-react';
import type {
  Annotation,
  KnowledgeObject,
  KnowledgeObjectFilter,
  MetadataCommit,
  NotePage,
  NoteSummary,
  SourceVersion,
  WorkspaceIdentity,
} from '@pkms/contracts';
import type { DraftController } from '../editor/draft-controller';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useCreateNote } from '../organization/useCreateNote';
import {
  annotationNoteActionKey,
  annotationNoteItems,
  appendAnnotationToNote,
  containsAnyAnnotationIntent,
  createAnnotationNoteAction,
  validateAnnotationNoteAction,
  type AnnotationNoteAction,
} from './annotation-to-note';
import { annotationNoteStorage } from './annotation-to-note-storage';
import {
  excerptNoteDocument,
  inheritedExcerptMetadata,
  type InheritedExcerptMetadata,
} from './excerpt-note';
import '../editor/reference-ui.css';
import './annotation-to-note.css';

export function AnnotationToNoteDialog({
  identity,
  annotations,
  controllerFor,
  onSaved,
  onOpenNote,
  onClose,
}: {
  identity: WorkspaceIdentity;
  annotations: Annotation[];
  controllerFor: (noteId: string) => Promise<DraftController>;
  onSaved: () => void;
  onOpenNote: (noteId: string) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const live = useRef(false);
  const sequence = useRef(0);
  const submitting = useRef(false);
  const [mode, setMode] = useState<'append' | 'create'>('append');
  const [noteTitle, setNoteTitle] = useState('摘录笔记');
  const [inherited, setInherited] = useState<InheritedExcerptMetadata | null>(null);
  const [inheritLoading, setInheritLoading] = useState(false);
  const [inheritError, setInheritError] = useState<string | null>(null);
  const [createdNoteId, setCreatedNoteId] = useState<string | null>(null);
  const creation = useCreateNote(identity);
  const [query, setQuery] = useState('');
  const [search, setSearch] = useState('');
  const [archived, setArchived] = useState(false);
  const [page, setPage] = useState<NotePage | null>(null);
  const [selected, setSelected] = useState<NoteSummary | null>(null);
  const [intent, setIntent] = useState<AnnotationNoteAction | null>(null);
  const [ready, setReady] = useState(false);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [storageReload, setStorageReload] = useState(0);
  const key = annotationNoteActionKey(
    identity,
    annotations.map((item) => item.id),
  );
  const activeKey = useRef(key);
  activeKey.current = key;
  useEffect(() => {
    live.current = true;
    dialog.current?.showModal();
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    let current = true;
    setReady(false);
    setIntent(null);
    setError(null);
    void annotationNoteStorage
      .readSelection(
        identity,
        annotations.map((item) => item.id),
      )
      .then((saved) => {
        if (saved) validateAnnotationNoteAction(saved, identity);
        if (current) {
          setIntent(saved ?? null);
          setReady(true);
        }
      })
      .catch((e) => {
        if (current) setError(message(e));
      });
    return () => {
      current = false;
    };
  }, [key, identity.local_instance_id, identity.workspace_id, storageReload]);
  useEffect(() => {
    if (intent && mode === 'create') setMode('append');
  }, [intent, mode]);
  useEffect(() => {
    if (mode !== 'create') return;
    let current = true;
    setInheritLoading(true);
    setInheritError(null);
    setInherited(null);
    void (async () => {
      const versionIds = [...new Set(annotations.map((item) => item.source_version_id))];
      const versions = await Promise.all(
        versionIds.map((id) =>
          call(
            { kind: 'sources_version', payload: { source_version_id: id } },
            'source_version',
            identity,
          ),
        ),
      );
      const sourceIds = [...new Set(versions.map((version: SourceVersion) => version.source_id))];
      if (!sourceIds.length)
        throw new Error(t('找不到摘录来源。', 'The excerpt source was not found.'));
      const filter: KnowledgeObjectFilter = {
        kinds: [],
        collection_id: null,
        uncategorized: false,
        tag_ids: [],
        object_ids: sourceIds,
        object_types: [],
        inbox_state: null,
        archived: false,
        trashed: false,
      };
      const page = await call(
        {
          kind: 'knowledge_objects_list',
          payload: { limit: sourceIds.length, cursor: null, query: null, filter },
        },
        'knowledge_objects',
        identity,
      );
      if (current) setInherited(inheritedExcerptMetadata(page.items));
    })()
      .catch((caught) => {
        if (current) setInheritError(message(caught));
      })
      .finally(() => {
        if (current) setInheritLoading(false);
      });
    return () => {
      current = false;
    };
  }, [annotations, identity, key, mode, t]);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 180);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    const token = ++sequence.current;
    setPage(null);
    setSelected(null);
    setLoading(true);
    setSearchError(null);
    void call(
      {
        kind: 'notes_list',
        payload: {
          limit: 30,
          cursor: null,
          trashed: false,
          filter: { inbox_state: null, tag_ids: [], object_types: [], archived },
          sort: 'title_asc',
          query: search || null,
        },
      },
      'notes',
      identity,
    )
      .then((next) => {
        if (token === sequence.current) setPage(next);
      })
      .catch((e) => {
        if (token === sequence.current) setSearchError(message(e));
      })
      .finally(() => {
        if (token === sequence.current) setLoading(false);
      });
    return () => {
      sequence.current++;
    };
  }, [identity.local_instance_id, identity.workspace_id, search, archived, reload]);
  async function more() {
    if (loading || !page?.next_cursor) return;
    const token = sequence.current;
    setLoading(true);
    setSearchError(null);
    try {
      const next = await call(
        {
          kind: 'notes_list',
          payload: {
            limit: 30,
            cursor: page.next_cursor,
            trashed: false,
            filter: { inbox_state: null, tag_ids: [], object_types: [], archived },
            sort: 'title_asc',
            query: search || null,
          },
        },
        'notes',
        identity,
      );
      if (token === sequence.current)
        setPage((current) => ({
          items: [...(current?.items ?? []), ...next.items],
          next_cursor: next.next_cursor,
        }));
    } catch (e) {
      if (token === sequence.current) setSearchError(message(e));
    } finally {
      if (token === sequence.current) setLoading(false);
    }
  }
  async function verifyAnnotations() {
    for (let offset = 0; offset < annotations.length; offset += 8)
      await Promise.all(
        annotations.slice(offset, offset + 8).map(async (annotation) => {
          const [current, resolved] = await Promise.all([
            call(
              { kind: 'annotations_get', payload: { id: annotation.id } },
              'annotation',
              identity,
            ),
            call(
              {
                kind: 'links_resolve',
                payload: {
                  input: {
                    target_id: annotation.id,
                    target_block_id: null,
                    target_revision_id: annotation.source_version_id,
                  },
                },
              },
              'resolved_link',
              identity,
            ),
          ]);
          if (
            current.trashed ||
            current.source_version_id !== annotation.source_version_id ||
            current.row_version !== annotation.row_version ||
            resolved.state !== 'available'
          )
            throw new Error(
              t(
                '摘录或来源已变化、被回收或不可用，笔记尚未保存。请先检查原始摘录。',
                'The excerpt or its source changed or is unavailable. Check the original excerpt before saving.',
              ),
            );
        }),
      );
  }

  async function createNoteFromExcerpts(): Promise<string> {
    if (!creation.controller)
      throw new Error(t('新建笔记服务尚未就绪。', 'Note creation is not ready.'));
    if (!inherited)
      throw new Error(
        t('来源分类与标签尚未读取完成。', 'Source categories and tags are still loading.'),
      );
    const title = noteTitle.trim();
    if (!title) throw new Error(t('请输入笔记标题。', 'Enter a note title.'));
    const firstCollection = inherited.collectionIds[0] ?? null;
    const firstCollectionDetail = firstCollection
      ? await call(
          { kind: 'collections_get', payload: { collection_id: firstCollection } },
          'collection',
          identity,
        )
      : null;
    const document = excerptNoteDocument(
      identity,
      annotations,
      title,
      t('查看原始摘录', 'View original excerpt'),
    );
    const created = await creation.controller.create({
      kind: 'notes_create',
      payload: {
        command_id: crypto.randomUUID(),
        note: {
          title,
          ...(firstCollectionDetail
            ? {
                collection_id: firstCollectionDetail.id,
                base_collection_row_version: firstCollectionDetail.metadata.row_version,
              }
            : {}),
          document_schema: 3,
          document,
        },
      },
    });
    if (!created)
      throw new Error(
        creation.error ?? t('新建笔记尚未确认。', 'Note creation was not confirmed.'),
      );
    const noteId = created.note.id;
    try {
      for (const collectionId of inherited.collectionIds.slice(1)) {
        const detail = await call(
          { kind: 'collections_get', payload: { collection_id: collectionId } },
          'collection',
          identity,
        );
        await call(
          {
            kind: 'collections_add_members',
            payload: {
              command_id: crypto.randomUUID(),
              request: {
                collection_id: detail.id,
                base_row_version: detail.metadata.row_version,
                member_ids: [noteId],
              },
            },
          },
          'collection_commit',
          identity,
        );
      }
      if (inherited.tagNames.length) {
        const noteFilter: KnowledgeObjectFilter = {
          kinds: [],
          collection_id: null,
          uncategorized: false,
          tag_ids: [],
          object_ids: [noteId],
          object_types: [],
          inbox_state: null,
          archived: false,
          trashed: false,
        };
        let rowVersion: number | null = null;
        for (const tagName of inherited.tagNames) {
          const notePage = await call(
            {
              kind: 'knowledge_objects_list',
              payload: { limit: 1, cursor: null, query: null, filter: noteFilter },
            },
            'knowledge_objects',
            identity,
          );
          rowVersion = notePage.items[0]?.object.row_version ?? rowVersion;
          if (rowVersion == null)
            throw new Error(
              t('无法确认新笔记版本。', 'The new note version could not be confirmed.'),
            );
          const commit: MetadataCommit = await call(
            {
              kind: 'tags_assign',
              payload: {
                command_id: crypto.randomUUID(),
                request: {
                  targets: [{ object_id: noteId, base_row_version: rowVersion }],
                  tag: { kind: 'name', name: tagName },
                },
              },
            },
            'metadata_commit',
            identity,
          );
          rowVersion = commit.objects.find((item) => item.id === noteId)?.row_version ?? rowVersion;
        }
      }
    } catch (caught) {
      setCreatedNoteId(noteId);
      throw new Error(
        t(
          `笔记已创建，但继承来源分类或标签失败：${message(caught)}`,
          `The note was created, but inheriting source categories or tags failed: ${message(caught)}`,
        ),
      );
    }
    return noteId;
  }

  async function save() {
    const canSave = mode === 'create' ? true : Boolean(intent || selected);
    if (submitting.current || !ready || !canSave) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    setCreatedNoteId(null);
    try {
      if (mode === 'create') {
        await verifyAnnotations();
        const noteId = await createNoteFromExcerpts();
        if (live.current && activeKey.current === key) {
          onSaved();
          onOpenNote(noteId);
          onClose();
        }
        return;
      }
      const candidate =
        intent ??
        createAnnotationNoteAction(
          identity,
          annotations,
          selected!.id,
          selected!.title,
          t('查看原始摘录', 'View original excerpt'),
        );
      const pending = await annotationNoteStorage.begin(candidate);
      validateAnnotationNoteAction(pending, identity);
      if (live.current && activeKey.current === key) setIntent(pending);
      if (pending.occurrenceId !== candidate.occurrenceId)
        throw new Error(
          t(
            '发现这些摘录的待确认操作，请核对下面的笔记与摘录后继续。',
            'A pending action was found. Review its note and excerpts below before continuing.',
          ),
        );
      const controller = await controllerFor(pending.noteId);
      await appendAnnotationToNote(controller, pending, verifyAnnotations);
      await annotationNoteStorage.remove(pending);
      if (live.current && activeKey.current === key) {
        onSaved();
        onOpenNote(pending.noteId);
        onClose();
      }
    } catch (e) {
      if (live.current && activeKey.current === key) setError(message(e));
    } finally {
      submitting.current = false;
      if (live.current && activeKey.current === key) setBusy(false);
    }
  }
  async function resetUnappliedIntent() {
    if (!intent || submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      const controller = await controllerFor(intent.noteId);
      const { record } = controller.getSnapshot();
      if (
        containsAnyAnnotationIntent(record.document, intent) ||
        containsAnyAnnotationIntent(record.base.document, intent) ||
        (record.pending && containsAnyAnnotationIntent(record.pending.request.document, intent))
      )
        throw new Error(
          t(
            '摘录已进入笔记草稿，请继续确认保存或打开笔记检查。',
            'The excerpt is already in the note draft. Confirm the save or open the note to review it.',
          ),
        );
      await annotationNoteStorage.remove(intent);
      if (live.current && activeKey.current === key) {
        setIntent(null);
        setStorageReload((value) => value + 1);
      }
    } catch (e) {
      if (live.current && activeKey.current === key) setError(message(e));
    } finally {
      submitting.current = false;
      if (live.current && activeKey.current === key) setBusy(false);
    }
  }
  return (
    <dialog
      ref={dialog}
      className="reference-picker"
      aria-labelledby={titleId}
      onCancel={(event) => {
        if (busy) event.preventDefault();
        else onClose();
      }}
    >
      <header>
        <h2 id={titleId}>{t('将摘录加入笔记', 'Add excerpt to note')}</h2>
        <button
          className="icon-button"
          disabled={busy}
          onClick={onClose}
          aria-label={t('关闭', 'Close')}
        >
          <X size={18} />
        </button>
      </header>
      <div
        className="annotation-note-mode"
        role="radiogroup"
        aria-label={t('保存摘录的方式', 'How to save excerpts')}
      >
        <button
          type="button"
          role="radio"
          aria-checked={mode === 'append'}
          disabled={busy}
          onClick={() => {
            setMode('append');
            setError(null);
            setCreatedNoteId(null);
          }}
        >
          {t('追加到已有笔记', 'Add to existing note')}
        </button>
        <button
          type="button"
          role="radio"
          aria-checked={mode === 'create'}
          disabled={busy || !!intent}
          onClick={() => {
            setMode('create');
            setError(null);
            setCreatedNoteId(null);
          }}
        >
          {t('新建笔记', 'Create new note')}
        </button>
      </div>
      <p role="status">
        {t('摘录数量', 'Excerpts')}:{' '}
        {intent ? annotationNoteItems(intent).length : annotations.length}
      </p>
      <div
        className="reference-selection-preview annotation-note-previews"
        aria-label={t('所选摘录', 'Selected excerpts')}
      >
        {(intent
          ? annotationNoteItems(intent).map((item) => ({
              id: item.annotationId,
              quote:
                item.locator?.quote ??
                item.nodes
                  .filter((node) => node.type === 'blockquote')
                  .flatMap((node) => node.content ?? [])
                  .map((node) => node.content?.map((text) => text.text ?? '').join('') ?? '')
                  .join('\n'),
              locator: item.locator,
            }))
          : annotations.map((item) => ({
              id: item.id,
              quote: item.locator.quote,
              locator: item.locator,
            }))
        ).map((item) => (
          <blockquote key={item.id}>
            {item.quote}
            {item.locator?.kind === 'pdf' && (
              <small>
                {' '}
                · {t('页', 'Page')} {item.locator.page_label ?? item.locator.page_index + 1}
              </small>
            )}
          </blockquote>
        ))}
      </div>
      <p className="muted">
        {mode === 'create'
          ? t(
              '所选原文、固定版本出处、页码与批注会创建为一篇新笔记，并继承来源分类与标签。',
              'The selected quotes, fixed source versions, page numbers and comments become a new note, inheriting the source categories and tags.',
            )
          : t(
              '所选原文、固定版本出处、页码与批注会一并追加到同一笔记末尾。',
              'All selected quotes, versioned sources, page numbers, and comments will be saved together at the end of one note.',
            )}
      </p>
      {mode === 'append' ? (
        intent ? (
          <div role="status">
            <strong>{intent.noteTitle}</strong>
            <p className="muted">
              {t(
                '以下是已保留的完整操作；继续会确认这些摘录与目标笔记，不会重复添加。',
                'This is the complete saved action. Continuing confirms these excerpts and their destination without adding them twice.',
              )}
            </p>
            <button
              className="text-button"
              disabled={busy}
              onClick={() => void resetUnappliedIntent()}
            >
              {t('取消尚未追加的操作', 'Cancel an unapplied action')}
            </button>
          </div>
        ) : (
          <>
            <label className="reference-search">
              <Search size={16} />
              <input
                autoFocus
                aria-label={t('搜索笔记标题', 'Search note titles')}
                placeholder={t('搜索笔记标题…', 'Search note titles…')}
                value={query}
                maxLength={512}
                disabled={busy}
                onChange={(event) => setQuery(event.target.value)}
              />
            </label>
            <label className="reference-archive">
              <input
                type="checkbox"
                checked={archived}
                disabled={busy}
                onChange={(event) => setArchived(event.target.checked)}
              />
              {t('查看已归档笔记', 'Show archived notes')}
            </label>
            <div className="reference-target-list" aria-label={t('选择笔记', 'Choose a note')}>
              {page?.items.map((note) => (
                <button
                  key={note.id}
                  className={`reference-target${selected?.id === note.id ? ' is-selected' : ''}`}
                  aria-pressed={selected?.id === note.id}
                  disabled={busy}
                  onClick={() => setSelected(note)}
                >
                  <FileText size={17} />
                  <span>
                    <strong>{note.title}</strong>
                    <small>{note.preview || t('空笔记', 'Empty note')}</small>
                  </span>
                </button>
              ))}
              {loading && <p role="status">{t('正在读取笔记…', 'Loading notes…')}</p>}
              {!loading && page?.items.length === 0 && (
                <p className="muted">{t('没有匹配的笔记。', 'No matching notes.')}</p>
              )}
            </div>
            {page?.next_cursor && (
              <button
                className="secondary-button"
                disabled={busy || loading}
                onClick={() => void more()}
              >
                {t('载入更多', 'Load more')}
              </button>
            )}
            {searchError && (
              <div role="alert" className="inline-error">
                <p>{searchError}</p>
                <button onClick={() => setReload((value) => value + 1)} disabled={busy}>
                  {t('刷新列表', 'Refresh list')}
                </button>
              </div>
            )}
          </>
        )
      ) : (
        <div className="annotation-note-create">
          <label>
            {t('笔记标题', 'Note title')}
            <input
              autoFocus
              maxLength={300}
              value={noteTitle}
              disabled={busy}
              onChange={(event) => setNoteTitle(event.target.value)}
            />
          </label>
          {inheritLoading && (
            <p role="status">
              {t('正在读取来源分类与标签…', 'Reading source categories and tags…')}
            </p>
          )}
          {inheritError && (
            <p className="annotation-note-inherit-error" role="alert">
              {inheritError}
            </p>
          )}
          {inherited && (
            <p className="muted">
              {inherited.collectionNames.length || inherited.tagNames.length
                ? t(
                    `将继承分类：${inherited.collectionNames.join('、') || '无'}；标签：${inherited.tagNames.join('、') || '无'}。`,
                    `Inherit categories: ${inherited.collectionNames.join(', ') || 'none'}; tags: ${inherited.tagNames.join(', ') || 'none'}.`,
                  )
                : t(
                    '来源没有分类或标签；新笔记仍保留固定版本出处、页码和批注。',
                    'The source has no categories or tags; the new note still keeps the fixed source version, page and comments.',
                  )}
            </p>
          )}
          {(creation.error || creation.state?.error) && (
            <div className="inline-error" role="alert">
              <p>{creation.error || creation.state?.error}</p>
              <button type="button" disabled={busy} onClick={creation.reload}>
                {t('重试读取新建操作', 'Retry note creation state')}
              </button>
            </div>
          )}
          {creation.state?.pending && (
            <p role="status">
              {t(
                '已有新建笔记操作待确认；继续会复用同一次操作，不会重复创建。',
                'A note creation is pending. Continuing reuses the same operation without creating a duplicate.',
              )}
            </p>
          )}
        </div>
      )}
      {error && (
        <div className="inline-error" role="alert">
          <p>{error}</p>
          {!ready && (
            <button disabled={busy} onClick={() => setStorageReload((value) => value + 1)}>
              {t('重试读取操作记录', 'Retry reading saved actions')}
            </button>
          )}
          {intent && (
            <button
              className="text-button"
              disabled={busy}
              onClick={() => onOpenNote(intent.noteId)}
            >
              {t('打开笔记检查', 'Open note to review')}
            </button>
          )}
          {createdNoteId && (
            <button
              className="text-button"
              disabled={busy}
              onClick={() => {
                onSaved();
                onOpenNote(createdNoteId);
              }}
            >
              {t('打开已创建的笔记', 'Open the created note')}
            </button>
          )}
        </div>
      )}
      <footer>
        <button className="secondary-button" disabled={busy} onClick={onClose}>
          {t('关闭', 'Close')}
        </button>
        <button
          className="primary-button"
          disabled={
            busy ||
            !ready ||
            (mode === 'create'
              ? inheritLoading ||
                !!inheritError ||
                !inherited ||
                !noteTitle.trim() ||
                !creation.controller ||
                !!creation.state?.pending
              : !intent && !selected)
          }
          onClick={() => void save()}
        >
          {busy
            ? mode === 'create'
              ? t('正在创建…', 'Creating…')
              : t('正在保存…', 'Saving…')
            : mode === 'create'
              ? t('创建笔记', 'Create note')
              : intent
                ? t('继续确认保存', 'Confirm save')
                : t('加入笔记', 'Add to note')}
        </button>
      </footer>
    </dialog>
  );
}
