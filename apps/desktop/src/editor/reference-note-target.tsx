import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { FileText, Heading, Pilcrow, Search } from 'lucide-react';
import type { Note, NoteSummary, RevisionPage, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { displayDate } from '../format';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { referenceBlocks } from './reference-model';
import type { ReferenceDraft } from './reference-picker-model';

export function ReferenceNoteTarget({
  identity,
  selected,
  disabled,
  onChange,
}: {
  identity: WorkspaceIdentity;
  selected: NoteSummary;
  disabled: boolean;
  onChange: (draft: ReferenceDraft | null) => void;
}) {
  const { t, language } = useI18n();
  const selectId = useId();
  const [revision, setRevision] = useState('current');
  const [revisions, setRevisions] = useState<RevisionPage | null>(null);
  const [loadedNote, setLoadedNote] = useState<{ revision: string; note: Note } | null>(null);
  const note = loadedNote?.revision === revision ? loadedNote.note : null;
  const [blockId, setBlockId] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [count, setCount] = useState(60);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [loadingVersions, setLoadingVersions] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    let live = true;
    setLoadingVersions(true);
    void call(
      { kind: 'revisions_list', payload: { note_id: selected.id, before: null, limit: 50 } },
      'revisions',
      identity,
    )
      .then((value) => {
        if (live) setRevisions(value);
      })
      .catch((cause) => {
        if (live) setError(message(cause));
      })
      .finally(() => {
        if (live) setLoadingVersions(false);
      });
    return () => {
      live = false;
    };
  }, [selected.id, identity.local_instance_id, identity.workspace_id, retry]);
  useEffect(() => {
    let live = true;
    onChange(null);
    setLoadedNote(null);
    setError(null);
    setBlockId(null);
    setQuery('');
    setCount(60);
    void call(
      revision === 'current'
        ? { kind: 'notes_get', payload: { note_id: selected.id } }
        : { kind: 'revision_get', payload: { note_id: selected.id, revision_id: revision } },
      'note',
      identity,
    )
      .then((value) => {
        if (live) setLoadedNote({ revision, note: value });
      })
      .catch((cause) => {
        if (live) setError(message(cause));
      });
    return () => {
      live = false;
    };
  }, [selected.id, revision, identity.local_instance_id, identity.workspace_id, retry, onChange]);
  const blocks = useMemo(() => (note ? referenceBlocks(note.document) : []), [note]);
  const filtered = useMemo(
    () =>
      blocks.filter((block) =>
        block.text.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()),
      ),
    [blocks, query],
  );
  useEffect(() => {
    if (
      !note ||
      note.trashed ||
      (blockId !== null && !blocks.some((block) => block.id === blockId))
    ) {
      onChange(null);
      return;
    }
    onChange({
      target: {
        target_id: note.id,
        target_block_id: blockId,
        target_revision_id: revision === 'current' ? null : note.revision_id,
      },
      label: note.title,
      preview: blocks.find((block) => block.id === blockId)?.text,
    });
  }, [note, blockId, revision, blocks, onChange]);
  async function moreVersions() {
    if (loadingVersions || !revisions?.next_before) return;
    setLoadingVersions(true);
    try {
      const next = await call(
        {
          kind: 'revisions_list',
          payload: { note_id: selected.id, before: revisions.next_before, limit: 50 },
        },
        'revisions',
        identity,
      );
      if (alive.current)
        setRevisions((current) => ({
          items: [...(current?.items ?? []), ...next.items],
          next_before: next.next_before,
        }));
    } catch (cause) {
      if (alive.current) setError(message(cause));
    } finally {
      if (alive.current) setLoadingVersions(false);
    }
  }
  return (
    <>
      <h3>{note?.title || selected.title}</h3>
      <div className="choice-field reference-version">
        <span>{t('目标版本', 'Target version')}</span>
        <Select
          id={selectId}
          aria-label={t('目标版本', 'Target version')}
          value={revision}
          disabled={disabled}
          onValueChange={(value) => {
            onChange(null);
            setRevision(value);
          }}
          options={[
            { value: 'current', label: t('跟随当前版本', 'Follow current version') },
            ...(revisions?.items ?? []).map((item) => ({
              value: item.revision_id,
              label: t(`固定第 ${item.revision} 版`, `Pin version ${item.revision}`),
              description: `${displayDate(item.created_at, language)} · ${item.title}`,
            })),
          ]}
        />
      </div>
      {revisions?.next_before && (
        <button
          type="button"
          className="text-button"
          disabled={disabled || loadingVersions}
          onClick={() => void moreVersions()}
        >
          {t('载入更早版本', 'Load earlier versions')}
        </button>
      )}
      <p className="muted reference-picker-hint">
        {revision === 'current'
          ? t(
              '跟随稳定身份，改名或移动不会改指向。',
              'Follows the stable identity through renaming and moves.',
            )
          : t(
              '固定版本打开只读历史，不会替换当前正文。',
              'A pinned version opens read-only history and preserves the current note.',
            )}
      </p>
      {!note && !error && <p role="status">{t('正在读取目标…', 'Loading target…')}</p>}
      {note && (
        <>
          <button
            type="button"
            className={`reference-target${blockId === null ? ' is-selected' : ''}`}
            aria-pressed={blockId === null}
            disabled={disabled || note.trashed}
            onClick={() => setBlockId(null)}
          >
            <FileText size={17} />
            <span>
              <strong>{t('整篇笔记', 'Whole note')}</strong>
              <small>
                {t(`第 ${note.revision} 版`, `Version ${note.revision}`)} ·{' '}
                {displayDate(note.updated_at, language)}
              </small>
            </span>
          </button>
          <label className="reference-search">
            <Search size={15} />
            <input
              aria-label={t('查找标题或段落', 'Find heading or paragraph')}
              value={query}
              disabled={disabled}
              onChange={(event) => {
                setQuery(event.target.value);
                setCount(60);
              }}
              placeholder={t('查找标题或段落…', 'Find a heading or paragraph…')}
            />
          </label>
          <div
            className="reference-target-list"
            aria-label={t('标题与段落', 'Headings and paragraphs')}
          >
            {filtered.slice(0, count).map((block) => (
              <button
                type="button"
                key={block.id}
                className={`reference-target${blockId === block.id ? ' is-selected' : ''}`}
                aria-pressed={blockId === block.id}
                disabled={disabled || note.trashed}
                onClick={() => setBlockId(block.id)}
              >
                {block.kind === 'heading' ? <Heading size={16} /> : <Pilcrow size={16} />}
                <span>
                  <strong>{block.text}</strong>
                  <small>
                    {block.kind === 'heading' ? t('标题', 'Heading') : t('段落', 'Paragraph')} ·{' '}
                    {block.id.slice(-8)}
                  </small>
                </span>
              </button>
            ))}
            {filtered.length > count && (
              <button
                type="button"
                className="text-button"
                disabled={disabled}
                onClick={() => setCount((value) => value + 60)}
              >
                {t('显示更多段落', 'Show more paragraphs')}
              </button>
            )}
            {!filtered.length && (
              <p className="muted">
                {t('没有匹配的标题或段落。', 'No matching headings or paragraphs.')}
              </p>
            )}
          </div>
          {blockId && (
            <blockquote className="reference-selection-preview">
              {blocks.find((block) => block.id === blockId)?.text}
            </blockquote>
          )}
        </>
      )}
      {error && (
        <div className="inline-error" role="alert">
          <p>{error}</p>
          <button type="button" disabled={disabled} onClick={() => setRetry((value) => value + 1)}>
            {t('重试读取目标', 'Retry loading target')}
          </button>
        </div>
      )}
    </>
  );
}
