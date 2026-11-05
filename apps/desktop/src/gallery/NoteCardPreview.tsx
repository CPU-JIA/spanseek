import { useEffect, useRef, useState } from 'react';
import type { NoteSummary, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';
import { PreviewDocumentView } from './PreviewDocumentView';
import { PreviewStore, previewKey, previewRequest, type PreviewResult } from './preview-store';
import { previewExcerpt } from './preview-document';
import './gallery-preview.css';

const previews = new PreviewStore((request) =>
  call(
    { kind: 'revision_get', payload: { note_id: request.noteId, revision_id: request.revisionId } },
    'note',
    request.identity,
  ),
);
export type GalleryPreviewStatus = 'idle' | 'loading' | 'ready' | 'error';

export function NoteCardPreview({
  identity,
  note,
  root,
  retryToken = 0,
  onPreviewState,
}: {
  identity: WorkspaceIdentity;
  note: NoteSummary;
  root: HTMLElement | null;
  retryToken?: number;
  onPreviewState?: (state: GalleryPreviewStatus) => void;
}) {
  const { t } = useI18n();
  const host = useRef<HTMLSpanElement>(null);
  const request = previewRequest(identity, note),
    key = previewKey(request);
  const [visibility, setVisibility] = useState<{
    key: string;
    root: HTMLElement;
    near: boolean;
  } | null>(null);
  const near = visibility?.key === key && visibility.root === root && visibility.near;
  const [result, setResult] = useState<{ key: string; value: PreviewResult } | null>(null);
  const [imageError, setImageError] = useState<string | null>(null);
  const remembered = useRef<{ key: string; retry: number; error: PreviewResult | null }>({
    key,
    retry: retryToken,
    error: null,
  });
  const report = useRef(onPreviewState);
  report.current = onPreviewState;

  useEffect(() => {
    if (!root || !host.current) return;
    let live = true;
    const observer = new IntersectionObserver(
      (entries) => {
        if (live) setVisibility({ key, root, near: entries.some((entry) => entry.isIntersecting) });
      },
      { root, rootMargin: '220px 0px', threshold: 0 },
    );
    observer.observe(host.current);
    return () => {
      live = false;
      observer.disconnect();
    };
  }, [key, root]);

  useEffect(() => {
    const previous = remembered.current;
    const retry = previous.key === key && previous.retry !== retryToken;
    if (previous.key !== key || retry) {
      remembered.current = { key, retry: retryToken, error: null };
      setImageError(null);
    }
    if (retry) previews.retry(request);
    if (!near) {
      setResult(null);
      return;
    }
    if (remembered.current.error) {
      setResult({ key, value: remembered.current.error });
      return;
    }
    let live = true;
    const unsubscribe = previews.subscribe(request, (value) => {
      if (!live) return;
      if (value.status === 'error') remembered.current.error = value;
      setResult({ key, value });
    });
    return () => {
      live = false;
      unsubscribe();
    };
  }, [key, near, retryToken]);

  const current = near && result?.key === key ? result.value : null;
  const status: GalleryPreviewStatus =
    current?.status === 'ready' && imageError === key
      ? 'error'
      : (current?.status ?? (near ? 'loading' : 'idle'));
  useEffect(() => {
    report.current?.(status);
  }, [status, key]);
  return (
    <span ref={host} className="gallery-preview" data-preview-state={status}>
      {current?.status === 'ready' ? (
        <PreviewDocumentView
          key={`${key}/${retryToken}`}
          identity={identity}
          revision={note.revision_id}
          document={current.document}
          title={note.title}
          imageFailed={imageError === key}
          onImageError={() => setImageError(key)}
        />
      ) : (
        <span className="gallery-preview-canvas gallery-preview-placeholder" aria-hidden="true">
          <h1 className="gallery-preview-title">
            {note.title || t('未命名笔记', 'Untitled note')}
          </h1>
          <p>{previewExcerpt(note.preview, note.title)}</p>
        </span>
      )}
      {current?.status === 'error' ? (
        <span
          className="gallery-preview-status gallery-preview-error"
          role="status"
          title={current.error}
        >
          {t('预览读取失败，请重试', 'Preview unavailable. Retry to load it.')}
        </span>
      ) : current?.status === 'ready' && imageError === key ? (
        <span className="gallery-preview-status gallery-preview-error" role="status">
          {t(
            '缩略图读取失败，正文仍可打开',
            'Thumbnail unavailable. The note can still be opened.',
          )}
        </span>
      ) : (near && !current) || current?.status === 'loading' ? (
        <span className="gallery-preview-status" role="status" aria-live="off">
          {t('正在读取预览…', 'Loading preview…')}
        </span>
      ) : null}
    </span>
  );
}
