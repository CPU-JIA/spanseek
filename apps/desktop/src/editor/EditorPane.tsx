import { Select } from '../ui/Select';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useI18n } from '../i18n';
import { VoiceNotePlayer } from '../voice/VoiceNotePlayer';
import { KnowledgeTaskButton } from '../inference/knowledge-task-navigation';
import { TaskOriginPanel } from '../inference/TaskOriginPanel';
import {
  ArrowLeft,
  Check,
  ChevronDown,
  Code2,
  CopyPlus,
  Download,
  FileText,
  History,
  LoaderCircle,
  X,
} from 'lucide-react';
import type {
  CitationStyleSummary,
  Note,
  RevisionPage,
  WorkspaceIdentity,
  ExportFormat,
} from '@pkms/contracts';
import { call, message } from '../api';
import type { DraftController } from './draft-controller';
import { NoteEditor } from './NoteEditor';
import { ReadDocument } from './ReadDocument';
import { ConflictDialog } from './ConflictDialog';
import { displayDate } from '../format';
import { NoteOrganization } from '../organization/NoteOrganization';
import { PopupMenu } from '../ui/PopupMenu';
import { AddToReview } from '../review/AddToReview';
import { ReferenceProvider } from './reference-context';
import { ReferencePanel } from './reference-panel';
import type { OpenReference, ReferenceLocation } from './reference-model';
import { documentTextStatistics } from './text-statistics';
import { ObjectTypePicker } from '../organization/ObjectTypeControls';
import {
  objectTypeCatalog,
  readFrontmatterText,
  readObjectType,
  setObjectType,
  type ObjectTypeDefinition,
} from '../organization/object-types';

export function EditorPane({
  controller,
  identity,
  pane,
  onSaved,
  organizationVersion,
  onTitleChange,
  onExport,
  onDuplicate,
  onOpenReference,
  onOpenNote,
  referenceLocation,
  onReturnReference,
  returnAvailable,
  customTypes = [],
}: {
  onOpenReference?: OpenReference;
  onOpenNote?: (id: string) => void;
  referenceLocation?: ReferenceLocation;
  onReturnReference?: () => Promise<void> | void;
  returnAvailable?: boolean;
  customTypes?: readonly ObjectTypeDefinition[];
  onExport: (
    controller: DraftController,
    format: ExportFormat,
    citationStyleId: string | null,
  ) => Promise<void>;
  onDuplicate: (controller: DraftController) => Promise<void>;
  controller: DraftController;
  identity: WorkspaceIdentity;
  pane: string;
  onSaved: () => void;
  organizationVersion: number;
  onTitleChange?: (noteId: string, title: string) => void;
}) {
  const { t, language } = useI18n();
  const labels = {
    saved: t('已保存到知识库', 'Saved to workspace'),
    dirty: t('正在暂存草稿', 'Saving draft…'),
    local: t('本地草稿已暂存', 'Draft saved locally'),
    saving: t('正在保存', 'Saving…'),
    error: t('保存未完成', 'Not saved'),
    conflict: t('存在版本冲突', 'Version conflict'),
  };
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const statistics = useMemo(
    () => documentTextStatistics(state.record.document.content, language),
    [language, state.record.document.content],
  );
  const objectType = readObjectType(state.record.document, customTypes);
  const objectTypeOptions = useMemo(
    () =>
      objectTypeCatalog(customTypes).map((type) => ({
        value: type.id,
        label: t(type.labelZh, type.labelEn),
      })),
    [customTypes, t],
  );
  const objectSummary =
    readFrontmatterText(state.record.document, 'summary') ||
    readFrontmatterText(state.record.document, 'description');
  useEffect(() => {
    onTitleChange?.(state.record.base.id, state.record.title);
  }, [onTitleChange, state.record.base.id, state.record.title]);
  const [exportFormat, setExportFormat] = useState<ExportFormat>('markdown');
  const exportFormatLabel = {
    markdown: 'Markdown',
    html: 'HTML',
    docx: 'DOCX',
    pdf: 'PDF',
  }[exportFormat];
  const [citationStyles, setCitationStyles] = useState<CitationStyleSummary[]>([]);
  const [citationStyleId, setCitationStyleId] = useState('plain');
  const [exporting, setExporting] = useState(false);
  const [duplicating, setDuplicating] = useState(false);
  const [history, setHistory] = useState(false);
  const [comparing, setComparing] = useState(false);
  const compareButton = useRef<HTMLButtonElement>(null);
  const noteTitle = useRef<HTMLInputElement>(null);
  const [revisions, setRevisions] = useState<RevisionPage | null>(null);
  const [historical, setHistorical] = useState<Note | null>(null);
  const [error, setError] = useState<string | null>(null);
  const busy = state.historyBusy;
  const historyBlocked = busy || !!state.record.historyIntent;
  const historyButton = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => () => controller.setComposing(`${pane}-title`, false), [controller, pane]);
  function changeObjectType(id: string) {
    setError(null);
    try {
      controller.update(state.record.title, setObjectType(state.record.document, id));
    } catch (failure) {
      setError(message(failure));
    }
  }
  useEffect(() => {
    if (exportFormat === 'markdown' || citationStyles.length > 0) return;
    let live = true;
    void call({ kind: 'citation_styles' }, 'citation_styles', identity).then(
      (styles) => {
        if (!live) return;
        setCitationStyles(styles);
        setCitationStyleId((current) =>
          styles.some((style) => style.id === current) ? current : 'plain',
        );
      },
      (failure: unknown) => {
        if (live) setError(message(failure));
      },
    );
    return () => {
      live = false;
    };
  }, [citationStyles.length, exportFormat, identity]);
  useEffect(() => {
    if (history) dialog.current?.showModal();
    else dialog.current?.close();
  }, [history]);
  useEffect(() => {
    const target = referenceLocation?.target;
    if (!target || target.target_id !== state.record.base.id) return;
    if (!target.target_revision_id) {
      setHistory(false);
      setHistorical(null);
      return;
    }
    // A pinned source revision opens as immutable history, even if it currently is the head.
    let live = true;
    setError(null);
    setHistorical(null);
    setHistory(false);
    void call(
      {
        kind: 'revision_get',
        payload: { note_id: target.target_id, revision_id: target.target_revision_id },
      },
      'note',
      identity,
    )
      .then((note) => {
        if (live) {
          setHistorical(note);
          setHistory(true);
        }
      })
      .catch((error) => {
        if (live) setError(message(error));
      });
    return () => {
      live = false;
    };
  }, [referenceLocation?.requestId, identity, state.record.base.id]);
  function closeHistory() {
    setHistory(false);
    setHistorical(null);
    historyButton.current?.focus();
  }
  async function openHistory() {
    setError(null);
    try {
      setRevisions(
        await call(
          {
            kind: 'revisions_list',
            payload: { note_id: state.record.base.id, before: null, limit: 50 },
          },
          'revisions',
          identity,
        ),
      );
      setHistory(true);
    } catch (error) {
      setError(message(error));
    }
  }
  function closeComparison(resolved = false) {
    setComparing(false);
    requestAnimationFrame(() => {
      if (resolved) noteTitle.current?.focus();
      else compareButton.current?.focus();
    });
    if (resolved) onSaved();
  }
  function historyOperation(operation?: { restoreRevision: string } | { conflictSuffix: string }) {
    return controller.runHistory(
      (command) => call(command, 'note_commit', identity),
      () =>
        call({ kind: 'notes_get', payload: { note_id: state.record.base.id } }, 'note', identity),
      operation,
    );
  }
  async function runHistory(operation?: { restoreRevision: string } | { conflictSuffix: string }) {
    setError(null);
    try {
      await historyOperation(operation);
      closeHistory();
      onSaved();
    } catch (error) {
      setError(message(error));
    }
  }
  async function recover() {
    if (historical) await runHistory({ restoreRevision: historical.revision_id });
  }
  async function preserveConflict() {
    await runHistory({ conflictSuffix: t('（冲突草稿）', ' (conflict draft)') });
  }
  return (
    <ReferenceProvider
      identity={identity}
      generation={`${organizationVersion}/${state.record.base.revision_id}`}
      onOpenReference={onOpenReference}
    >
      <div className="document-meta">
        <span className={`save-state ${state.status}`} aria-live="polite">
          {state.status === 'saved' ? (
            <Check size={14} />
          ) : state.status === 'saving' ? (
            <LoaderCircle className="spin" size={14} />
          ) : (
            <span className="state-dot" />
          )}
          {labels[state.status]}
        </span>
        <div className="document-actions">
          <KnowledgeTaskButton
            disabled={historyBlocked || state.record.base.trashed}
            prepare={async () => {
              await controller.flush();
              const saved = controller.getSnapshot();
              if (saved.status !== 'saved')
                throw new Error(
                  t('请先完成保存，再使用这篇笔记。', 'Finish saving before using this note.'),
                );
              return [
                {
                  object_id: saved.record.base.id,
                  content_revision: saved.record.base.revision_id,
                },
              ];
            }}
          />
          <span className="document-stat">
            {t(
              `${statistics.words} 词 · ${statistics.characters} 字符`,
              `${statistics.words} ${statistics.words === 1 ? 'word' : 'words'} · ${statistics.characters} ${statistics.characters === 1 ? 'character' : 'characters'}`,
            )}
          </span>
          <AddToReview
            identity={identity}
            target={{
              target_id: state.record.base.id,
              target_revision_id: state.record.base.revision_id,
              target_block_id: null,
            }}
            disabled={historyBlocked || state.record.base.trashed}
            prepareTarget={async () => {
              await controller.flush();
              const saved = controller.getSnapshot();
              if (saved.status !== 'saved')
                throw new Error(
                  t(
                    '请先完成保存，再加入回顾。',
                    'Finish saving before adding this note to review.',
                  ),
                );
              return {
                target_id: saved.record.base.id,
                target_revision_id: saved.record.base.revision_id,
                target_block_id: null,
              };
            }}
          />
          <div className="export-controls">
            <PopupMenu
              label={t('导出格式', 'Export format')}
              className="export-format-trigger"
              placement="bottom-start"
              disabled={exporting}
              trigger={
                <>
                  <span>{exportFormatLabel}</span>
                  <ChevronDown size={13} />
                </>
              }
              items={[
                {
                  id: 'markdown',
                  label: 'Markdown',
                  icon: <FileText />,
                  checked: exportFormat === 'markdown',
                  radio: true,
                  onSelect: () => setExportFormat('markdown'),
                },
                {
                  id: 'html',
                  label: 'HTML',
                  icon: <Code2 />,
                  checked: exportFormat === 'html',
                  radio: true,
                  onSelect: () => setExportFormat('html'),
                },
                {
                  id: 'docx',
                  label: 'DOCX',
                  icon: <FileText />,
                  checked: exportFormat === 'docx',
                  radio: true,
                  onSelect: () => setExportFormat('docx'),
                },
                {
                  id: 'pdf',
                  label: 'PDF',
                  icon: <FileText />,
                  checked: exportFormat === 'pdf',
                  radio: true,
                  onSelect: () => setExportFormat('pdf'),
                },
              ]}
            />
            {exportFormat !== 'markdown' && (
              <div className="choice-field export-citation-style">
                <span className="sr-only">{t('引文样式', 'Citation style')}</span>
                <Select
                  value={citationStyleId}
                  aria-label={t('引文样式', 'Citation style')}
                  disabled={exporting || citationStyles.length === 0}
                  onValueChange={(value) => setCitationStyleId(value)}
                >
                  {citationStyles.map((style) => (
                    <option key={style.id} value={style.id}>
                      {style.id === 'plain' ? t('默认（简式）', 'Default (plain)') : style.title}
                    </option>
                  ))}
                </Select>
              </div>
            )}
            <button
              className="icon-button"
              title={t('导出笔记', 'Export note')}
              aria-label={t('导出笔记', 'Export note')}
              disabled={exporting || historyBlocked}
              onClick={() => {
                setExporting(true);
                setError(null);
                void onExport(
                  controller,
                  exportFormat,
                  exportFormat === 'markdown' ? null : citationStyleId,
                )
                  .catch((error) => setError(message(error)))
                  .finally(() => setExporting(false));
              }}
            >
              {exporting ? <LoaderCircle className="spin" size={17} /> : <Download size={17} />}
            </button>
          </div>
          <button
            className="icon-button"
            aria-label={t('创建副本', 'Duplicate note')}
            title={t('创建副本', 'Duplicate note')}
            disabled={duplicating || historyBlocked || state.record.base.trashed}
            onClick={() => {
              setDuplicating(true);
              setError(null);
              void onDuplicate(controller)
                .catch((error) => setError(message(error)))
                .finally(() => setDuplicating(false));
            }}
          >
            {duplicating ? <LoaderCircle className="spin" size={17} /> : <CopyPlus size={17} />}
          </button>
          <button
            ref={historyButton}
            className="icon-button"
            aria-label={t('版本历史', 'Version history')}
            title={t('版本历史', 'Version history')}
            onClick={() => void openHistory()}
          >
            <History size={18} />
          </button>
        </div>
      </div>
      {state.record.historyIntent && (
        <div className="save-error" role="alert">
          <p>
            {t(
              '版本操作结果尚未确认。草稿和原请求已保留，请确认后继续编辑。',
              'The version operation is not confirmed. Your draft and original request are kept. Confirm it to continue editing.',
            )}
          </p>
          <button disabled={busy} onClick={() => void runHistory()}>
            {t('确认原操作结果', 'Confirm original operation')}
          </button>
        </div>
      )}
      {state.error && !historyBlocked && (
        <div className="save-error" role="alert">
          <p>{state.error}</p>
          {state.status === 'conflict' ? (
            <>
              <button
                ref={compareButton}
                disabled={historyBlocked}
                onClick={() => setComparing(true)}
              >
                {t('比较知识库版本', 'Compare saved and local versions')}
              </button>
              <button disabled={historyBlocked} onClick={() => void preserveConflict()}>
                {t(
                  '保留草稿为新笔记并加载当前版本',
                  'Save draft as a new note and load current version',
                )}
              </button>
            </>
          ) : (
            <button
              onClick={() => void controller.retry().catch((error) => setError(message(error)))}
            >
              {t('重试保存', 'Retry save')}
            </button>
          )}
        </div>
      )}
      {error && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      <div className="document-scroll">
        <div className="document-content">
          <div
            className={`document-cover tone-${objectType.definition.tone}`}
            data-object-type={objectType.rawType ?? 'note'}
          >
            <div className="document-cover-ornament" aria-hidden="true">
              {objectType.definition.labelEn}
            </div>
            <ObjectTypePicker
              state={objectType}
              disabled={historyBlocked || !!objectType.error}
              onChange={changeObjectType}
              customTypes={customTypes}
            />
            {objectType.error && (
              <p className="document-cover-error" role="alert">
                {t(
                  'YAML 属性无法解析。请在正文 frontmatter 中修正后重试。',
                  'YAML properties could not be parsed. Fix them in the frontmatter block and try again.',
                )}
              </p>
            )}
            <p className="document-cover-description">
              {t(objectType.definition.descriptionZh, objectType.definition.descriptionEn)}
            </p>
          </div>
          <input
            ref={noteTitle}
            disabled={historyBlocked}
            className="note-title"
            aria-label={t('笔记标题', 'Note title')}
            placeholder={t('未命名笔记', 'Untitled note')}
            maxLength={300}
            value={state.record.title}
            onChange={(event) =>
              controller.update(event.target.value, controller.getSnapshot().record.document)
            }
            onCompositionStart={() => controller.setComposing(`${pane}-title`, true)}
            onCompositionEnd={() => controller.setComposing(`${pane}-title`, false)}
          />
          {objectSummary && <p className="document-summary">{objectSummary}</p>}
          <VoiceNotePlayer identity={identity} noteId={state.record.base.id} />
          {!historyBlocked && (
            <NoteOrganization
              controller={controller}
              identity={identity}
              onChanged={onSaved}
              generation={organizationVersion}
              createdAt={state.record.base.created_at}
              updatedAt={state.record.base.updated_at}
              words={statistics.words}
              characters={statistics.characters}
              customTypes={customTypes}
              onOpenNote={onOpenNote}
            />
          )}
          {!historyBlocked && (
            <TaskOriginPanel
              key={state.record.base.id}
              identity={identity}
              noteId={state.record.base.id}
              revisionId={state.record.base.revision_id}
            />
          )}
          {historyBlocked ? (
            <ReadDocument
              note={{
                ...state.record.base,
                title: state.record.title,
                document: state.record.document,
              }}
              identity={identity}
              imageImports={state.record.imageImports}
            />
          ) : (
            <NoteEditor
              controller={controller}
              identity={identity}
              pane={pane}
              referenceLocation={
                referenceLocation?.target.target_revision_id ? undefined : referenceLocation
              }
              objectTypeOptions={objectTypeOptions}
            />
          )}
          <ReferencePanel
            noteId={state.record.base.id}
            revisionId={state.record.base.revision_id}
            identity={identity}
            generation={organizationVersion}
            document={state.record.base.document}
          />
        </div>
      </div>
      {comparing && (
        <ConflictDialog
          controller={controller}
          identity={identity}
          pane={pane}
          onClose={() => closeComparison()}
          onResolved={() => closeComparison(true)}
          onPreserve={() =>
            historyOperation({ conflictSuffix: t('（冲突草稿）', ' (conflict draft)') })
          }
        />
      )}
      <dialog
        ref={dialog}
        className="history-dialog"
        onCancel={closeHistory}
        aria-labelledby={`history-title-${pane}`}
      >
        <header>
          <h2 id={`history-title-${pane}`}>{t('版本历史', 'Version history')}</h2>
          <button
            className="icon-button"
            aria-label={t('关闭历史', 'Close history')}
            onClick={closeHistory}
          >
            <X size={18} />
          </button>
        </header>
        <p className="muted">
          {t(
            '恢复会创建新版本，已有历史仍保留。',
            'Restoring creates a new version. Your history is kept.',
          )}
        </p>
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        {state.record.historyIntent && (
          <button disabled={busy} onClick={() => void runHistory()}>
            {t('确认原操作结果', 'Confirm original operation')}
          </button>
        )}
        {historical ? (
          <>
            {onReturnReference &&
              returnAvailable &&
              referenceLocation?.target.target_revision_id === historical.revision_id && (
                <button
                  type="button"
                  className="secondary-button reference-return"
                  onClick={() => {
                    closeHistory();
                    requestAnimationFrame(
                      () =>
                        void Promise.resolve(onReturnReference()).catch((cause) =>
                          setError(message(cause)),
                        ),
                    );
                  }}
                >
                  <ArrowLeft size={16} />
                  {t('返回来处', 'Return to origin')}
                </button>
              )}
            <button
              className="text-button"
              onClick={() => {
                setHistorical(null);
                void openHistory();
              }}
            >
              <ArrowLeft size={16} />
              {t('返回历史列表', 'Back to versions')}
            </button>
            <h3>{historical.title}</h3>
            <p className="muted">
              {t(
                `固定第 ${historical.revision} 版 · 只读历史`,
                `Pinned version ${historical.revision} · Read-only history`,
              )}
            </p>
            <ReadDocument
              note={historical}
              identity={identity}
              referenceLocation={
                referenceLocation?.target.target_revision_id === historical.revision_id
                  ? referenceLocation
                  : undefined
              }
            />
            <ReferencePanel
              noteId={historical.id}
              revisionId={historical.revision_id}
              identity={identity}
              generation={organizationVersion}
              document={historical.document}
            />
            <button
              className="primary-button"
              disabled={historyBlocked}
              onClick={() => void recover()}
            >
              {t('恢复这一版', 'Restore this version')}
            </button>
          </>
        ) : (
          <div className="history-list">
            {revisions?.items.map((revision) => (
              <button
                key={revision.revision_id}
                onClick={() =>
                  void call(
                    {
                      kind: 'revision_get',
                      payload: { note_id: state.record.base.id, revision_id: revision.revision_id },
                    },
                    'note',
                    identity,
                  )
                    .then(setHistorical)
                    .catch((error) => setError(message(error)))
                }
              >
                <strong>{revision.title}</strong>
                <span>
                  {t(`第 ${revision.revision} 版`, `Version ${revision.revision}`)} ·{' '}
                  {displayDate(revision.created_at, language)}
                </span>
              </button>
            ))}
            {revisions?.next_before && (
              <button
                onClick={() =>
                  void call(
                    {
                      kind: 'revisions_list',
                      payload: {
                        note_id: state.record.base.id,
                        before: revisions.next_before,
                        limit: 50,
                      },
                    },
                    'revisions',
                    identity,
                  )
                    .then((next) =>
                      setRevisions((current) => ({
                        items: [...current!.items, ...next.items],
                        next_before: next.next_before,
                      })),
                    )
                    .catch((error) => setError(message(error)))
                }
              >
                {t('更早的版本', 'Earlier versions')}
              </button>
            )}
          </div>
        )}
      </dialog>
    </ReferenceProvider>
  );
}
