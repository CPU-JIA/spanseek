import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import {
  BookOpenText,
  Building2,
  CalendarDays,
  ChevronDown,
  Clapperboard,
  FileText,
  FolderKanban,
  Lightbulb,
  Link2,
  MapPin,
  Plus,
  Sparkles,
  UserRound,
  UsersRound,
  X,
} from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import type { ObjectFieldDefinition, ObjectTypeState } from './object-types';
import {
  objectTypeCatalog,
  objectTypeDefinition,
  readFrontmatter,
  readObjectRelations,
  relationAllowedTypeTokens,
  setFrontmatterRelations,
  setFrontmatterValue,
  type ObjectRelationValue,
  type ObjectTypeDefinition,
} from './object-types';
import { useI18n } from '../i18n';
import { PopupMenu } from '../ui/PopupMenu';
import { Select } from '../ui/Select';
import { ReferencePicker } from '../editor/reference-picker';
import type { DraftController } from '../editor/draft-controller';

export function ObjectTypeIcon({
  type,
  size = 16,
}: {
  type: ObjectTypeDefinition['icon'];
  size?: number;
}) {
  const props = { size, 'aria-hidden': true as const };
  switch (type) {
    case 'character':
      return <UsersRound {...props} />;
    case 'person':
      return <UserRound {...props} />;
    case 'book':
      return <BookOpenText {...props} />;
    case 'film':
      return <Clapperboard {...props} />;
    case 'place':
      return <MapPin {...props} />;
    case 'event':
      return <CalendarDays {...props} />;
    case 'project':
      return <FolderKanban {...props} />;
    case 'idea':
      return <Lightbulb {...props} />;
    case 'organization':
      return <Building2 {...props} />;
    case 'meeting':
      return <UsersRound {...props} />;
    case 'custom':
      return <Sparkles {...props} />;
    default:
      return <FileText {...props} />;
  }
}

export function objectTypeMenuItems(
  t: (zh: string, en: string) => string,
  onSelect: (id: string) => void,
  current?: string | null,
  customTypes: readonly ObjectTypeDefinition[] = [],
): Array<{
  id: string;
  label: string;
  detail: string;
  icon: ReactNode;
  checked: boolean;
  radio: boolean;
  groupLabel?: string;
  onSelect: () => void;
}> {
  return objectTypeCatalog(customTypes).map((type, index) => ({
    id: type.id,
    label: t(type.labelZh, type.labelEn),
    detail: t(type.descriptionZh, type.descriptionEn),
    icon: <ObjectTypeIcon type={type.icon} />,
    checked: current === type.id,
    radio: true,
    groupLabel: index === 0 ? t('对象类型', 'Object type') : undefined,
    onSelect: () => onSelect(type.id),
  }));
}

export function ObjectTypePicker({
  state,
  disabled,
  onChange,
  customTypes = [],
}: {
  state: ObjectTypeState;
  disabled?: boolean;
  onChange: (id: string) => void;
  customTypes?: readonly ObjectTypeDefinition[];
}) {
  const { t } = useI18n();
  const { definition } = state;
  const items = objectTypeMenuItems(t, onChange, definition.id, customTypes);
  const knownCustom = customTypes.some((type) => type.id === definition.id);
  if (state.custom && state.rawType && !knownCustom) {
    items.unshift({
      id: definition.id,
      label: state.rawType,
      detail: t('当前自定义对象类型', 'Current custom object type'),
      icon: <ObjectTypeIcon type="custom" />,
      checked: true,
      radio: true,
      groupLabel: t('当前类型', 'Current type'),
      onSelect: () => onChange(state.rawType!),
    });
  }
  return (
    <PopupMenu
      label={t('更换对象类型', 'Change object type')}
      className="object-type-picker"
      placement="bottom-start"
      disabled={disabled}
      trigger={
        <>
          <span className={`object-type-mark tone-${definition.tone}`}>
            <ObjectTypeIcon type={definition.icon} size={18} />
          </span>
          <span className="object-type-picker-copy">
            <span>{t(definition.labelZh, definition.labelEn)}</span>
            <small>{t('对象类型', 'Object type')}</small>
          </span>
          <ChevronDown size={14} aria-hidden="true" />
        </>
      }
      items={items}
    />
  );
}

function TypePropertyInput({
  field,
  value,
  disabled,
  onCommit,
}: {
  field: ObjectFieldDefinition;
  value: string;
  disabled?: boolean;
  onCommit: (value: string) => void;
}) {
  const { t, language } = useI18n();
  const [draft, setDraft] = useState(value);
  const committed = useRef(value);
  const skipCommit = useRef(false);
  useEffect(() => {
    committed.current = value;
    setDraft(value);
  }, [value, field.key]);
  if (field.kind === 'select') {
    const options = [
      {
        value: '',
        label: t('未设置', 'Not set'),
      },
      ...(field.options ?? []).map((option) => ({
        value: option.value,
        label: t(option.zh, option.en),
      })),
    ];
    return (
      <Select
        className="note-type-value-select"
        aria-label={`${t(field.labelZh, field.labelEn)} · ${t('对象属性', 'Object property')}`}
        disabled={disabled}
        value={value}
        options={options}
        onValueChange={onCommit}
      />
    );
  }
  function commit() {
    if (skipCommit.current) {
      skipCommit.current = false;
      return;
    }
    if (draft === committed.current) return;
    committed.current = draft;
    onCommit(draft);
  }
  const common = {
    className: 'note-type-value-input',
    'aria-label': `${t(field.labelZh, field.labelEn)} · ${t('对象属性', 'Object property')}`,
    disabled,
    value: draft,
    placeholder: t(field.placeholderZh ?? '未设置', field.placeholderEn ?? 'Not set'),
    autoComplete: 'off',
    spellCheck: false,
    onChange: (event: React.ChangeEvent<HTMLInputElement>) => setDraft(event.target.value),
    onBlur: commit,
    onKeyDown: (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.nativeEvent.isComposing || event.keyCode === 229) return;
      if (event.key === 'Enter') {
        event.preventDefault();
        commit();
        event.currentTarget.blur();
      } else if (event.key === 'Escape') {
        event.preventDefault();
        skipCommit.current = true;
        setDraft(committed.current);
        event.currentTarget.blur();
      }
    },
  };
  return (
    <input
      {...common}
      type={field.kind === 'date' ? 'date' : field.kind === 'number' ? 'number' : 'text'}
      inputMode={field.kind === 'number' ? 'decimal' : undefined}
      step={field.kind === 'number' ? 'any' : undefined}
      lang={language}
    />
  );
}

function RelationPropertyInput({
  field,
  values,
  error,
  disabled,
  identity,
  currentNoteId,
  customTypes,
  onOpenNote,
  onChange,
}: {
  field: ObjectFieldDefinition;
  values: readonly ObjectRelationValue[];
  error: string | null;
  disabled?: boolean;
  identity: WorkspaceIdentity;
  currentNoteId: string;
  customTypes: readonly ObjectTypeDefinition[];
  onOpenNote?: (id: string) => void;
  onChange: (values: ObjectRelationValue[]) => void;
}) {
  const { t } = useI18n();
  const [pickerOpen, setPickerOpen] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const label = t(field.labelZh, field.labelEn);
  const allowedNoteTypes = useMemo(
    () => relationAllowedTypeTokens(field, customTypes),
    [customTypes, field],
  );
  const atLimit = values.length >= 12;
  function add(target: ObjectRelationValue) {
    setOperationError(null);
    if (atLimit) {
      setOperationError(
        t('一个关联字段最多保存 12 篇笔记。', 'A relation field can contain up to 12 notes.'),
      );
      return;
    }
    if (values.some((value) => value.id === target.id)) return;
    onChange([...values, target]);
  }
  function remove(id: string) {
    setOperationError(null);
    onChange(values.filter((value) => value.id !== id));
  }
  return (
    <div className="note-relation-field">
      {values.length > 0 ? (
        <ul className="note-relation-list" aria-label={label}>
          {values.map((value) => (
            <li className="note-relation-chip" key={value.id}>
              <button
                type="button"
                className="note-relation-open"
                disabled={!onOpenNote}
                title={value.id}
                onClick={() => onOpenNote?.(value.id)}
              >
                <Link2 size={12} aria-hidden="true" />
                <span>{value.title}</span>
              </button>
              <button
                type="button"
                className="note-relation-remove"
                aria-label={t(`移除关联 ${value.title}`, `Remove link ${value.title}`)}
                disabled={disabled}
                onClick={() => remove(value.id)}
              >
                <X size={11} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <span className="note-property-empty">{t('尚未关联', 'No linked notes')}</span>
      )}
      <button
        type="button"
        className="note-relation-add"
        aria-label={t(`添加${label}关联`, `Add a ${field.labelEn.toLowerCase()} link`)}
        disabled={disabled || atLimit}
        onClick={() => {
          setOperationError(null);
          setPickerOpen(true);
        }}
      >
        <Plus size={13} aria-hidden="true" />
        {atLimit ? t('已达 12 篇上限', '12 links maximum') : t('添加关联', 'Add link')}
      </button>
      {(error || operationError) && (
        <p className="note-property-error" role="alert">
          {operationError ?? error}
        </p>
      )}
      {pickerOpen && (
        <ReferencePicker
          identity={identity}
          mode="note"
          excludeNoteIds={[currentNoteId, ...values.map((value) => value.id)]}
          objectTypes={allowedNoteTypes}
          onClose={() => setPickerOpen(false)}
          onSelect={(target, title) => {
            add({ kind: 'note', id: target.target_id, title });
            setPickerOpen(false);
          }}
        />
      )}
    </div>
  );
}

export function ObjectTypeProperties({
  controller,
  identity,
  onChanged,
  disabled,
  customTypes = [],
  onOpenNote,
}: {
  controller: DraftController;
  identity: WorkspaceIdentity;
  onChanged: () => void;
  disabled?: boolean;
  customTypes?: readonly ObjectTypeDefinition[];
  onOpenNote?: (id: string) => void;
}) {
  const { t } = useI18n();
  const { record } = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const frontmatter = readFrontmatter(record.document);
  const rawType = frontmatter.values.type ?? frontmatter.values.object_type;
  const typeValue = rawType === null || rawType === undefined ? '' : String(rawType).trim();
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setError(null), [record.document, typeValue]);
  if (!typeValue && !frontmatter.error) return null;
  const definition = objectTypeDefinition(typeValue || 'note', customTypes);
  function commit(key: string, value: string) {
    setError(null);
    try {
      controller.update(record.title, setFrontmatterValue(record.document, key, value));
      onChanged();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }
  function commitRelations(key: string, value: ObjectRelationValue[]) {
    setError(null);
    try {
      controller.update(record.title, setFrontmatterRelations(record.document, key, value));
      onChanged();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }
  return (
    <div className="note-type-properties" aria-label={t('对象属性', 'Object properties')}>
      {frontmatter.error ? (
        <p className="note-property-error" role="alert">
          {t(
            `YAML 属性暂时无法读取：${frontmatter.error}`,
            `YAML properties cannot be read: ${frontmatter.error}`,
          )}
        </p>
      ) : (
        definition.fields.map((field) => {
          const relations =
            field.kind === 'relation' ? readObjectRelations(record.document, field.key) : null;
          return (
            <div className="note-property-row" key={field.key}>
              <span className="note-property-label">{t(field.labelZh, field.labelEn)}</span>
              <div className="note-property-value">
                {field.kind === 'relation' && relations ? (
                  <RelationPropertyInput
                    field={field}
                    values={relations.values}
                    error={relations.error}
                    disabled={disabled}
                    identity={identity}
                    currentNoteId={record.base.id}
                    customTypes={customTypes}
                    onOpenNote={onOpenNote}
                    onChange={(values) => commitRelations(field.key, values)}
                  />
                ) : (
                  <TypePropertyInput
                    field={field}
                    value={
                      frontmatter.values[field.key] === null ||
                      frontmatter.values[field.key] === undefined
                        ? ''
                        : String(frontmatter.values[field.key])
                    }
                    disabled={disabled}
                    onCommit={(value) => commit(field.key, value)}
                  />
                )}
              </div>
            </div>
          );
        })
      )}
      {error && (
        <p className="note-property-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
