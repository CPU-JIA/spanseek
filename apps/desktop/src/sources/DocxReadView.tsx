import { useEffect, useRef, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SourceResourceTicket, SourceVersion, WorkspaceIdentity } from '@pkms/contracts';
import { openExternalUrl } from '../api';
import { useI18n } from '../i18n';
import { PdfReader } from './PdfReader';
import {
  MAX_DOCX_PREVIEW_BYTES,
  docxPreviewObjectUrls,
  isSafeDocxHref,
  readSourceResourceBytes,
  sanitizeDocxPreview,
} from './source-resource-bytes';

type Status = 'loading' | 'ready' | 'error';
type Renderer = 'checking' | 'word' | 'builtin' | 'legacy';
type FallbackReason = 'unavailable' | 'failed';

function previewFailure(error: unknown, text: (zh: string, en: string) => string): string {
  const code = error instanceof Error ? error.message : '';
  if (code === 'source_resource_too_large')
    return text(
      '此 DOCX 超过 64 MiB，未在阅读器内做版式预览。请切到原文阅读，或用 Word 打开原件。',
      'This DOCX is over 64 MiB, so it is not laid out here. Switch to the source view or open the original in Word.',
    );
  if (code === 'source_resource_range')
    return text(
      'DOCX 原件读取不完整，未做版式预览。请重试，或切到原文阅读。',
      'The DOCX original could not be read completely, so it was not laid out. Retry or switch to the source view.',
    );
  if (code === 'docx_preview_empty')
    return text(
      '此 DOCX 没有可显示的 Word 版式内容。请切到原文阅读，或用 Word 打开原件。',
      'This DOCX produced no Word layout to display. Switch to the source view or open the original in Word.',
    );
  return text(
    '此 DOCX 暂无法按 Word 版式渲染。请重试，或切到原文阅读。',
    'This DOCX cannot be laid out here right now. Retry or switch to the source view.',
  );
}

/**
 * Word-like read-only view for the immutable DOCX or legacy DOC original.
 *
 * A local Microsoft Word instance, when present, is the source of truth for
 * pagination and table splitting: it exports the document to a cached PDF and
 * the existing PDF reader displays it. DOCX machines without Word use the
 * bundled semantic renderer; legacy DOC stays readable through the exact-text
 * source view because no safe built-in binary Word layout engine is bundled.
 *
 * The existing source view remains the exact-text surface used for anchors and
 * excerpts in both cases.
 */
export function DocxReadView({
  identity,
  version,
  onFallback,
}: {
  identity: WorkspaceIdentity;
  version: SourceVersion;
  onFallback: () => void;
}) {
  const { t, language } = useI18n();
  const legacy = version.format === 'doc';
  const body = useRef<HTMLDivElement>(null);
  const styles = useRef<HTMLDivElement>(null);
  const [renderer, setRenderer] = useState<Renderer>('checking');
  const [wordTicket, setWordTicket] = useState<SourceResourceTicket | null>(null);
  const [fallbackReason, setFallbackReason] = useState<FallbackReason | null>(null);
  const [wordAttempt, setWordAttempt] = useState(0);
  const [status, setStatus] = useState<Status>('loading');
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  // Ask the native side for Word's exact PDF first. `null` means Word is not
  // registered; an error means this particular export failed. Both keep the
  // document readable through the built-in semantic renderer.
  useEffect(() => {
    let cancelled = false;
    setRenderer('checking');
    setWordTicket(null);
    setFallbackReason(null);
    setStatus('loading');
    setError(null);
    void invoke<SourceResourceTicket | null>('issue_docx_word_preview', {
      identity,
      sourceVersionId: version.id,
    })
      .then((ticket) => {
        if (cancelled) {
          if (ticket)
            void invoke('release_source_resource', { resourceUrl: ticket.url }).catch(() => {});
          return;
        }
        if (ticket) {
          setWordTicket(ticket);
          setRenderer('word');
          setStatus('ready');
        } else {
          setFallbackReason('unavailable');
          setRenderer(legacy ? 'legacy' : 'builtin');
        }
      })
      .catch(() => {
        if (cancelled) return;
        setFallbackReason('failed');
        setRenderer(legacy ? 'legacy' : 'builtin');
      });
    return () => {
      cancelled = true;
    };
  }, [
    identity.local_instance_id,
    identity.workspace_id,
    version.id,
    version.size,
    version.format,
    legacy,
    wordAttempt,
  ]);

  // The PDF reader only borrows the native resource URL. Release it when the
  // Word view unmounts or switches back to the built-in renderer.
  useEffect(() => {
    if (!wordTicket) return;
    const url = wordTicket.url;
    return () => {
      void invoke('release_source_resource', { resourceUrl: url }).catch(() => {});
    };
  }, [wordTicket]);

  useEffect(() => {
    if (renderer !== 'builtin') return;
    let cancelled = false;
    let objectUrls: string[] = [];
    const controller = new AbortController();
    const clear = () => {
      body.current?.replaceChildren();
      styles.current?.replaceChildren();
    };
    setStatus('loading');
    setError(null);
    setActionError(null);
    clear();
    void (async () => {
      if (version.size > MAX_DOCX_PREVIEW_BYTES) throw new Error('source_resource_too_large');
      const bytes = await readSourceResourceBytes(
        identity,
        version.id,
        MAX_DOCX_PREVIEW_BYTES,
        controller.signal,
      );
      if (cancelled || !body.current || !styles.current) return;
      const { renderAsync } = await import('docx-preview');
      await renderAsync(bytes, body.current, styles.current, {
        className: 'docx',
        inWrapper: true,
        breakPages: true,
        ignoreWidth: false,
        ignoreHeight: false,
        ignoreFonts: false,
        // docx-preview experimental tab stops expand some Word forms to 236pt,
        // forcing a single field value onto a new line. The default tab spacing
        // keeps the stored line closer to Word without changing the document.
        experimental: false,
        trimXmlDeclaration: true,
        ignoreLastRenderedPageBreak: false,
        renderHeaders: true,
        renderFooters: true,
        renderFootnotes: true,
        renderEndnotes: true,
        useBase64URL: false,
        renderChanges: false,
        renderComments: false,
        // altChunks are arbitrary HTML and can execute script; the source text
        // remains available in the other view, so never trust this part.
        renderAltChunks: false,
      });
      const renderedUrls =
        body.current && styles.current ? docxPreviewObjectUrls([body.current, styles.current]) : [];
      if (cancelled || !body.current || !styles.current) {
        for (const url of renderedUrls) URL.revokeObjectURL(url);
        return;
      }
      sanitizeDocxPreview(body.current);
      if (!body.current.querySelector('.docx-wrapper') && !body.current.querySelector('.docx'))
        throw new Error('docx_preview_empty');
      objectUrls = renderedUrls;
      setStatus('ready');
    })().catch((failure) => {
      if (cancelled || (failure as { name?: string } | null)?.name === 'AbortError') return;
      const partialUrls =
        body.current && styles.current ? docxPreviewObjectUrls([body.current, styles.current]) : [];
      for (const url of partialUrls) URL.revokeObjectURL(url);
      clear();
      setError(previewFailure(failure, t));
      setStatus('error');
    });
    return () => {
      cancelled = true;
      controller.abort();
      for (const url of objectUrls) URL.revokeObjectURL(url);
    };
  }, [
    renderer,
    identity.local_instance_id,
    identity.workspace_id,
    version.id,
    version.size,
    attempt,
    language,
  ]);

  function followLink(event: ReactMouseEvent<HTMLDivElement>) {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const anchor = target.closest('a');
    if (!(anchor instanceof HTMLAnchorElement)) return;
    const href = anchor.getAttribute('href');
    if (!href) return;
    event.preventDefault();
    setActionError(null);
    if (href.startsWith('#')) {
      try {
        const id = decodeURIComponent(href.slice(1));
        body.current?.querySelector(`[id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'start' });
      } catch {
        // A malformed internal anchor is not worth replacing the document with
        // an error; the reader still sees exactly what Word stored.
      }
      return;
    }
    if (!isSafeDocxHref(href)) return;
    void openExternalUrl(href).catch(() =>
      setActionError(
        t(
          '无法打开此链接，请检查地址和系统默认浏览器。',
          'Could not open this link. Check the URL and default browser.',
        ),
      ),
    );
  }

  const hint =
    renderer === 'word'
      ? t(
          'Microsoft Word 精确版式（只读）；需要按原文摘录时切换到「原文」。',
          'Exact Microsoft Word layout (read-only). Switch to Source for exact-text excerpts.',
        )
      : renderer === 'checking'
        ? t(
            '正在检测本机 Microsoft Word，准备精确版式…',
            'Checking for local Microsoft Word to prepare the exact layout…',
          )
        : renderer === 'legacy'
          ? t(
              '旧版 DOC 需要本机 Microsoft Word 才能显示精确版式；正文仍可按原文阅读和摘录。',
              'Legacy DOC needs local Microsoft Word for its exact layout; the source text remains available for reading and excerpts.',
            )
          : t(
              '内置 Word 版式（只读）；复杂分页可能与 Word 不同，需要按原文摘录时切换到「原文」。',
              'Built-in Word layout (read-only). Complex pagination may differ from Word; switch to Source for excerpts.',
            );

  return (
    <div
      className="source-docx-render"
      data-docx-renderer={renderer}
      data-docx-fallback={fallbackReason ?? undefined}
    >
      <p className="source-docx-hint">{hint}</p>
      {renderer === 'checking' && (
        <p className="source-docx-status" role="status">
          {t('正在准备 Word 版式…', 'Preparing the Word layout…')}
        </p>
      )}
      {renderer === 'word' && wordTicket && (
        <div className="source-docx-word" data-docx-renderer="word">
          <PdfReader identity={identity} versionId={version.id} input={{ url: wordTicket.url }} />
        </div>
      )}
      {renderer === 'legacy' && (
        <div className="source-docx-fallback source-docx-legacy" role="status">
          <span>
            {fallbackReason === 'failed'
              ? t(
                  '本机 Word 无法完成这次旧版 DOC 渲染。请重试，或直接阅读原文。',
                  'Local Word could not finish this legacy DOC layout. Retry or read the source text.',
                )
              : t(
                  '本机未检测到 Microsoft Word，无法显示旧版 DOC 的精确版式。原始正文已可阅读和摘录。',
                  'Microsoft Word was not found, so the exact legacy DOC layout is unavailable. The original text is still ready to read and excerpt.',
                )}
          </span>
          <button type="button" onClick={onFallback}>
            {t('切到原文', 'Switch to source')}
          </button>
          <button type="button" onClick={() => setWordAttempt((value) => value + 1)}>
            {t('重新检测', 'Check again')}
          </button>
        </div>
      )}
      {renderer === 'builtin' && (
        <>
          {fallbackReason && status === 'ready' && (
            <div className="source-docx-fallback" role="status">
              <span>
                {fallbackReason === 'unavailable'
                  ? t(
                      '本机未检测到 Microsoft Word，已使用内置版式。',
                      'Microsoft Word was not found, so the built-in layout is shown.',
                    )
                  : t(
                      '本机 Word 无法完成这次渲染，已使用内置版式。',
                      'Local Word could not finish this layout, so the built-in layout is shown.',
                    )}
              </span>
              <button type="button" onClick={() => setWordAttempt((value) => value + 1)}>
                {t('再次检测', 'Check again')}
              </button>
            </div>
          )}
          {status === 'loading' && (
            <p className="source-docx-status" role="status">
              {t('正在按 Word 版式渲染…', 'Rendering the Word layout…')}
            </p>
          )}
          {error && (
            <div className="source-notice" role="alert">
              {error}
              <button onClick={() => setAttempt((value) => value + 1)}>
                {t('重新渲染', 'Render again')}
              </button>
              <button onClick={onFallback}>{t('切到原文', 'Switch to source')}</button>
            </div>
          )}
          {actionError && (
            <p className="source-notice" role="status">
              {actionError}
            </p>
          )}
          <div ref={styles} hidden />
          <div
            className={`source-docx-scroll ${status === 'ready' ? 'is-ready' : ''}`}
            onClick={followLink}
          >
            <div ref={body} className="source-docx-pages" />
          </div>
        </>
      )}
    </div>
  );
}
