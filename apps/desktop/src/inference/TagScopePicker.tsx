import { useEffect, useRef, useState } from 'react';
import type { Tag, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { tagToneClass } from '../ui/tag-color';

export function TagScopePicker({
  identity,
  selected,
  onChange,
}: {
  identity: WorkspaceIdentity;
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  const { t } = useI18n();
  const [tags, setTags] = useState<Tag[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const live = useRef(true);
  const flight = useRef(false);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  async function read(after: string | null) {
    if (flight.current) return;
    flight.current = true;
    setBusy(true);
    try {
      const page = await call(
        { kind: 'tags_list', payload: { limit: 50, cursor: after, query: null } },
        'tags',
        identity,
      );
      if (live.current) {
        setTags((old) => (after ? [...old, ...page.items] : page.items));
        setCursor(page.next_cursor);
        setLoaded(true);
        setError(null);
      }
    } catch (e) {
      if (live.current) setError(message(e));
    } finally {
      flight.current = false;
      if (live.current) setBusy(false);
    }
  }
  return (
    <details
      onToggle={(e) => {
        if (e.currentTarget.open && !loaded) void read(null);
      }}
    >
      <summary>
        {t('标签过滤', 'Filter by tags')} · {selected.length}
      </summary>
      {error && (
        <p className="inference-error" role="alert">
          {error}
          <button type="button" disabled={busy} onClick={() => void read(null)}>
            {t('重新读取', 'Reload')}
          </button>
        </p>
      )}
      {busy && <p>{t('读取标签…', 'Loading tags…')}</p>}
      {loaded && !tags.length && <p>{t('没有标签。', 'No tags.')}</p>}
      <div className="inference-tag-options">
        {tags.map((tag) => (
          <label key={tag.id}>
            <input
              type="checkbox"
              checked={selected.includes(tag.id)}
              onChange={(e) =>
                onChange(
                  e.target.checked ? [...selected, tag.id] : selected.filter((id) => id !== tag.id),
                )
              }
            />
            <span className={`tag-inline ${tagToneClass(tag.id)}`}>{tag.display_name}</span>
          </label>
        ))}
      </div>
      {cursor && (
        <button
          type="button"
          className="text-button"
          disabled={busy}
          onClick={() => void read(cursor)}
        >
          {t('更多标签', 'More tags')}
        </button>
      )}
      {!!selected.length && (
        <button type="button" className="text-button" onClick={() => onChange([])}>
          {t('清除标签限制', 'Clear tag restrictions')}
        </button>
      )}
    </details>
  );
}
