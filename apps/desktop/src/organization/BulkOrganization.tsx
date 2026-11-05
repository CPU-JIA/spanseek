import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Check, ChevronDown, Link2, Plus, Tag, X } from 'lucide-react';
import type { NoteDocument, WorkspaceIdentity } from '@pkms/contracts';
import type { MetadataController } from './metadata-controller';
import { BulkActions } from './bulk-actions';
import {
  readObjectRelations,
  relationAllowedTypeTokens,
  setFrontmatterRelations,
  setFrontmatterValue,
  type ObjectFieldDefinition,
  type ObjectRelationValue,
  type ObjectTypeDefinition,
} from './object-types';
import { call, message } from '../api';
import { bulkStorage } from './bulk-storage';
import { useI18n } from '../i18n';
import { ReferencePicker } from '../editor/reference-picker';
import { PopupMenu } from '../ui/PopupMenu';
import { Select } from '../ui/Select';
import './bulk-organization.css';

export type BulkOrganizationProps = {
  identity: WorkspaceIdentity;
  getMetadata: (id: string) => Promise<MetadataController>;
  onChanged: () => void;
  propertyFields?: readonly ObjectFieldDefinition[];
  customTypes?: readonly ObjectTypeDefinition[];
};
const instances = new Map<string, Promise<BulkActions>>();
export function BulkOrganization({
  identity,
  getMetadata,
  onChanged,
  propertyFields = [],
  customTypes = [],
  selected,
  onClear,
}: BulkOrganizationProps & {
  selected: { id: string; title: string }[];
  onClear: () => void;
}) {
  const { t } = useI18n();
  const [controller, setController] = useState<BulkActions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const getter = useRef(getMetadata);
  getter.current = getMetadata;
  useEffect(() => {
    let live = true;
    setController(null);
    setError(null);
    const key = `${identity.local_instance_id}/${identity.workspace_id}`;
    let opening = instances.get(key);
    if (!opening) {
      opening = BulkActions.open(identity, bulkStorage, (id) => getter.current(id)).catch(
        (error) => {
          instances.delete(key);
          throw error;
        },
      );
      instances.set(key, opening);
    }
    void opening
      .then((value) => {
        if (live) setController(value);
      })
      .catch((error) => {
        if (live) setError(String(error));
      });
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, retry]);
  if (!controller)
    return error ? (
      <div className="bulk-organization" role="alert">
        <p>{error}</p>
        <button className="text-button" onClick={() => setRetry((value) => value + 1)}>
          {t('重新读取整理记录', 'Reload organization records')}
        </button>
      </div>
    ) : null;
  return (
    <BulkForm
      controller={controller}
      identity={identity}
      selected={selected}
      propertyFields={propertyFields}
      customTypes={customTypes}
      onClear={onClear}
      onChanged={onChanged}
    />
  );
}
type PropertyResult = {
  id: string;
  title: string;
  status: 'waiting' | 'done' | 'failed';
  error?: string;
};
function BulkForm({
  controller,
  identity,
  selected,
  propertyFields,
  customTypes,
  onClear,
  onChanged,
}: {
  controller: BulkActions;
  identity: WorkspaceIdentity;
  selected: { id: string; title: string }[];
  propertyFields: readonly ObjectFieldDefinition[];
  customTypes: readonly ObjectTypeDefinition[];
  onClear: () => void;
  onChanged: () => void;
}) {
  const { t } = useI18n();
  const { busy, record, error } = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
  );
  const [name, setName] = useState('');
  const availableProperties = useMemo(() => propertyFields, [propertyFields]);
  const [propertyFieldKey, setPropertyFieldKey] = useState('');
  const propertyField =
    availableProperties.find((field) => field.key === propertyFieldKey) ?? availableProperties[0];
  const [propertyValue, setPropertyValue] = useState('');
  const [propertyBusy, setPropertyBusy] = useState(false);
  const [propertyResults, setPropertyResults] = useState<PropertyResult[] | null>(null);
  const [propertyExpanded, setPropertyExpanded] = useState(false);
  const [relationTargets, setRelationTargets] = useState<ObjectRelationValue[]>([]);
  const [relationMode, setRelationMode] = useState<'add' | 'replace' | 'clear'>('add');
  const [relationPickerOpen, setRelationPickerOpen] = useState(false);
  const selectedKey = selected.map((item) => item.id).join('/');
  const relationObjectTypes = useMemo(
    () =>
      propertyField?.kind === 'relation'
        ? relationAllowedTypeTokens(propertyField, customTypes)
        : [],
    [customTypes, propertyField],
  );
  const relationExcludedIds = [
    ...selected.map((item) => item.id),
    ...relationTargets.map((item) => item.id),
  ];
  const relationNeedsTarget = propertyField?.kind === 'relation' && relationMode !== 'clear';
  useEffect(() => {
    if (!availableProperties.some((field) => field.key === propertyFieldKey)) {
      setPropertyFieldKey(availableProperties[0]?.key ?? '');
    }
  }, [availableProperties, propertyFieldKey]);
  useEffect(() => {
    setPropertyValue('');
    setRelationTargets([]);
    setRelationMode('add');
    setRelationPickerOpen(false);
  }, [propertyFieldKey]);
  useEffect(() => {
    if (!propertyBusy) setPropertyResults(null);
  }, [propertyFieldKey]);
  useEffect(() => {
    if (!propertyBusy) {
      setPropertyResults(null);
      setPropertyExpanded(false);
      setRelationTargets([]);
      setRelationMode('add');
      setRelationPickerOpen(false);
    }
  }, [selectedKey]);
  useEffect(() => {
    if (!propertyBusy) setPropertyResults(null);
  }, [relationMode, relationTargets]);
  async function applyProperty(targets: { id: string; title: string }[], mergeResults = false) {
    if (!propertyField || propertyBusy || !targets.length) return;
    setPropertyBusy(true);
    const targetIds = new Set(targets.map((item) => item.id));
    const results: PropertyResult[] =
      mergeResults && propertyResults
        ? propertyResults.map((item) =>
            targetIds.has(item.id)
              ? { id: item.id, title: item.title, status: 'waiting' as const }
              : item,
          )
        : [];
    for (const target of targets) {
      if (!results.some((item) => item.id === target.id))
        results.push({ ...target, status: 'waiting' });
    }
    setPropertyResults([...results]);
    let changed = false;
    for (const target of targets) {
      const index = results.findIndex((item) => item.id === target.id);
      if (index < 0) continue;
      try {
        const note = await call(
          { kind: 'notes_get', payload: { note_id: target.id } },
          'note',
          identity,
        );
        let document: NoteDocument;
        if (propertyField.kind === 'relation') {
          if (relationMode !== 'clear' && !relationTargets.length)
            throw new Error(
              t('请先选择至少一个关联目标。', 'Choose at least one linked note first.'),
            );
          if (relationTargets.some((link) => link.id === target.id))
            throw new Error(t('笔记不能关联到自身。', 'A note cannot link to itself.'));
          const current = readObjectRelations(note.document, propertyField.key);
          if (current.error) throw new Error(current.error);
          let values: ObjectRelationValue[];
          if (relationMode === 'clear') {
            values = [];
          } else if (relationMode === 'replace') {
            values = [...relationTargets];
          } else {
            values = [...current.values];
            for (const link of relationTargets) {
              if (!values.some((value) => value.id === link.id)) values.push(link);
            }
          }
          if (values.length > 12)
            throw new Error(
              t(
                '一个关联字段最多保存 12 篇笔记；请减少目标或改用替换。',
                'A relation field can hold at most 12 notes; choose fewer targets or replace the list.',
              ),
            );
          const unchanged =
            values.length === current.values.length &&
            values.every(
              (value) => current.values.find((item) => item.id === value.id)?.title === value.title,
            );
          if (unchanged) {
            results[index] = { ...results[index], status: 'done' };
            setPropertyResults([...results]);
            continue;
          }
          document = setFrontmatterRelations(note.document, propertyField.key, values);
        } else {
          document = setFrontmatterValue(note.document, propertyField.key, propertyValue);
        }
        await call(
          {
            kind: 'notes_save',
            payload: {
              command_id: crypto.randomUUID(),
              note: {
                note_id: note.id,
                base_revision_id: note.revision_id,
                title: note.title,
                document_schema: note.schema_version,
                document,
              },
            },
          },
          'note_commit',
          identity,
        );
        results[index] = { ...results[index], status: 'done' };
        changed = true;
      } catch (failure) {
        results[index] = {
          ...results[index],
          status: 'failed',
          error: message(failure),
        };
      }
      setPropertyResults([...results]);
    }
    setPropertyBusy(false);
    if (changed) onChanged();
  }
  const done = record?.items.filter((item) => item.status === 'done').length ?? 0;
  const previous = useRef(done);
  useEffect(() => {
    if (previous.current !== done) {
      previous.current = done;
      onChanged();
    }
  }, [done, onChanged]);
  if (!record && !selected.length && !error) return null;
  const unresolved = record?.items.some(
    (item) => item.status === 'pending' || item.status === 'ready',
  );
  return (
    <section
      className="bulk-organization"
      aria-label={t('批量整理', 'Organize selected notes')}
      aria-busy={busy || propertyBusy}
    >
      <div className="bulk-heading">
        <strong>
          {record
            ? t('整理结果', 'Organization results')
            : t(`已选 ${selected.length} 篇`, `${selected.length} selected`)}
        </strong>
        {!record && (
          <button
            className="icon-button"
            aria-label={t('清除选择', 'Clear selection')}
            disabled={busy || propertyBusy}
            onClick={onClear}
          >
            <X size={14} />
          </button>
        )}
      </div>
      {!record && (
        <>
          <form
            className="bulk-tag-entry"
            onSubmit={(event) => {
              event.preventDefault();
              if (name.trim()) void controller.start(selected, { kind: 'tag', name: name.trim() });
            }}
          >
            <Tag size={14} />
            <input
              aria-label={t('为所选笔记添加标签', 'Add a tag to selected notes')}
              placeholder={t('添加标签…', 'Add a tag…')}
              value={name}
              maxLength={100}
              disabled={busy}
              onChange={(event) => setName(event.target.value)}
            />
            <button
              className="icon-button"
              aria-label={t('应用标签', 'Apply tag')}
              type="submit"
              disabled={busy || !name.trim()}
            >
              <Check size={15} />
            </button>
          </form>
          <PopupMenu
            label={t('更改所选笔记整理状态', 'Change organization status')}
            trigger={
              <>
                {t('整理状态', 'Organization status')}
                <ChevronDown size={13} />
              </>
            }
            disabled={busy}
            placement="bottom-start"
            items={[
              {
                id: 'pending',
                label: t('收件箱 · 待整理', 'Inbox · Unprocessed'),
                onSelect: () =>
                  void controller.start(selected, { kind: 'inbox', target: 'pending' }),
              },
              {
                id: 'snoozed',
                label: t('稍后整理', 'Later'),
                onSelect: () =>
                  void controller.start(selected, { kind: 'inbox', target: 'snoozed' }),
              },
              {
                id: 'processed',
                label: t('已整理', 'Organized'),
                onSelect: () =>
                  void controller.start(selected, { kind: 'inbox', target: 'processed' }),
              },
            ]}
          />
          {availableProperties.length ? (
            <details
              className="bulk-property-editor"
              open={propertyExpanded}
              onToggle={(event) => setPropertyExpanded(event.currentTarget.open)}
            >
              <summary
                className="bulk-property-heading"
                onKeyDown={(event) => {
                  if (
                    event.key !== 'Enter' ||
                    event.nativeEvent.isComposing ||
                    event.keyCode === 229
                  )
                    return;
                  event.preventDefault();
                  setPropertyExpanded((value) => !value);
                }}
              >
                <strong>{t('批量属性', 'Batch properties')}</strong>
                <small>
                  {t(
                    '只修改 YAML，每篇笔记仍保留独立历史。留空会移除该属性。',
                    'YAML only; each note keeps its own history. Empty removes the property.',
                  )}
                </small>
                <ChevronDown size={14} className="bulk-property-chevron" aria-hidden="true" />
              </summary>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  void applyProperty(selected);
                }}
              >
                <Select
                  aria-label={t('选择批量属性', 'Choose a batch property')}
                  value={propertyField?.key ?? ''}
                  disabled={propertyBusy}
                  onValueChange={setPropertyFieldKey}
                >
                  {availableProperties.map((field) => (
                    <option key={field.key} value={field.key}>
                      {t(field.labelZh, field.labelEn)}
                    </option>
                  ))}
                </Select>
                {propertyField?.kind === 'relation' ? (
                  <div
                    className="bulk-relation-editor"
                    role="group"
                    aria-label={t('批量关联设置', 'Batch relation settings')}
                  >
                    <Select
                      aria-label={t('批量关联方式', 'Batch relation mode')}
                      value={relationMode}
                      disabled={propertyBusy}
                      onValueChange={(value) =>
                        setRelationMode(value as 'add' | 'replace' | 'clear')
                      }
                    >
                      <option value="add">{t('添加关联', 'Add links')}</option>
                      <option value="replace">{t('替换全部关联', 'Replace all links')}</option>
                      <option value="clear">{t('清空关联', 'Clear links')}</option>
                    </Select>
                    {relationMode !== 'clear' ? (
                      <>
                        <div
                          className="bulk-relation-targets"
                          aria-label={t('批量关联目标', 'Batch relation targets')}
                        >
                          {relationTargets.length ? (
                            relationTargets.map((target) => (
                              <span className="bulk-relation-chip" key={target.id}>
                                <Link2 size={12} aria-hidden="true" />
                                <span>{target.title}</span>
                                <button
                                  type="button"
                                  className="icon-button"
                                  aria-label={t(
                                    `移除关联目标 ${target.title}`,
                                    `Remove link target ${target.title}`,
                                  )}
                                  disabled={propertyBusy}
                                  onClick={() =>
                                    setRelationTargets((items) =>
                                      items.filter((item) => item.id !== target.id),
                                    )
                                  }
                                >
                                  <X size={12} aria-hidden="true" />
                                </button>
                              </span>
                            ))
                          ) : (
                            <span className="bulk-relation-empty">
                              {t('尚未选择关联目标', 'No link targets selected')}
                            </span>
                          )}
                        </div>
                        <button
                          type="button"
                          className="secondary-button"
                          disabled={propertyBusy || relationTargets.length >= 12}
                          onClick={() => setRelationPickerOpen(true)}
                          onKeyDown={(event) => {
                            if (
                              event.key !== 'Enter' ||
                              event.nativeEvent.isComposing ||
                              event.keyCode === 229
                            )
                              return;
                            event.preventDefault();
                            setRelationPickerOpen(true);
                          }}
                        >
                          <Plus size={13} aria-hidden="true" />
                          {relationTargets.length >= 12
                            ? t('已达 12 篇上限', '12 links maximum')
                            : t('添加关联目标', 'Add link target')}
                        </button>
                        <p className="bulk-property-hint">
                          {t(
                            '添加会保留现有目标；替换会重写该字段。每篇笔记独立保存并保留历史。',
                            'Add keeps existing targets; replace rewrites the field. Every note saves independently with history.',
                          )}
                        </p>
                      </>
                    ) : (
                      <p className="bulk-property-hint">
                        {t(
                          '将从此类型的所选笔记中移除该关联字段。',
                          'The relation field will be removed from each selected note of this type.',
                        )}
                      </p>
                    )}
                  </div>
                ) : propertyField?.kind === 'select' ? (
                  <Select
                    aria-label={t('批量属性值', 'Batch property value')}
                    value={propertyValue}
                    disabled={propertyBusy}
                    onValueChange={setPropertyValue}
                  >
                    <option value="">{t('留空并移除', 'Empty and remove')}</option>
                    {(propertyField.options ?? []).map((option) => (
                      <option key={option.value} value={option.value}>
                        {t(option.zh, option.en)}
                      </option>
                    ))}
                  </Select>
                ) : (
                  <input
                    aria-label={t('批量属性值', 'Batch property value')}
                    type={
                      propertyField?.kind === 'date'
                        ? 'date'
                        : propertyField?.kind === 'number'
                          ? 'number'
                          : 'text'
                    }
                    value={propertyValue}
                    disabled={propertyBusy}
                    placeholder={t('输入值或留空', 'Enter a value or leave empty')}
                    onChange={(event) => setPropertyValue(event.target.value)}
                  />
                )}
                <button
                  className="secondary-button"
                  type="submit"
                  disabled={
                    propertyBusy ||
                    busy ||
                    !selected.length ||
                    !propertyField ||
                    Boolean(relationNeedsTarget && !relationTargets.length)
                  }
                >
                  {propertyBusy ? t('正在写入…', 'Writing…') : t('应用到所选', 'Apply to selected')}
                </button>
              </form>
              {relationPickerOpen && propertyField?.kind === 'relation' && (
                <ReferencePicker
                  identity={identity}
                  mode="note"
                  excludeNoteIds={relationExcludedIds}
                  objectTypes={relationObjectTypes}
                  onClose={() => setRelationPickerOpen(false)}
                  onSelect={(target, title) => {
                    setRelationTargets((items) =>
                      items.some((item) => item.id === target.target_id)
                        ? items
                        : [...items, { kind: 'note', id: target.target_id, title }],
                    );
                    setRelationPickerOpen(false);
                  }}
                />
              )}
              {propertyResults && (
                <div className="bulk-property-results">
                  <p role="status">
                    {t(
                      `${propertyResults.filter((item) => item.status === 'done').length} / ${propertyResults.length} 篇已写入`,
                      `${propertyResults.filter((item) => item.status === 'done').length} / ${propertyResults.length} written`,
                    )}
                  </p>
                  <details open={propertyResults.some((item) => item.status === 'failed')}>
                    <summary>{t('逐项查看属性结果', 'View each property result')}</summary>
                    <ul className="bulk-results">
                      {propertyResults.map((item) => (
                        <li key={item.id}>
                          <strong>{item.title || t('未命名笔记', 'Untitled note')}</strong>
                          <span>
                            {item.status === 'done'
                              ? t('已写入', 'Written')
                              : item.status === 'failed'
                                ? t('失败', 'Failed')
                                : t('等待中', 'Waiting')}
                          </span>
                          {item.error && <small>{item.error}</small>}
                        </li>
                      ))}
                    </ul>
                  </details>
                  {propertyResults.some((item) => item.status === 'failed') && (
                    <button
                      className="text-button"
                      type="button"
                      disabled={propertyBusy}
                      onClick={() =>
                        void applyProperty(
                          propertyResults
                            .filter((item) => item.status === 'failed')
                            .map(({ id, title }) => ({ id, title })),
                          true,
                        )
                      }
                    >
                      {t('重试失败项', 'Retry failed items')}
                    </button>
                  )}
                </div>
              )}
            </details>
          ) : (
            <p className="bulk-property-hint">
              {t(
                '先在笔记列表按对象类型筛选，才能批量编辑该类型的属性。',
                'Filter the note list by object type first to edit that type’s properties in bulk.',
              )}
            </p>
          )}
        </>
      )}
      {record && (
        <>
          <p className="bulk-summary" role="status">
            {t(
              `${done} / ${record.items.length} 篇已完成`,
              `${done} / ${record.items.length} completed`,
            )}
            {busy ? t(' · 正在整理…', ' · Organizing…') : ''}
          </p>
          <details open={record.items.some((item) => item.error)}>
            <summary>{t('逐项查看', 'View each result')}</summary>
            <ul className="bulk-results">
              {record.items.map((item) => (
                <li key={item.id}>
                  <strong>{item.title || t('未命名笔记', 'Untitled note')}</strong>
                  <span>
                    {item.status === 'done'
                      ? t('已完成', 'Completed')
                      : item.status === 'failed'
                        ? t('未完成', 'Failed')
                        : item.status === 'pending'
                          ? t('待确认', 'Awaiting confirmation')
                          : t('待处理', 'Waiting')}
                  </span>
                  {item.error && <small>{item.error}</small>}
                </li>
              ))}
            </ul>
          </details>
          {unresolved ? (
            <button
              className="text-button"
              disabled={busy}
              onClick={() => void controller.resume()}
            >
              {t('继续并确认结果', 'Continue and confirm results')}
            </button>
          ) : (
            <button
              className="text-button"
              disabled={busy}
              onClick={() => void controller.dismiss().then(onClear)}
            >
              {t('完成', 'Done')}
            </button>
          )}
        </>
      )}
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
