import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { BookOpen, FileText, Search } from 'lucide-react';
import type {
  SourceBlock,
  SourceSummary,
  SourceTextPage,
  SourceVersion,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import {
  sourceBlockReference,
  sourceReferenceBlocks,
  type ReferenceDraft,
} from './reference-picker-model';

export function ReferenceSourceTarget({
  identity,
  selected,
  disabled,
  onChange,
}: {
  identity: WorkspaceIdentity;
  selected: SourceSummary;
  disabled: boolean;
  onChange: (draft: ReferenceDraft | null) => void;
}) {
  const { t, language } = useI18n();
  const selectId = useId();
  const [versionId, setVersionId] = useState(selected.current_version_id);
  const [version, setVersion] = useState<SourceVersion | null>(null);
  const [versions, setVersions] = useState<SourceVersion[]>([]);
  const [moreVersions, setMoreVersions] = useState(false);
  const [text, setText] = useState<SourceTextPage | null>(null);
  const [blockId, setBlockId] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [textError, setTextError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [loading, setLoading] = useState(false);
  const [versionsBusy, setVersionsBusy] = useState(false);
  const request = useRef(0);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      request.current++;
    };
  }, []);
  useEffect(() => {
    let live = true;
    void call(
      { kind: 'sources_versions', payload: { source_id: selected.id, after_id: null, limit: 30 } },
      'source_versions',
      identity,
    )
      .then((value) => {
        if (live) {
          setVersions(value);
          setMoreVersions(value.length === 30);
        }
      })
      .catch((cause) => {
        if (live) setError(message(cause));
      });
    return () => {
      live = false;
    };
  }, [selected.id, identity.local_instance_id, identity.workspace_id, retry]);
  useEffect(() => {
    const id = ++request.current;
    setVersion(null);
    setBlockId(null);
    setText(null);
    setQuery('');
    setError(null);
    setTextError(null);
    setLoading(true);
    onChange(null);
    void call(
      { kind: 'sources_version', payload: { source_version_id: versionId } },
      'source_version',
      identity,
    )
      .then((value) => {
        if (id === request.current) setVersion(value);
      })
      .catch((cause) => {
        if (id === request.current) setError(message(cause));
      });
    void call(
      {
        kind: 'sources_text',
        payload: { source_version_id: versionId, parse_id: null, offset: 0, limit: 100 },
      },
      'source_text',
      identity,
    )
      .then((value) => {
        if (id === request.current) setText(value);
      })
      .catch((cause) => {
        if (id === request.current)
          setTextError(
            (cause as { details?: { code?: string } }).details?.code === 'not_found'
              ? t(
                  '此版本仍在准备可定位的页面与文字，可先引用整份资料，或稍后重试。',
                  'This version is still preparing pages and text. Reference the whole source or retry later.',
                )
              : message(cause),
          );
      })
      .finally(() => {
        if (id === request.current) setLoading(false);
      });
    return () => {
      request.current++;
    };
  }, [versionId, identity.local_instance_id, identity.workspace_id, retry, onChange]);
  const blocks = useMemo(
    () => (version ? sourceReferenceBlocks(text?.blocks ?? [], version.format) : []),
    [text, version],
  );
  const filtered = useMemo(
    () =>
      blocks.filter((block) => {
        const wanted = query.trim().toLocaleLowerCase();
        return (
          !wanted ||
          block.text.toLocaleLowerCase().includes(wanted) ||
          (block.page_index !== null && String(block.page_index + 1) === wanted)
        );
      }),
    [blocks, query],
  );
  useEffect(() => {
    const block = blockId === null ? null : blocks.find((item) => item.id === blockId);
    onChange(
      version &&
        version.id === versionId &&
        version.source_id === selected.id &&
        block !== undefined &&
        !selected.trashed
        ? sourceBlockReference(selected, version.id, block)
        : null,
    );
  }, [version, versionId, selected, blockId, blocks, onChange]);
  async function nextText() {
    if (loading || text?.next_offset == null) return;
    const id = request.current;
    setLoading(true);
    setTextError(null);
    try {
      const next = await call(
        {
          kind: 'sources_text',
          payload: {
            source_version_id: versionId,
            parse_id: text.parse_id,
            offset: text.next_offset,
            limit: 100,
          },
        },
        'source_text',
        identity,
      );
      if (id === request.current)
        setText((current) =>
          current ? { ...next, blocks: [...current.blocks, ...next.blocks] } : next,
        );
    } catch (cause) {
      if (id === request.current) setTextError(message(cause));
    } finally {
      if (id === request.current) setLoading(false);
    }
  }
  async function nextVersions() {
    if (versionsBusy || !moreVersions || !versions.length) return;
    setVersionsBusy(true);
    try {
      const next = await call(
        {
          kind: 'sources_versions',
          payload: { source_id: selected.id, after_id: versions.at(-1)!.id, limit: 30 },
        },
        'source_versions',
        identity,
      );
      if (alive.current) {
        setVersions((current) => [...current, ...next]);
        setMoreVersions(next.length === 30);
      }
    } catch (cause) {
      if (alive.current) setError(message(cause));
    } finally {
      if (alive.current) setVersionsBusy(false);
    }
  }
  const availableVersions =
    versions.some((item) => item.id === versionId) || !version ? versions : [version, ...versions];
  return (
    <>
      <h3>{selected.display_title}</h3>
      <div className="choice-field reference-version">
        <span>{t('固定原件版本', 'Pinned original version')}</span>
        <Select
          id={selectId}
          aria-label={t('固定原件版本', 'Pinned original version')}
          value={versionId}
          disabled={disabled}
          onValueChange={(value) => {
            onChange(null);
            setVersionId(value);
          }}
          options={availableVersions.map((item) => ({
            value: item.id,
            label: `${item.original_name} · ${displayDate(item.captured_at, language)}`,
            description:
              item.id === selected.current_version_id
                ? t('当前原件', 'Current original')
                : t('历史原件', 'Historical original'),
          }))}
        />
      </div>
      {moreVersions && (
        <button
          type="button"
          className="text-button"
          disabled={disabled || versionsBusy}
          onClick={() => void nextVersions()}
        >
          {t('载入更早原件', 'Load earlier originals')}
        </button>
      )}
      {version && (
        <p className="reference-picker-hint muted">{version.origin_url || version.original_name}</p>
      )}
      <button
        type="button"
        className={`reference-target${blockId === null ? ' is-selected' : ''}`}
        aria-pressed={blockId === null}
        disabled={disabled || !version}
        onClick={() => setBlockId(null)}
      >
        <BookOpen size={17} />
        <span>
          <strong>{t('整份资料', 'Whole source')}</strong>
          <small>{t('保留此次选择的原件版本', 'Keep the selected original version')}</small>
        </span>
      </button>
      <label className="reference-search">
        <Search size={15} />
        <input
          aria-label={t('查找已载入的页面或段落', 'Find a loaded page or paragraph')}
          placeholder={
            version?.format === 'pdf'
              ? t('输入页码或页面文字…', 'Enter a page number or page text…')
              : t('查找段落…', 'Find a paragraph…')
          }
          value={query}
          disabled={disabled}
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      <div
        className="reference-target-list"
        aria-label={t('资料页面与段落', 'Source pages and paragraphs')}
        aria-busy={loading}
      >
        {filtered.map((block: SourceBlock) => (
          <button
            type="button"
            key={block.id}
            className={`reference-target${blockId === block.id ? ' is-selected' : ''}`}
            aria-pressed={blockId === block.id}
            disabled={disabled || !version}
            onClick={() => setBlockId(block.id)}
          >
            <FileText size={16} />
            <span>
              {block.page_index !== null && (
                <strong>
                  {t(`第 ${block.page_index + 1} 页`, `Page ${block.page_index + 1}`)}
                  {block.pdf_geometry?.page_label &&
                  block.pdf_geometry.page_label !== String(block.page_index + 1)
                    ? ` · ${block.pdf_geometry.page_label}`
                    : ''}
                </strong>
              )}
              <small>
                {block.text || t('此页没有可搜索文字', 'No searchable text on this page')}
              </small>
            </span>
          </button>
        ))}
        {loading && <p role="status">{t('正在读取页面与文字…', 'Loading pages and text…')}</p>}
        {!loading && !filtered.length && !textError && (
          <p className="muted">
            {t('已载入内容中没有匹配项。', 'No matches in the loaded content.')}
          </p>
        )}
      </div>
      {text?.next_offset != null && (
        <button
          type="button"
          className="text-button"
          disabled={disabled || loading}
          onClick={() => void nextText()}
        >
          {t('载入更多页面与段落', 'Load more pages and paragraphs')}
        </button>
      )}
      {version?.format === 'pdf' && text?.needs_ocr && (
        <p className="muted reference-picker-hint">
          {t(
            '无文字页面可在阅读器建立区域摘录，再从“摘录与批注”选择。',
            'For pages without text, create an area excerpt in the reader, then select it under Excerpts and annotations.',
          )}
        </p>
      )}
      {blockId && (
        <blockquote className="reference-selection-preview">
          {blocks.find((block) => block.id === blockId)?.text}
        </blockquote>
      )}
      {(error || textError) && (
        <div className="inline-error" role="alert">
          <p>{error || textError}</p>
          <button type="button" disabled={disabled} onClick={() => setRetry((value) => value + 1)}>
            {t('重试读取目标', 'Retry loading target')}
          </button>
        </div>
      )}
    </>
  );
}
