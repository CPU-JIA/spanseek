import { Select } from '../ui/Select';
import { useI18n } from '../i18n';
import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { Calendar, Clock, Plus, Tag as TagIcon, X } from 'lucide-react';
import type { InboxState, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import type { DraftController } from '../editor/draft-controller';
import { MetadataController } from './metadata-controller';
import { indexedIntents } from './intent-storage';
import { tagToneClass } from '../ui/tag-color';
import { displayDateTime } from '../format';
import { ObjectTypeProperties } from './ObjectTypeControls';
import type { ObjectTypeDefinition } from './object-types';

const controllers = new WeakMap<DraftController, Promise<MetadataController>>();
export function getMetadataController(controller: DraftController, identity: WorkspaceIdentity) {
  let pending = controllers.get(controller);
  if (!pending) {
    const objectId = controller.getSnapshot().record.base.id;
    pending = MetadataController.open(
      identity,
      objectId,
      controller,
      indexedIntents,
      () =>
        call(
          { kind: 'objects_metadata_get', payload: { object_id: objectId } },
          'object_metadata',
          identity,
        ),
      (command) => call(command, 'metadata_commit', identity),
    ).catch((error) => {
      controllers.delete(controller);
      throw error;
    });
    controllers.set(controller, pending);
  }
  return pending;
}
export function NoteOrganization({
  controller,
  identity,
  onChanged,
  generation,
  createdAt,
  updatedAt,
  words,
  characters,
  customTypes = [],
  onOpenNote,
}: {
  controller: DraftController;
  identity: WorkspaceIdentity;
  onChanged: () => void;
  generation: number;
  createdAt: string;
  updatedAt: string;
  words: number;
  characters: number;
  customTypes?: readonly ObjectTypeDefinition[];
  onOpenNote?: (id: string) => void;
}) {
  const { t } = useI18n();
  const { record } = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const [metadata, setMetadata] = useState<MetadataController | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let live = true;
    setMetadata(null);
    setError(null);
    void getMetadataController(controller, identity)
      .then((value) => {
        if (live) setMetadata(value);
      })
      .catch((error) => {
        if (live) setError(message(error));
      });
    return () => {
      live = false;
    };
  }, [controller, identity, retry]);
  useEffect(() => {
    if (metadata) void metadata.refresh().catch(() => undefined); // Controller exposes the error.
  }, [metadata, record.base.revision_id, generation]);
  if (!metadata)
    return (
      <section className="note-organization" aria-busy={!error}>
        <div className="note-tag-strip">
          <span className={error ? 'note-property-error' : 'note-property-empty'}>
            {error ?? t('正在读取属性…', 'Loading properties…')}
          </span>
          {error && (
            <button className="text-button" onClick={() => setRetry((value) => value + 1)}>
              {t('重新读取', 'Reload')}
            </button>
          )}
        </div>
        <div className="note-property-table">
          <div className="note-property-row">
            <span>{t('整理状态', 'Status')}</span>
            <div className="note-property-value">
              {error ? t('暂不可用', 'Unavailable') : t('正在读取…', 'Loading…')}
            </div>
          </div>
          <PropertyMetrics
            words={words}
            characters={characters}
            createdAt={createdAt}
            updatedAt={updatedAt}
          />
        </div>
      </section>
    );
  return (
    <OrganizationForm
      identity={identity}
      onOpenNote={onOpenNote}
      controller={metadata}
      draftController={controller}
      onChanged={onChanged}
      createdAt={createdAt}
      updatedAt={updatedAt}
      words={words}
      characters={characters}
      customTypes={customTypes}
    />
  );
}

function PropertyRow({
  label,
  icon,
  children,
}: {
  label: string;
  icon?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="note-property-row">
      <span className="note-property-label">
        {icon}
        {label}
      </span>
      <div className="note-property-value">{children}</div>
    </div>
  );
}

function PropertyMetrics({
  words,
  characters,
  createdAt,
  updatedAt,
}: {
  words: number;
  characters: number;
  createdAt: string;
  updatedAt: string;
}) {
  const { t, language } = useI18n();
  return (
    <>
      <PropertyRow label={t('字数', 'Count')}>
        <span className="note-count-value">
          <span>{t(`${words} 词`, `${words} ${words === 1 ? 'word' : 'words'}`)}</span>
          <span className="note-property-separator" aria-hidden="true">
            ·
          </span>
          <span>
            {t(
              `${characters} 字符`,
              `${characters} ${characters === 1 ? 'character' : 'characters'}`,
            )}
          </span>
        </span>
      </PropertyRow>
      <PropertyRow label={t('创建', 'Created')} icon={<Calendar size={14} />}>
        <time dateTime={createdAt} title={createdAt}>
          {displayDateTime(createdAt, language)}
        </time>
      </PropertyRow>
      <PropertyRow label={t('更新', 'Updated')} icon={<Clock size={14} />}>
        <time dateTime={updatedAt} title={updatedAt}>
          {displayDateTime(updatedAt, language)}
        </time>
      </PropertyRow>
    </>
  );
}

function OrganizationForm({
  controller,
  draftController,
  onChanged,
  createdAt,
  updatedAt,
  words,
  characters,
  customTypes = [],
  identity,
  onOpenNote,
}: {
  controller: MetadataController;
  draftController: DraftController;
  onChanged: () => void;
  createdAt: string;
  updatedAt: string;
  words: number;
  characters: number;
  customTypes?: readonly ObjectTypeDefinition[];
  identity: WorkspaceIdentity;
  onOpenNote?: (id: string) => void;
}) {
  const { t } = useI18n();
  const { metadata, busy, pending, error, generation } = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [tagName, setTagName] = useState('');
  const [addingTag, setAddingTag] = useState(false);
  const tagInput = useRef<HTMLInputElement>(null);
  const addTagButton = useRef<HTMLButtonElement>(null);
  const wasAddingTag = useRef(false);
  const previous = useRef(generation);
  useEffect(() => {
    if (addingTag) tagInput.current?.focus();
    else if (wasAddingTag.current) addTagButton.current?.focus({ preventScroll: true });
    wasAddingTag.current = addingTag;
  }, [addingTag]);
  useEffect(() => {
    if (previous.current !== generation) {
      previous.current = generation;
      onChanged();
    }
  }, [generation, onChanged]);
  if (!metadata) return null;
  const disabled = busy || pending;
  function setInbox(target: InboxState) {
    void controller.mutate((current) => ({
      kind: 'objects_set_inbox_state',
      payload: {
        command_id: crypto.randomUUID(),
        request: { object_id: current.id, base_row_version: current.row_version, target },
      },
    }));
  }
  return (
    <section
      className="note-organization"
      aria-label={t('笔记整理', 'Note properties')}
      aria-busy={busy}
    >
      <div className="note-tag-strip" aria-label={t('标签与整理状态', 'Tags and status')}>
        <span className={`note-status-chip is-${metadata.inbox_state ?? 'pending'}`}>
          <span className="state-dot" aria-hidden="true" />
          {metadata.inbox_state === 'processed'
            ? t('已整理', 'Organized')
            : metadata.inbox_state === 'snoozed'
              ? t('稍后整理', 'Later')
              : t('待整理', 'Inbox')}
        </span>
        {metadata.tags.map((tag) => (
          <span className={`tag-chip ${tagToneClass(tag.id)}`} key={tag.id}>
            {tag.display_name}
            <button
              type="button"
              aria-label={t(`移除标签 ${tag.display_name}`, `Remove tag ${tag.display_name}`)}
              disabled={disabled}
              onClick={() =>
                void controller.mutate((current) => ({
                  kind: 'tags_remove',
                  payload: {
                    command_id: crypto.randomUUID(),
                    request: {
                      targets: [{ object_id: current.id, base_row_version: current.row_version }],
                      tag_id: tag.id,
                    },
                  },
                }))
              }
            >
              <X size={12} />
            </button>
          </span>
        ))}
        {!metadata.tags.length && (
          <span className="note-property-empty">{t('尚无标签', 'No tags yet')}</span>
        )}
      </div>
      <details className="note-detail-disclosure">
        <summary>
          <span>{t('详细信息', 'Details')}</span>
          <span className="note-detail-summary">
            {t('属性、整理状态与统计', 'Properties, status and stats')}
          </span>
        </summary>
        <div className="note-property-table">
          <ObjectTypeProperties
            identity={identity}
            onOpenNote={onOpenNote}
            controller={draftController}
            onChanged={onChanged}
            disabled={disabled}
            customTypes={customTypes}
          />
          <PropertyRow label={t('整理状态', 'Status')}>
            <Select
              aria-label={t('整理状态', 'Organization status')}
              disabled={disabled}
              value={metadata.inbox_state ?? 'pending'}
              onValueChange={(nextValue) => setInbox(nextValue as InboxState)}
            >
              <option value="pending">{t('收件箱 · 待整理', 'Inbox · Unprocessed')}</option>
              <option value="snoozed">{t('稍后整理', 'Later')}</option>
              <option value="processed">{t('已整理', 'Organized')}</option>
            </Select>
          </PropertyRow>
          <PropertyMetrics
            words={words}
            characters={characters}
            createdAt={createdAt}
            updatedAt={updatedAt}
          />
          {addingTag && (
            <form
              className="tag-entry note-tag-entry"
              onSubmit={(event) => {
                event.preventDefault();
                const name = tagName;
                if (!name.trim()) return;
                void controller
                  .mutate((current) => ({
                    kind: 'tags_assign',
                    payload: {
                      command_id: crypto.randomUUID(),
                      request: {
                        targets: [{ object_id: current.id, base_row_version: current.row_version }],
                        tag: { kind: 'name', name },
                      },
                    },
                  }))
                  .then((success) => {
                    if (!success) return;
                    setTagName('');
                    setAddingTag(false);
                  });
              }}
            >
              <TagIcon size={15} aria-hidden="true" />
              <input
                ref={tagInput}
                aria-label={t('添加标签', 'Add tag')}
                placeholder={t('输入标签名称…', 'Enter a tag name…')}
                maxLength={100}
                disabled={disabled}
                value={tagName}
                onChange={(event) => setTagName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') {
                    event.preventDefault();
                    setAddingTag(false);
                  }
                }}
              />
              <button
                type="submit"
                className="icon-button"
                aria-label={t('确认添加标签', 'Add tag to note')}
                disabled={!tagName.trim() || disabled}
              >
                <Plus size={16} />
              </button>
            </form>
          )}
          {!addingTag && (
            <button
              ref={addTagButton}
              type="button"
              className="note-property-add text-button"
              disabled={disabled}
              onClick={() => setAddingTag(true)}
            >
              <Plus size={14} />
              {t('添加标签', 'Add tag')}
            </button>
          )}
        </div>
      </details>
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
      {pending && !busy && (
        <p className="pending-intent">
          {t(
            '上次整理操作的结果待确认。记录已保存在本机，重开后仍可继续确认。',
            'The previous change is awaiting confirmation. It is saved locally and can be checked after reopening.',
          )}
        </p>
      )}
      {pending ? (
        <button className="text-button" disabled={busy} onClick={() => void controller.mutate()}>
          {t('确认上次操作结果', 'Check previous change')}
        </button>
      ) : (
        error && (
          <button
            className="text-button"
            disabled={busy}
            onClick={() => void controller.refresh().catch(() => undefined)}
          >
            {t('重新读取', 'Reload')}
          </button>
        )
      )}
    </section>
  );
}
