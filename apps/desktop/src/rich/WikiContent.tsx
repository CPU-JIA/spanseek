import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { NodeAttributes, SourceTextPage, WorkspaceIdentity } from '@pkms/contracts';
import { ArrowUpRight, FileSymlink, Link2Off, Pin, X } from 'lucide-react';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { resolveReference, useReferenceContext } from '../editor/reference-context';
import { ReferenceChip } from '../editor/reference-chip';
import { useRichDocument } from './document-context';
import { opensReference, type ReferenceInteraction } from '../editor/reference-model';
import {
  boundedEmbed,
  MAX_EMBED_DEPTH,
  resolveWiki,
  type WikiPort,
  type WikiResult,
} from './wiki-model';
import { DocumentImage } from './DocumentImage';
import './rich-content.css';

const PdfReader = lazy(() =>
  import('../sources/PdfReader').then((module) => ({ default: module.PdfReader })),
);
const noPosition = () => undefined;
const emptyVisited: ReadonlySet<string> = new Set();
const inFlight = new Map<string, Promise<WikiResult>>();

type WikiContentProps = {
  attrs: NodeAttributes;
  identity: WorkspaceIdentity;
  embedded: boolean;
  depth?: number;
  visited?: ReadonlySet<string>;
  interaction?: ReferenceInteraction;
  onSelect?: () => void;
};

export function WikiContent(props: WikiContentProps) {
  // Picker aliases already bind every part of the target by ID. Share the full
  // reference detail/recovery UI; textual wiki anchors still need wiki resolution.
  if (!props.embedded && props.attrs.target_id && props.attrs.wiki_target === props.attrs.target_id)
    return <ReferenceChip {...props} displayLabel={props.attrs.label ?? undefined} />;
  return <ResolvedWikiContent {...props} />;
}

function ResolvedWikiContent({
  attrs,
  identity,
  embedded,
  depth,
  visited,
  interaction = 'read',
  onSelect,
}: WikiContentProps) {
  const { t } = useI18n();
  const reference = useReferenceContext();
  const scope = useRichDocument();
  const currentDepth = depth ?? scope?.depth ?? 0,
    trail = visited ?? scope?.visited ?? emptyVisited;
  const port = useMemo<WikiPort>(
    () => ({
      notes: (query, cursor) =>
        call(
          { kind: 'notes_list', payload: { query, cursor, limit: 100, trashed: false } },
          'notes',
          identity,
        ),
      sources: (query, cursor) =>
        call(
          {
            kind: 'sources_list',
            payload: { query, cursor, filter: null, limit: 100, trashed: false },
          },
          'sources',
          identity,
        ),
      note: (id, revision) =>
        call(
          revision
            ? { kind: 'revision_get', payload: { note_id: id, revision_id: revision } }
            : { kind: 'notes_get', payload: { note_id: id } },
          'note',
          identity,
        ),
      source: (id) => call({ kind: 'sources_get', payload: { source_id: id } }, 'source', identity),
      version: (id) =>
        call(
          { kind: 'sources_version', payload: { source_version_id: id } },
          'source_version',
          identity,
        ),
      resolve: (target, options) => resolveReference(identity, target, options),
    }),
    [identity.local_instance_id, identity.workspace_id],
  );
  const key = JSON.stringify([
    identity,
    attrs,
    scope?.note?.id,
    scope?.note?.title,
    scope?.note?.revision_id,
    reference?.generation,
  ]);
  const activeKey = useRef(key);
  const navigationSerial = useRef(0);
  activeKey.current = key;
  useEffect(() => {
    navigationSerial.current++;
    activeKey.current = key;
    return () => {
      if (activeKey.current === key) activeKey.current = '';
    };
  }, [key]);
  const [result, setResult] = useState<{ key: string; value?: WikiResult; error?: string } | null>(
    null,
  );
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [actionError, setActionError] = useState<{ key: string; message: string } | null>(null);
  const host = useRef<HTMLSpanElement>(null);
  const details = useRef<HTMLDialogElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [near, setNear] = useState(false);
  useEffect(() => {
    if (!host.current) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setNear(true);
          observer.disconnect();
        }
      },
      { rootMargin: '200px' },
    );
    observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (!near || (embedded && currentDepth >= MAX_EMBED_DEPTH)) return;
    let live = true;
    let request = inFlight.get(key);
    if (!request) {
      request = resolveWiki(attrs, scope?.note ?? null, port);
      inFlight.set(key, request);
      void request
        .finally(() => {
          if (inFlight.get(key) === request) inFlight.delete(key);
        })
        .catch(() => undefined);
    }
    void request
      .then((value) => {
        if (live) setResult({ key, value });
      })
      .catch((cause) => {
        if (live) setResult({ key, error: message(cause) });
      });
    return () => {
      live = false;
    };
  }, [key, near, currentDepth, embedded, port]);
  const active = result?.key === key ? result : null,
    resolved = active?.value;
  const available = resolved?.state === 'available';
  const recursive = available && resolved.kind === 'note' && trail.has(resolved.note.id);
  const failure =
    (actionError?.key === key ? actionError.message : null) ||
    active?.error ||
    (resolved && !available ? resolved.message : null) ||
    (embedded && currentDepth >= MAX_EMBED_DEPTH
      ? t('嵌入已达到层级上限。', 'The embed depth limit has been reached.')
      : null) ||
    (embedded && recursive
      ? t('检测到循环嵌入，已停止展开。', 'Circular embed detected; expansion stopped.')
      : null);
  const label =
    attrs.label ||
    (available
      ? resolved.kind === 'note'
        ? resolved.note.title
        : resolved.kind === 'source'
          ? resolved.source.display_title
          : resolved.alt
      : attrs.wiki_target) ||
    t('内部链接', 'Internal link');
  function closeDetails() {
    details.current?.close();
    trigger.current?.focus({ preventScroll: true });
  }
  function showDetails() {
    if (!details.current?.open) details.current?.showModal();
  }
  async function open() {
    if (busyKey === key) return;
    const serial = ++navigationSerial.current;
    setBusyKey(key);
    setActionError(null);
    try {
      // Following a link must never act on a remembered answer.
      const latest = await resolveWiki(attrs, scope?.note ?? null, port, { fresh: true });
      if (activeKey.current !== key || serial !== navigationSerial.current) return;
      setResult({ key, value: latest });
      if (latest.state !== 'available') {
        setActionError({ key, message: latest.message });
        if (interaction === 'edit') showDetails();
        return;
      }
      if (latest.kind === 'image') {
        setActionError({
          key,
          message: t('此图片已显示在当前笔记中。', 'This image is displayed in the current note.'),
        });
        return;
      }
      if (!reference?.onOpenReference) {
        setActionError({
          key,
          message: t(
            '当前阅读区域不能跳转，请在工作台打开笔记。',
            'Open the note in the workspace to follow this link.',
          ),
        });
        return;
      }
      details.current?.close();
      await reference.onOpenReference(latest.target);
    } catch (cause) {
      if (activeKey.current === key && serial === navigationSerial.current)
        setActionError({ key, message: message(cause) });
    } finally {
      if (activeKey.current === key && serial === navigationSerial.current) setBusyKey(null);
    }
  }
  let content = null;
  if (embedded && available && !failure) {
    if (resolved.kind === 'note') {
      const preview = boundedEmbed(resolved.blocks);
      const nextTrail = new Set(trail);
      nextTrail.add(resolved.note.id);
      content = scope?.renderNote ? (
        <>
          <div className="rich-embedded-document">
            {scope.renderNote(
              {
                ...resolved.note,
                document: { ...resolved.note.document, content: preview.content },
              },
              currentDepth + 1,
              nextTrail,
            )}
          </div>
          {preview.truncated && (
            <p className="rich-caption">
              {t(
                '嵌入已截取，打开目标查看完整正文。',
                'Embed preview truncated. Open the target for the complete document.',
              )}
            </p>
          )}
        </>
      ) : (
        <p className="rich-caption">
          {t('打开目标笔记查看完整正文。', 'Open the target note to read its content.')}
        </p>
      );
    } else if (resolved.kind === 'image') {
      content = (
        <DocumentImage
          identity={identity}
          revision={resolved.note.revision_id}
          hash={resolved.hash}
          alt={resolved.alt}
          importCommand={scope?.imageImports?.[resolved.hash]}
        />
      );
    } else
      content = (
        <SourceEmbed key={`${key}/${resolved.version.id}`} result={resolved} identity={identity} />
      );
  }
  const button = (
    <button
      ref={trigger}
      type="button"
      className={`reference-chip rich-wiki-link${failure ? ' is-unavailable' : ''}`}
      data-reference-occurrence={attrs.occurrence_id ?? undefined}
      onClick={(event) => {
        if (opensReference(interaction, event)) void open();
        else onSelect?.();
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        showDetails();
      }}
      onKeyDown={(event) => {
        if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
          event.preventDefault();
          event.stopPropagation();
          showDetails();
        }
      }}
      disabled={busyKey === key}
      title={
        failure ||
        `${attrs.wiki_target ?? label}${interaction === 'edit' ? ` · ${t('Ctrl+点击打开；右键查看详情', 'Ctrl+click to open; right-click for details')}` : ''}`
      }
    >
      {failure ? (
        <Link2Off size={13} aria-hidden="true" />
      ) : (
        <FileSymlink size={13} aria-hidden="true" />
      )}
      <span>{label}</span>
      {attrs.target_revision_id && <Pin size={11} aria-label={t('固定版本', 'Pinned version')} />}
    </button>
  );
  return (
    <span
      ref={host}
      className={embedded ? 'rich-wiki-embed' : 'rich-wiki'}
      style={
        embedded
          ? {
              width: attrs.embed_width ? `${Math.min(attrs.embed_width, 2400)}px` : undefined,
              maxHeight: attrs.embed_height ? `${Math.min(attrs.embed_height, 2400)}px` : undefined,
            }
          : undefined
      }
    >
      {button}
      {failure && (
        // Inline references stay compact: the state lives in the tooltip, the accessible
        // name and the details dialog instead of repeating a sentence inside the sentence.
        <span className="rich-wiki-error" role="status" aria-label={failure} title={failure}>
          {embedded ? failure : <span aria-hidden="true">{'⚠'}</span>}
        </span>
      )}
      {embedded && !failure && !available && (
        <span className="rich-caption">{t('正在解析嵌入…', 'Resolving embed…')}</span>
      )}
      {content}
      {typeof document !== 'undefined' &&
        createPortal(
          <dialog
            ref={details}
            className="reference-detail"
            aria-label={t('引用目标', 'Reference target')}
            onClick={(event) => event.stopPropagation()}
            onCancel={(event) => {
              event.preventDefault();
              event.stopPropagation();
              closeDetails();
            }}
          >
            <header>
              <h2>{t('引用目标', 'Reference target')}</h2>
              <button
                type="button"
                className="icon-button"
                aria-label={t('关闭', 'Close')}
                onClick={closeDetails}
              >
                <X size={18} />
              </button>
            </header>
            <h3>{label}</h3>
            {available && resolved.kind !== 'image' && (
              <p>
                {resolved.kind === 'note' ? resolved.note.title : resolved.source.display_title}
              </p>
            )}
            <p className="muted">
              {attrs.target_revision_id
                ? t('固定版本', 'Pinned version')
                : t('跟随当前版本', 'Follows current version')}
            </p>
            {attrs.target_id && (
              <p className="reference-identity">
                {t('目标标识', 'Target ID')} · {attrs.target_id}
              </p>
            )}
            {failure && (
              <p className="inline-error" role="alert">
                {failure}
              </p>
            )}
            <button
              type="button"
              className="primary-button"
              disabled={busyKey === key}
              onClick={() => void open()}
            >
              {available ? t('打开目标', 'Open target') : t('重新解析并打开', 'Resolve and open')}{' '}
              <ArrowUpRight size={14} />
            </button>
          </dialog>,
          document.body,
        )}
    </span>
  );
}

function SourceEmbed({
  result,
  identity,
}: {
  result: Extract<WikiResult, { kind: 'source' }>;
  identity: WorkspaceIdentity;
}) {
  const { t } = useI18n();
  const [text, setText] = useState<SourceTextPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (result.version.format === 'pdf') return;
    let live = true;
    void call(
      {
        kind: 'sources_text',
        payload: {
          source_version_id: result.version.id,
          parse_id: result.sourceTarget.parse_id,
          offset: Math.max(0, result.sourceTarget.block_offset ?? 0),
          limit: 30,
        },
      },
      'source_text',
      identity,
    )
      .then((value) => {
        if (live) setText(value);
      })
      .catch((cause) => {
        if (live) setError(message(cause));
      });
    return () => {
      live = false;
    };
  }, [
    result.version.id,
    result.sourceTarget.parse_id,
    result.sourceTarget.block_offset,
    identity.local_instance_id,
    identity.workspace_id,
  ]);
  const approximate =
    result.sourceTarget.locator_status === 'approximate' ? (
      <p className="rich-caption">
        {result.sourceTarget.annotation_id === null &&
        result.sourceTarget.locator?.kind === 'pdf' &&
        result.sourceTarget.locator.rects.length === 0
          ? t('定位资料页，未标记具体文字。', 'Links to a source page without a text highlight.')
          : t(
              '摘录位置为近似定位，请核对原文。',
              'The excerpt location is approximate; check the original source.',
            )}
      </p>
    ) : null;
  if (result.version.format === 'pdf')
    return (
      <>
        {approximate}
        <div className="rich-source-pdf">
          <Suspense fallback={<p role="status">{t('正在读取 PDF…', 'Loading PDF…')}</p>}>
            <PdfReader
              identity={identity}
              versionId={result.version.id}
              location={result.sourceTarget.locator}
              onSelection={noPosition}
              onPosition={noPosition}
            />
          </Suspense>
        </div>
      </>
    );
  if (error)
    return (
      <p className="inline-error" role="alert">
        {error}
      </p>
    );
  if (!text) return <p role="status">{t('正在读取资料…', 'Loading source…')}</p>;
  let remaining = 12000;
  const paragraphs = text.blocks.map((block) => {
    if (remaining <= 0) return null;
    let end = Math.min(block.text.length, remaining);
    if (end < block.text.length && end > 0 && /[\uD800-\uDBFF]/.test(block.text[end - 1])) --end;
    const body = block.text.slice(0, end);
    remaining -= body.length;
    const location = result.sourceTarget.locator;
    if (location?.kind === 'text' && location.block_id === block.id) {
      const points = Array.from(body);
      if (
        location.end_codepoint <= points.length &&
        points.slice(location.start_codepoint, location.end_codepoint).join('') === location.quote
      )
        return (
          <p key={block.id}>
            {points.slice(0, location.start_codepoint).join('')}
            <mark>{location.quote}</mark>
            {points.slice(location.end_codepoint).join('')}
          </p>
        );
    }
    return <p key={block.id}>{body}</p>;
  });
  return (
    <div className="rich-source-text">
      {approximate}
      {paragraphs}
      {(text.next_offset !== null || remaining <= 0) && (
        <p className="rich-caption">
          {t('打开资料查看后续内容。', 'Open the source to continue reading.')}
        </p>
      )}
    </div>
  );
}
