import { useEffect, useRef, useState } from 'react';
import { FilePlus2 } from 'lucide-react';
import type {
  NoteDocument,
  SourceNoteInput,
  SourceVersion,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useCreateNote } from '../organization/useCreateNote';
import './source-note-import.css';

type Prepared = { input: SourceNoteInput; document: NoteDocument; preserved: boolean };

export function SourceNoteImport({
  identity,
  version,
  disabled,
  onChanged,
}: {
  identity: WorkspaceIdentity;
  version: SourceVersion;
  disabled: boolean;
  onChanged?: () => void;
}) {
  const { t, language } = useI18n();
  const creation = useCreateNote(identity);
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState(version.original_name.replace(/\.(md|markdown|txt)$/i, ''));
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const trigger = useRef<HTMLButtonElement>(null);
  const titleInput = useRef<HTMLInputElement>(null);
  const live = useRef(true);
  useEffect(() => {
    if (open) titleInput.current?.focus();
  }, [open]);
  function close() {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  }
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    if (!open) return;
    let active = true;
    let worker: Worker | undefined;
    setPrepared(null);
    setError(null);
    if (version.size > 1024 * 1024) {
      setError(
        t(
          '此原件超过 1 MiB，可继续阅读和摘录，不能整份转为笔记。',
          'This original exceeds 1 MiB. Keep reading or highlight a smaller excerpt.',
        ),
      );
      return;
    }
    void call(
      { kind: 'sources_note_input', payload: { source_version_id: version.id } },
      'source_note_input',
      identity,
    )
      .then((input) => {
        if (!active) return;
        if (input.version.id !== version.id || input.version.blob_hash !== version.blob_hash)
          throw new Error(
            t(
              '原件版本不匹配，请重新打开资料。',
              'Original version does not match. Reopen the document.',
            ),
          );
        worker = new Worker(new URL('./source-note-worker.ts', import.meta.url), {
          type: 'module',
        });
        worker.onmessage = (event) => {
          if (!active) return;
          if (event.data.error)
            setError(
              t(
                '正文转换失败。原件已保留，可继续作为资料阅读。',
                'Conversion failed. The original remains available in the library.',
              ),
            );
          else
            setPrepared({
              input,
              document: event.data.value.document,
              preserved: !!event.data.value.preserved.length,
            });
          worker?.terminate();
        };
        worker.onerror = () => {
          if (active) setError(t('正文转换失败，请重试。', 'Conversion failed. Try again.'));
          worker?.terminate();
        };
        worker.postMessage({ text: input.text, format: input.version.format });
      })
      .catch((error) => {
        if (active) setError(message(error));
      });
    return () => {
      active = false;
      worker?.terminate();
    };
  }, [
    open,
    attempt,
    identity.local_instance_id,
    identity.workspace_id,
    version.id,
    version.blob_hash,
    version.size,
    language,
  ]);

  async function create() {
    if (!prepared || !creation.controller || creation.state?.pending || disabled) return;
    const result = await creation.controller.create({
      kind: 'sources_create_note',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          source_version_id: prepared.input.version.id,
          source_blob_hash: prepared.input.version.blob_hash,
          title,
          document_schema: 3,
          document: prepared.document,
        },
      },
    });
    if (result && live.current) {
      close();
      onChanged?.();
    }
  }

  return (
    <div className="source-note-import source-notice">
      <button
        ref={trigger}
        type="button"
        title={t('创建可编辑笔记', 'Create editable note')}
        disabled={disabled || creation.state?.busy}
        aria-expanded={open}
        onClick={() => (open ? close() : setOpen(true))}
      >
        <FilePlus2 size={15} aria-hidden="true" />
        <span className="source-note-import-label">
          {t('创建可编辑笔记', 'Create editable note')}
        </span>
      </button>
      {open && (
        <div>
          <p>
            {t(
              '从当前原件版本创建一篇独立笔记，并附原件引用。支持不超过 1 MiB 的 UTF-8 TXT / Markdown；编辑不会写回原文件。',
              'Create an independent note with a reference to this original version. Supports UTF-8 TXT / Markdown up to 1 MiB; edits never overwrite the original file.',
            )}
          </p>
          {!prepared && !error && (
            <p role="status">{t('正在读取并转换原件…', 'Reading and converting the original…')}</p>
          )}
          {prepared?.preserved && (
            <p role="status">
              {t(
                '部分格式、图片或链接超出编辑器支持范围，对应内容将按 Markdown 原文保留，不会自动下载图片。',
                'Unsupported formatting, images or links are preserved as Markdown source. Images are not downloaded.',
              )}
            </p>
          )}
          <label>
            {t('笔记标题', 'Note title')}
            <input
              ref={titleInput}
              aria-label={t('导入笔记标题', 'Imported note title')}
              value={title}
              maxLength={300}
              onChange={(event) => setTitle(event.target.value)}
            />
          </label>
          {(error || creation.error || creation.state?.error) && (
            <p role="alert">{error || creation.error || creation.state?.error}</p>
          )}
          {error && (
            <button onClick={() => setAttempt((value) => value + 1)}>
              {t('重新读取', 'Read again')}
            </button>
          )}
          {creation.error && (
            <button onClick={creation.reload}>
              {t('重试本机操作记录', 'Retry local operation storage')}
            </button>
          )}
          {creation.state?.pending && (
            <p role="status">
              {t(
                '已有新建笔记操作待完成，请使用页面上方的提示核对或打开原笔记。',
                'A note creation is pending. Use the notice above to confirm or open that note.',
              )}
            </p>
          )}
          <div className="source-tools">
            <button
              disabled={
                !prepared ||
                !title.trim() ||
                disabled ||
                !creation.controller ||
                !!creation.state?.pending ||
                creation.state?.busy
              }
              onClick={() => void create()}
            >
              {t('确认创建笔记', 'Create note')}
            </button>
            <button type="button" disabled={creation.state?.busy} onClick={close}>
              {t('取消', 'Cancel')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
