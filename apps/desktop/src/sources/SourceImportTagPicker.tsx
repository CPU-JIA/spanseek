import { useEffect, useMemo, useRef, useState } from 'react';
import { Hash, Plus, Search, X } from 'lucide-react';
import type { Tag, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { tagToneClass } from '../ui/tag-color';

function normalizedTagName(name: string) {
  return name.trim().normalize('NFC').toLocaleLowerCase();
}

export function SourceImportTagPicker({
  identity,
  selected,
  onChange,
  disabled,
}: {
  identity: WorkspaceIdentity;
  selected: string[];
  onChange: (names: string[]) => void;
  disabled: boolean;
}) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const [applied, setApplied] = useState('');
  const [tags, setTags] = useState<Tag[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const live = useRef(true);
  const scope = `${identity.local_instance_id}/${identity.workspace_id}`;
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  useEffect(() => {
    const timer = setTimeout(() => setApplied(query.trim()), 180);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => {
    let current = true;
    setLoading(true);
    call(
      {
        kind: 'tags_list',
        payload: { limit: 100, cursor: null, query: applied || null },
      },
      'tags',
      identity,
    )
      .then((page) => {
        if (!current || !live.current) return;
        setTags(page.items);
        setError(null);
      })
      .catch((caught) => {
        if (!current || !live.current) return;
        setError(message(caught));
      })
      .finally(() => {
        if (current && live.current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [scope, applied, retry]);

  const selectedKeys = useMemo(() => new Set(selected.map(normalizedTagName)), [selected]);
  const name = query.trim();
  const existing = tags.find(
    (tag) => normalizedTagName(tag.display_name) === normalizedTagName(name),
  );
  const canCreate =
    !!name && name.length <= 100 && !existing && !selectedKeys.has(normalizedTagName(name));
  const full = selected.length >= 100;

  function toggle(tagName: string) {
    const key = normalizedTagName(tagName);
    if (selectedKeys.has(key)) {
      onChange(selected.filter((value) => normalizedTagName(value) !== key));
    } else if (!full) {
      onChange([...selected, tagName]);
    }
  }

  function addNew() {
    if (!canCreate || full) return;
    onChange([...selected, name]);
    setQuery('');
    setApplied('');
  }

  return (
    <section className="source-import-tags" aria-label={t('导入标签', 'Import tags')}>
      <div className="source-import-tags-heading">
        <span>{t('标签（可选）', 'Tags (optional)')}</span>
        <small>{selected.length}/100</small>
      </div>
      {selected.length > 0 && (
        <ul className="source-import-selected-tags" aria-label={t('已选标签', 'Selected tags')}>
          {selected.map((tag) => (
            <li key={tag}>
              <span className={`tag-inline ${tagToneClass(tag)}`}>#{tag}</span>
              <button
                type="button"
                disabled={disabled}
                aria-label={t(`移除标签 ${tag}`, `Remove tag ${tag}`)}
                onClick={() => toggle(tag)}
              >
                <X size={12} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <label className="source-import-tag-search">
        <Search size={15} aria-hidden="true" />
        <input
          value={query}
          maxLength={100}
          disabled={disabled}
          placeholder={t('搜索或输入新标签…', 'Search or enter a new tag…')}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing || event.keyCode === 229)
              return;
            event.preventDefault();
            event.stopPropagation();
            if (canCreate) addNew();
            else if (existing) toggle(existing.display_name);
          }}
        />
      </label>
      {error ? (
        <p className="source-dialog-error" role="alert">
          {error}
          <button type="button" disabled={loading} onClick={() => setRetry((value) => value + 1)}>
            {t('重新读取标签', 'Reload tags')}
          </button>
        </p>
      ) : loading ? (
        <p className="muted" role="status">
          {t('正在读取标签…', 'Loading tags…')}
        </p>
      ) : (
        <>
          <div className="source-import-tag-options">
            {tags.map((tag) => {
              const active = selectedKeys.has(normalizedTagName(tag.display_name));
              return (
                <button
                  key={tag.id}
                  type="button"
                  className={`source-import-tag-option tag-inline ${tagToneClass(tag.id)}`}
                  aria-pressed={active}
                  disabled={disabled || (full && !active)}
                  onClick={() => toggle(tag.display_name)}
                >
                  <Hash size={12} aria-hidden="true" />
                  {tag.display_name}
                </button>
              );
            })}
            {!tags.length && !name && (
              <p className="muted">
                {t(
                  '还没有标签，输入名称创建第一个。',
                  'No tags yet. Enter a name to create the first one.',
                )}
              </p>
            )}
            {!tags.length && name && !canCreate && (
              <p className="muted">{t('没有匹配的标签。', 'No matching tags.')}</p>
            )}
          </div>
          {canCreate && (
            <button
              type="button"
              className="source-import-tag-create"
              disabled={disabled || full}
              onClick={addNew}
            >
              <Plus size={14} aria-hidden="true" />
              {t(`新建并选择「${name}」`, `Create and select “${name}”`)}
            </button>
          )}
        </>
      )}
    </section>
  );
}
