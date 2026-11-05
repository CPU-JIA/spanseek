import { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useRef } from 'react';
import { ArrowLeft, BookOpen, FolderOpen, Plus, Save, X } from 'lucide-react';
import type {
  LocalDocument,
  PdfReadingView,
  SourceLocator,
  WorkspaceEntry,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { useI18n } from '../i18n';
import { workspaceAvailabilityText, workspaceIsAvailable } from '../welcome/workspace-availability';
import { displayPath } from '../format';
import { Select } from '../ui/Select';
import type { PdfReadingPosition } from './PdfReader';
import './local-document.css';

const PdfReader = lazy(() =>
  import('./PdfReader').then((module) => ({ default: module.PdfReader })),
);
export type LocalReadingPosition = { locator: SourceLocator; view: PdfReadingView };

export function LocalDocumentPage({
  documents,
  active,
  position,
  busy,
  onSelect,
  onPosition,
  onSave,
  onClose,
  onBack,
}: {
  documents: LocalDocument[];
  active: LocalDocument;
  position?: LocalReadingPosition;
  busy: boolean;
  onSelect: (token: string) => void;
  onPosition: (token: string, position: LocalReadingPosition) => void;
  onSave: () => void;
  onClose: () => void;
  onBack: () => void;
}) {
  const { t } = useI18n();
  // This component is keyed by the local file token. Its latest position is an
  // output, not a fresh navigation input on unrelated parent renders.
  const initialPosition = useRef(position).current;
  const positionReader = useRef<(() => PdfReadingPosition | null) | null>(null);
  const positionCallback = useRef(onPosition);
  positionCallback.current = onPosition;
  const capturePosition = useCallback(() => {
    const latest = positionReader.current?.();
    if (latest) positionCallback.current(active.token, latest);
  }, [active.token]);
  // Layout cleanup still has the reader's live DOM and captures scroll that has
  // not dispatched its deferred scroll event when navigation unmounts the page.
  useLayoutEffect(() => capturePosition, [capturePosition]);
  const save = () => {
    capturePosition();
    onSave();
  };
  return (
    <section
      className="local-document-page"
      aria-label={t('本地资料阅读', 'Local document reader')}
    >
      <header className="local-document-header">
        <button
          className="icon-button"
          aria-label={t('返回工作台', 'Back to workspace')}
          onClick={() => {
            capturePosition();
            onBack();
          }}
        >
          <ArrowLeft size={17} />
        </button>
        <div className="local-document-title">
          {documents.length > 1 ? (
            <Select
              aria-label={t('已打开的资料', 'Opened documents')}
              value={active.token}
              onValueChange={(token) => {
                capturePosition();
                onSelect(token);
              }}
              options={documents.map((doc) => ({
                value: doc.token,
                label: doc.display_name,
                description: displayPath(doc.location_hint),
              }))}
            />
          ) : (
            <h1 title={displayPath(active.location_hint) + ' / ' + active.display_name}>
              {active.display_name}
            </h1>
          )}
          <span title={displayPath(active.location_hint)}>
            {t('尚未保存到知识库', 'Not saved to a workspace')}
          </span>
        </div>
        <button
          className="primary-button local-document-save"
          title={t('保存到知识库', 'Save to workspace')}
          aria-label={t('保存到知识库', 'Save to workspace')}
          disabled={busy}
          onClick={save}
        >
          <Save size={16} />
          <span className="local-document-save-label">
            {t('保存到知识库', 'Save to workspace')}
          </span>
        </button>
        <button
          className="icon-button"
          disabled={busy}
          aria-label={t('关闭资料', 'Close document')}
          onClick={onClose}
        >
          <X size={17} />
        </button>
      </header>
      {active.format === 'pdf' && active.resource_url ? (
        <Suspense fallback={<p className="muted">{t('正在打开 PDF…', 'Opening PDF…')}</p>}>
          <PdfReader
            key={active.token}
            versionId={active.token}
            input={{ url: active.resource_url }}
            location={null}
            initialView={initialPosition?.view ?? null}
            positionReader={positionReader}
            onPosition={(locator, view) => onPosition(active.token, { locator, view })}
            onRequireLibrary={save}
          />
        </Suspense>
      ) : (
        <div className="empty-state local-document-empty">
          <BookOpen size={32} strokeWidth={1.5} />
          <h2>{active.display_name}</h2>
          <p>
            {t(
              '将 Markdown、文本、PDF、Word、EPUB、MOBI、表格或演示资料保存到知识库，即可阅读、摘录或转为可编辑笔记。',
              'Save this Markdown, text, PDF, Word, EPUB, MOBI, spreadsheet, or presentation document to a workspace to read, annotate, or turn it into an editable note.',
            )}
          </p>
          <button className="primary-button" disabled={busy} onClick={save}>
            {t('选择知识库并导入', 'Choose workspace and import')}
          </button>
        </div>
      )}
    </section>
  );
}

export function LocalDocumentLibraryDialog({
  name,
  workspaces,
  onChoose,
  onCancel,
}: {
  name: string;
  workspaces: WorkspaceEntry[];
  onChoose: (create: boolean, recent?: WorkspaceIdentity) => void;
  onCancel: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement;
    dialog.current?.showModal();
    return () => {
      dialog.current?.close();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className="local-library-dialog"
      aria-labelledby="local-library-title"
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
    >
      <header>
        <h2 id="local-library-title">{t('选择保存位置', 'Choose a workspace')}</h2>
        <button className="icon-button" aria-label={t('取消', 'Cancel')} onClick={onCancel}>
          <X size={18} />
        </button>
      </header>
      <p>
        {t(
          '保存后可以高亮和添加旁注，当前阅读位置会一并保留。',
          'Save to highlight and add notes. Your reading position will be kept.',
        )}
      </p>
      <strong className="local-library-filename">{name}</strong>
      <div className="local-library-recent">
        {workspaces.map((workspace) => (
          <button
            key={workspace.identity.local_instance_id + '/' + workspace.identity.workspace_id}
            disabled={!workspaceIsAvailable(workspace.availability)}
            title={
              workspaceAvailabilityText(workspace.availability, t) ?? displayPath(workspace.path)
            }
            onClick={() => onChoose(false, workspace.identity)}
          >
            <FolderOpen size={18} />
            <span>
              {workspace.path.split(/[\\/]/).filter(Boolean).at(-1)}
              <small>{displayPath(workspace.path)}</small>
            </span>
          </button>
        ))}
      </div>
      <footer>
        <button className="text-button" onClick={() => onChoose(false)}>
          <FolderOpen size={16} />
          {t('打开知识库', 'Open workspace')}
        </button>
        <button className="primary-button" onClick={() => onChoose(true)}>
          <Plus size={16} />
          {t('新建知识库', 'Create workspace')}
        </button>
      </footer>
    </dialog>
  );
}
