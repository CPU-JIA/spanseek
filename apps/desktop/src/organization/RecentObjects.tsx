import { useEffect, useId, useState } from 'react';
import { Clock3, FileText, RefreshCw } from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { displayDate } from '../format';
import './recent-objects.css';

/** The inbox keeps a short "continue where you left off" list. */
export const RECENT_LIMIT = 5;

export type RecentEntry = {
  id: string;
  title: string;
  kind: 'note' | 'source';
  trashed: boolean;
  lastOpenedAt: string;
};

/**
 * Reads the workspace recency log and resolves every entry to a title. Entries
 * whose object is gone (or cannot be displayed by this build) are dropped instead
 * of being rendered as a stale link.
 */
export async function loadRecentObjects(
  identity: WorkspaceIdentity,
  limit = RECENT_LIMIT,
): Promise<RecentEntry[]> {
  const recent = await call({ kind: 'recent_objects_list' }, 'recent_objects', identity);
  const entries = await Promise.all(
    recent.slice(0, limit).map(async (item): Promise<RecentEntry | null> => {
      try {
        const metadata = await call(
          { kind: 'objects_metadata_get', payload: { object_id: item.object_id } },
          'object_metadata',
          identity,
        );
        if (metadata.kind === 'note') {
          const note = await call(
            { kind: 'notes_get', payload: { note_id: item.object_id } },
            'note',
            identity,
          );
          return {
            id: note.id,
            title: note.title,
            kind: 'note',
            trashed: note.trashed,
            lastOpenedAt: item.last_opened_at,
          };
        }
        if (metadata.kind === 'source') {
          const source = await call(
            { kind: 'sources_get', payload: { source_id: item.object_id } },
            'source',
            identity,
          );
          return {
            id: source.id,
            title: source.display_title,
            kind: 'source',
            trashed: source.trashed,
            lastOpenedAt: item.last_opened_at,
          };
        }
        return null;
      } catch {
        // A recency row outlives its object until the next purge. Dropping the
        // row is honest; inventing a placeholder title would not be.
        return null;
      }
    }),
  );
  return entries.filter((entry) => entry !== null);
}

export function RecentObjects({
  identity,
  activeId,
  busy,
  onOpen,
}: {
  identity: WorkspaceIdentity;
  activeId: string | null;
  busy: boolean;
  onOpen: (id: string) => void;
}) {
  const { t, language } = useI18n();
  const headingId = useId();
  const [entries, setEntries] = useState<RecentEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    setError(null);
    void loadRecentObjects(identity)
      .then((items) => {
        if (live) setEntries(items);
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [identity, activeId, attempt]);
  if (error)
    return (
      <section className="recent-objects" aria-label={t('最近打开', 'Recently opened')}>
        <p className="recent-objects-error" role="status">
          {t('最近打开读取失败：', 'Could not read recently opened items: ')}
          {error}
        </p>
        <button
          type="button"
          className="text-button"
          disabled={busy}
          onClick={() => setAttempt((value) => value + 1)}
        >
          <RefreshCw size={13} aria-hidden="true" />
          {t('重试', 'Retry')}
        </button>
      </section>
    );
  if (!entries || entries.length === 0) return null;
  return (
    <section className="recent-objects" aria-labelledby={headingId}>
      <div className="recent-objects-heading">
        <h2 id={headingId}>
          <Clock3 size={13} aria-hidden="true" />
          {t('最近打开', 'Recently opened')}
        </h2>
        <small>{t('继续上次的工作', 'Continue where you left off')}</small>
      </div>
      <ul className="recent-object-list">
        {entries.map((entry) => (
          <li key={entry.id}>
            <button
              type="button"
              className={`recent-object ${entry.id === activeId ? 'active' : ''}`}
              aria-current={entry.id === activeId ? 'true' : undefined}
              // A trashed object cannot be opened; say so instead of letting the
              // click fail. The reference panel disables the same state.
              disabled={busy || entry.trashed}
              title={
                entry.trashed
                  ? t('已在回收站，请先恢复再打开。', 'In the trash. Restore it before opening.')
                  : entry.title
              }
              onClick={() => onOpen(entry.id)}
            >
              <FileText size={14} aria-hidden="true" />
              <span className="recent-object-title" title={entry.title}>
                {entry.title}
              </span>
              {entry.trashed && <span className="recent-object-state">{t('回收站', 'Trash')}</span>}
              <small>{displayDate(entry.lastOpenedAt, language)}</small>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
