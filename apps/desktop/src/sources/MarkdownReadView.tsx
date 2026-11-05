import { useEffect, useState } from 'react';
import type { NoteDocument, SourceVersion, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { StandaloneDocumentContent } from '../rich/DocumentContent';

/**
 * Read-only rendered view of a Markdown original.
 *
 * It reuses the same converter, worker and renderer as "create editable note", so the
 * reader sees exactly what that note would show. Nothing is persisted, and the original
 * file is never modified. The source view stays available for reading raw text and
 * saving excerpts.
 */
export function MarkdownReadView({
  identity,
  version,
}: {
  identity: WorkspaceIdentity;
  version: SourceVersion;
}) {
  const { t, language } = useI18n();
  const [document, setDocument] = useState<NoteDocument | null>(null);
  const [preserved, setPreserved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let active = true;
    let worker: Worker | undefined;
    setDocument(null);
    setPreserved(false);
    setError(null);
    if (version.size > 1024 * 1024) {
      setError(
        t(
          '此原件超过 1 MiB，未渲染。切换到原文视图即可继续阅读。',
          'This original is over 1 MiB so it is not rendered here. Switch to the source view to keep reading.',
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
                '此原件无法渲染，请切换到原文视图阅读。',
                'This original cannot be rendered here. Switch to the source view to read it.',
              ),
            );
          else {
            setDocument(event.data.value.document);
            setPreserved(!!event.data.value.preserved.length);
          }
          worker?.terminate();
        };
        worker.onerror = () => {
          if (active)
            setError(
              t(
                '此原件无法渲染，请切换到原文视图阅读。',
                'This original cannot be rendered here. Switch to the source view to read it.',
              ),
            );
          worker?.terminate();
        };
        worker.postMessage({ text: input.text, format: input.version.format });
      })
      .catch((failure) => {
        if (active) setError(message(failure));
      });
    return () => {
      active = false;
      worker?.terminate();
    };
  }, [
    identity.local_instance_id,
    identity.workspace_id,
    version.id,
    version.blob_hash,
    version.size,
    language,
    attempt,
  ]);
  if (error)
    return (
      <div className="source-notice" role="alert">
        {error}
        {version.size <= 1024 * 1024 && (
          <button onClick={() => setAttempt((value) => value + 1)}>
            {t('重新渲染', 'Render again')}
          </button>
        )}
      </div>
    );
  if (!document)
    return (
      <p className="source-markdown-status" role="status">
        {t('正在渲染 Markdown…', 'Rendering Markdown…')}
      </p>
    );
  return (
    <div className="source-markdown-render">
      {preserved && (
        <p className="source-notice">
          {t(
            '部分语法超出渲染范围，已按 Markdown 原文保留；切到原文视图可查看完整内容。',
            'Some syntax is outside the rendered range and kept as Markdown source. Switch to the source view to see everything.',
          )}
        </p>
      )}
      <div className="note-prose read-document">
        <StandaloneDocumentContent document={document} identity={identity} />
      </div>
    </div>
  );
}
