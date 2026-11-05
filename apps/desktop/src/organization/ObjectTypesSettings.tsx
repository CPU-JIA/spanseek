import { useEffect, useId, useMemo, useState } from 'react';
import { Check, Plus, Sparkles, Trash2, X } from 'lucide-react';
import type { AppPreferences, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { ObjectTypeIcon } from './ObjectTypeControls';
import {
  OBJECT_TYPES,
  readObjectTypeRegistry,
  writeObjectTypeRegistry,
  type ObjectFieldDefinition,
  type ObjectFieldKind,
  type ObjectTypeDefinition,
} from './object-types';

type EditorState = {
  mode: 'new' | 'edit';
  originalId: string | null;
  value: ObjectTypeDefinition;
  aliasesText: string;
};

const TONES: Array<ObjectTypeDefinition['tone']> = [
  'neutral',
  'amber',
  'blue',
  'rose',
  'violet',
  'teal',
  'green',
];

function optional(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function nextTypeId(types: readonly ObjectTypeDefinition[]): string {
  const used = new Set(types.map((type) => type.id.toLocaleLowerCase('en-US')));
  let index = 1;
  while (used.has(`custom-${index}`)) index += 1;
  return `custom-${index}`;
}

function newType(types: readonly ObjectTypeDefinition[]): ObjectTypeDefinition {
  return {
    id: nextTypeId(types),
    labelZh: '新对象',
    labelEn: 'New object',
    pluralZh: '新对象',
    pluralEn: 'New objects',
    descriptionZh: '说明这类对象用于记录什么，以及它与其他对象的区别。',
    descriptionEn: 'Explain what this object type captures and how it differs from the others.',
    icon: 'custom',
    tone: 'neutral',
    aliases: [],
    fields: [
      {
        key: 'summary',
        labelZh: '摘要',
        labelEn: 'Summary',
        kind: 'text',
        placeholderZh: '用一句话说明它是什么',
        placeholderEn: 'Describe this object in one line',
      },
    ],
    sections: [{ zh: '概览', en: 'Overview' }],
  };
}

function cleanField(field: ObjectFieldDefinition): ObjectFieldDefinition {
  const clean: ObjectFieldDefinition = {
    key: field.key.trim(),
    labelZh: field.labelZh.trim(),
    labelEn: field.labelEn.trim(),
    kind: field.kind,
  };
  const placeholderZh = optional(field.placeholderZh);
  const placeholderEn = optional(field.placeholderEn);
  const defaultValue = optional(field.defaultValue);
  if (placeholderZh) clean.placeholderZh = placeholderZh;
  if (placeholderEn) clean.placeholderEn = placeholderEn;
  if (field.kind !== 'relation' && defaultValue) clean.defaultValue = defaultValue;
  if (field.kind === 'relation' && field.allowedTypeIds?.length) {
    clean.allowedTypeIds = [
      ...new Set(
        field.allowedTypeIds
          .map((value) => value.trim().toLocaleLowerCase('en-US'))
          .filter(Boolean),
      ),
    ];
  }
  if (field.kind === 'select') {
    clean.options = (field.options ?? []).map((option) => ({
      value: option.value.trim(),
      zh: option.zh.trim(),
      en: option.en.trim(),
    }));
  }
  return clean;
}

function cleanType(value: ObjectTypeDefinition, aliasesText: string): ObjectTypeDefinition {
  return {
    ...value,
    id: value.id.trim().toLocaleLowerCase('en-US'),
    labelZh: value.labelZh.trim(),
    labelEn: value.labelEn.trim(),
    pluralZh: value.pluralZh.trim(),
    pluralEn: value.pluralEn.trim(),
    descriptionZh: value.descriptionZh.trim(),
    descriptionEn: value.descriptionEn.trim(),
    aliases: aliasesText
      .split(/[,\n，]+/)
      .map((alias) => alias.trim())
      .filter(Boolean),
    fields: value.fields.map(cleanField),
    sections: value.sections.map((section) => ({
      zh: section.zh.trim(),
      en: section.en.trim(),
    })),
  };
}

function cloneType(type: ObjectTypeDefinition): ObjectTypeDefinition {
  return structuredClone(type);
}

export function ObjectTypesSettings({
  registryJson,
  identity,
  update,
  busy,
}: {
  registryJson: string;
  identity: WorkspaceIdentity | null;
  update: (change: Partial<AppPreferences>) => Promise<boolean>;
  busy: boolean;
}) {
  const { t } = useI18n();
  const baseId = useId();
  const loaded = useMemo(
    () => readObjectTypeRegistry(registryJson, identity?.workspace_id),
    [identity?.workspace_id, registryJson],
  );
  const [types, setTypes] = useState<ObjectTypeDefinition[]>(loaded.types);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setTypes(loaded.types);
    setEditor(null);
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }, [loaded.types, registryJson, identity?.workspace_id]);

  const savedSignature = JSON.stringify(loaded.types);
  const dirty = JSON.stringify(types) !== savedSignature;
  const disabled = busy || saving || !identity || Boolean(loaded.error);

  function beginNew() {
    setEditor({
      mode: 'new',
      originalId: null,
      value: newType(types),
      aliasesText: '',
    });
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }

  function beginEdit(type: ObjectTypeDefinition) {
    setEditor({
      mode: 'edit',
      originalId: type.id,
      value: cloneType(type),
      aliasesText: type.aliases.join('\n'),
    });
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }

  function patchType(patch: Partial<ObjectTypeDefinition>) {
    setEditor((current) =>
      current ? { ...current, value: { ...current.value, ...patch } } : current,
    );
  }

  function patchField(index: number, patch: Partial<ObjectFieldDefinition>) {
    setEditor((current) => {
      if (!current) return current;
      return {
        ...current,
        value: {
          ...current.value,
          fields: current.value.fields.map((field, fieldIndex) =>
            fieldIndex === index ? { ...field, ...patch } : field,
          ),
        },
      };
    });
  }

  function patchOption(
    fieldIndex: number,
    optionIndex: number,
    patch: Partial<NonNullable<ObjectFieldDefinition['options']>[number]>,
  ) {
    setEditor((current) => {
      if (!current) return current;
      return {
        ...current,
        value: {
          ...current.value,
          fields: current.value.fields.map((field, index) => {
            if (index !== fieldIndex) return field;
            return {
              ...field,
              options: (field.options ?? []).map((option, index) =>
                index === optionIndex ? { ...option, ...patch } : option,
              ),
            };
          }),
        },
      };
    });
  }

  function addField() {
    setEditor((current) => {
      if (!current || current.value.fields.length >= 20) return current;
      const index = current.value.fields.length + 1;
      return {
        ...current,
        value: {
          ...current.value,
          fields: [
            ...current.value.fields,
            {
              key: `field_${index}`,
              labelZh: `字段 ${index}`,
              labelEn: `Field ${index}`,
              kind: 'text',
            },
          ],
        },
      };
    });
  }

  function removeField(index: number) {
    setEditor((current) =>
      current
        ? {
            ...current,
            value: {
              ...current.value,
              fields: current.value.fields.filter((_, fieldIndex) => fieldIndex !== index),
            },
          }
        : current,
    );
  }

  function addOption(fieldIndex: number) {
    const field = editor?.value.fields[fieldIndex];
    if (!field) return;
    const used = new Set((field.options ?? []).map((option) => option.value));
    let index = 1;
    while (used.has(`option-${index}`)) index += 1;
    patchField(fieldIndex, {
      options: [
        ...(field.options ?? []),
        { value: `option-${index}`, zh: `选项 ${index}`, en: `Option ${index}` },
      ],
    });
  }

  function removeOption(fieldIndex: number, optionIndex: number) {
    const field = editor?.value.fields[fieldIndex];
    if (!field) return;
    patchField(fieldIndex, {
      options: (field.options ?? []).filter((_, index) => index !== optionIndex),
    });
  }

  function addSection() {
    setEditor((current) => {
      if (!current || current.value.sections.length >= 8) return current;
      const index = current.value.sections.length + 1;
      return {
        ...current,
        value: {
          ...current.value,
          sections: [...current.value.sections, { zh: `分节 ${index}`, en: `Section ${index}` }],
        },
      };
    });
  }

  function acceptEditor() {
    if (!editor || !identity) return;
    setEditorError(null);
    try {
      const candidate = cleanType(editor.value, editor.aliasesText);
      const nextTypes =
        editor.mode === 'new'
          ? [...types, candidate]
          : types.map((type) => (type.id === editor.originalId ? candidate : type));
      const nextJson = writeObjectTypeRegistry(registryJson, identity.workspace_id, nextTypes);
      const checked = readObjectTypeRegistry(nextJson, identity.workspace_id);
      if (checked.error) throw new Error(checked.error);
      setTypes(checked.types);
      setEditor(null);
      setMessage(
        t(
          '对象类型已加入草稿；保存设置后才会跨会话生效。',
          'Object type added to the draft. Save settings to keep it across sessions.',
        ),
      );
    } catch (failure) {
      setEditorError(failure instanceof Error ? failure.message : String(failure));
    }
  }

  function requestDelete(type: ObjectTypeDefinition) {
    setConfirmDeleteId(type.id);
    setEditor(null);
    setMessage(null);
  }

  function deleteType(type: ObjectTypeDefinition) {
    setTypes((current) => current.filter((candidate) => candidate.id !== type.id));
    setConfirmDeleteId(null);
    setMessage(
      t(
        '对象类型已从草稿中移除；已有笔记的 YAML 属性不会被删除。保存设置后生效。',
        'Object type removed from the draft. Existing notes keep their YAML values. Save settings to apply.',
      ),
    );
  }

  async function saveRegistry() {
    if (!identity || disabled) return;
    setSaving(true);
    setMessage(null);
    try {
      const nextJson = writeObjectTypeRegistry(registryJson, identity.workspace_id, types);
      const saved = await update({ object_type_registry: nextJson });
      setMessage(
        saved
          ? t('对象类型设置已保存。', 'Object type settings saved.')
          : t('对象类型设置尚未保存，请重试。', 'Object type settings were not saved. Try again.'),
      );
    } catch (failure) {
      setMessage(
        t('无法保存对象类型：', 'Could not save object types: ') +
          (failure instanceof Error ? failure.message : String(failure)),
      );
    } finally {
      setSaving(false);
    }
  }

  if (!identity) {
    return (
      <p className="settings-description">
        {t(
          '打开知识库后即可管理自定义对象类型。',
          'Open a workspace to manage custom object types.',
        )}
      </p>
    );
  }

  return (
    <div className="settings-object-types">
      <h3>{t('内置对象类型', 'Built-in object types')}</h3>
      <p className="settings-description">
        {t(
          '这些类型随应用提供，所有知识库共享同一套语义。自定义类型只属于当前知识库。',
          'These types ship with the application. Custom types belong only to the current workspace.',
        )}
      </p>
      <ul
        className="settings-object-type-builtins"
        aria-label={t('内置对象类型', 'Built-in object types')}
      >
        {OBJECT_TYPES.map((type) => (
          <li key={type.id}>
            <span className={`settings-object-type-icon tone-${type.tone}`}>
              <ObjectTypeIcon type={type.icon} size={17} />
            </span>
            <span>
              <strong>{t(type.labelZh, type.labelEn)}</strong>
              <small>{t(type.descriptionZh, type.descriptionEn)}</small>
            </span>
          </li>
        ))}
      </ul>

      <div className="settings-object-types-heading">
        <div>
          <h3>{t('自定义对象类型', 'Custom object types')}</h3>
          <p className="settings-description">
            {t(
              '用专门的结构记录人物、地点、项目之外的概念。每种类型可以拥有自己的属性、选项与正文分节。',
              'Give concepts beyond people, places and projects their own properties, choices and note sections.',
            )}
          </p>
        </div>
        <button
          type="button"
          className="secondary-button"
          disabled={disabled || types.length >= 32 || Boolean(editor)}
          onClick={beginNew}
        >
          <Plus size={15} aria-hidden="true" />
          {t('新建类型', 'New type')}
        </button>
      </div>

      {loaded.error && (
        <p className="settings-object-type-error" role="alert">
          {t('无法读取对象类型设置：', 'Could not read object type settings: ')}
          {loaded.error}
        </p>
      )}

      {types.length === 0 ? (
        <div className="settings-object-type-empty">
          <Sparkles size={22} aria-hidden="true" />
          <strong>{t('还没有自定义类型', 'No custom types yet')}</strong>
          <span>
            {t(
              '从“新建类型”开始定义字段与正文模板。',
              'Start with New type to define properties and a section template.',
            )}
          </span>
        </div>
      ) : (
        <ul className="settings-object-type-list">
          {types.map((type) => (
            <li key={type.id} className={`tone-${type.tone}`}>
              <span className="settings-object-type-icon">
                <ObjectTypeIcon type={type.icon} size={18} />
              </span>
              <div className="settings-object-type-copy">
                <strong>{t(type.labelZh, type.labelEn)}</strong>
                <span>{type.id}</span>
                <small>
                  {t(
                    `${type.fields.length} 个属性 · ${type.sections.length} 个正文分节`,
                    `${type.fields.length} properties · ${type.sections.length} sections`,
                  )}
                </small>
              </div>
              {confirmDeleteId === type.id ? (
                <div className="settings-object-type-delete" role="alert">
                  <span>
                    {t(
                      '删除类型定义？已有笔记和 YAML 属性会保留。',
                      'Delete this type? Existing notes and YAML values stay intact.',
                    )}
                  </span>
                  <button
                    type="button"
                    className="text-button danger"
                    disabled={disabled}
                    onClick={() => deleteType(type)}
                  >
                    {t('确认删除', 'Delete')}
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => setConfirmDeleteId(null)}
                  >
                    {t('取消', 'Cancel')}
                  </button>
                </div>
              ) : (
                <div className="settings-object-type-actions">
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={disabled || Boolean(editor)}
                    onClick={() => beginEdit(type)}
                  >
                    {t('编辑', 'Edit')}
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t(`删除${type.labelZh}`, `Delete ${type.labelEn}`)}
                    title={t('删除类型', 'Delete type')}
                    disabled={disabled || Boolean(editor)}
                    onClick={() => requestDelete(type)}
                  >
                    <Trash2 size={16} aria-hidden="true" />
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {editor && (
        <section
          className="settings-object-type-editor"
          aria-labelledby={`${baseId}-editor`}
          aria-busy={disabled}
        >
          <header>
            <div>
              <span className="settings-object-type-editor-kicker">
                {editor.mode === 'new'
                  ? t('新自定义类型', 'New custom type')
                  : t('编辑自定义类型', 'Edit custom type')}
              </span>
              <h3 id={`${baseId}-editor`}>
                {editor.mode === 'new'
                  ? t('定义对象类型', 'Define object type')
                  : t(editor.value.labelZh, editor.value.labelEn)}
              </h3>
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label={t('关闭类型编辑器', 'Close type editor')}
              onClick={() => setEditor(null)}
            >
              <X size={17} aria-hidden="true" />
            </button>
          </header>

          <fieldset disabled={disabled}>
            <legend>{t('基本信息', 'Basics')}</legend>
            <div className="settings-object-type-grid">
              <label className="settings-field stacked">
                <span>{t('类型标识', 'Type ID')}</span>
                <input
                  value={editor.value.id}
                  readOnly={editor.mode === 'edit'}
                  autoFocus={editor.mode === 'new'}
                  spellCheck={false}
                  autoComplete="off"
                  pattern="[a-z][a-z0-9_-]{1,47}"
                  onChange={(event) => patchType({ id: event.target.value })}
                />
                <small>
                  {editor.mode === 'edit'
                    ? t(
                        '类型标识用于 YAML，创建后不可更改。',
                        'The YAML identifier is fixed after creation.',
                      )
                    : t(
                        '2–48 位小写字母、数字、下划线或连字符。',
                        '2–48 lowercase letters, numbers, underscores or hyphens.',
                      )}
                </small>
              </label>
              <div className="choice-field settings-field stacked">
                <span>{t('色调', 'Tone')}</span>
                <Select
                  aria-label={t('对象类型色调', 'Object type tone')}
                  value={editor.value.tone}
                  onValueChange={(value) =>
                    patchType({ tone: value as ObjectTypeDefinition['tone'] })
                  }
                >
                  {TONES.map((tone) => (
                    <option key={tone} value={tone}>
                      {tone}
                    </option>
                  ))}
                </Select>
              </div>
              <label className="settings-field stacked">
                <span>{t('中文单数名称', 'Chinese singular name')}</span>
                <input
                  value={editor.value.labelZh}
                  onChange={(event) => patchType({ labelZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked">
                <span>{t('英文单数名称', 'English singular name')}</span>
                <input
                  value={editor.value.labelEn}
                  onChange={(event) => patchType({ labelEn: event.target.value })}
                />
              </label>
              <label className="settings-field stacked">
                <span>{t('中文复数名称', 'Chinese plural name')}</span>
                <input
                  value={editor.value.pluralZh}
                  onChange={(event) => patchType({ pluralZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked">
                <span>{t('英文复数名称', 'English plural name')}</span>
                <input
                  value={editor.value.pluralEn}
                  onChange={(event) => patchType({ pluralEn: event.target.value })}
                />
              </label>
              <label className="settings-field stacked full">
                <span>{t('中文说明', 'Chinese description')}</span>
                <textarea
                  rows={2}
                  value={editor.value.descriptionZh}
                  onChange={(event) => patchType({ descriptionZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked full">
                <span>{t('英文说明', 'English description')}</span>
                <textarea
                  rows={2}
                  value={editor.value.descriptionEn}
                  onChange={(event) => patchType({ descriptionEn: event.target.value })}
                />
              </label>
              <label className="settings-field stacked full">
                <span>{t('别名', 'Aliases')}</span>
                <textarea
                  rows={3}
                  value={editor.aliasesText}
                  placeholder={t('每行一个，也可用逗号分隔', 'One per line or separated by commas')}
                  onChange={(event) =>
                    setEditor((current) =>
                      current ? { ...current, aliasesText: event.target.value } : current,
                    )
                  }
                />
                <small>
                  {t(
                    '别名用于识别旧笔记里的 type 值；不能和内置类型或其他自定义类型冲突。',
                    'Aliases recognize type values in existing notes and cannot collide with another type.',
                  )}
                </small>
              </label>
            </div>
          </fieldset>

          <fieldset disabled={disabled}>
            <legend>{t('属性', 'Properties')}</legend>
            <p className="settings-description">
              {t(
                '属性写入笔记的 YAML frontmatter。选择字段可以附上中英双语选项。',
                'Properties live in note YAML frontmatter. Choice fields can carry bilingual options.',
              )}
            </p>
            <div className="settings-object-type-fields">
              {editor.value.fields.map((field, fieldIndex) => (
                <article className="settings-object-type-field" key={fieldIndex}>
                  <header>
                    <strong>
                      {t('属性', 'Property')} {fieldIndex + 1}
                    </strong>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t(
                        `删除属性 ${fieldIndex + 1}`,
                        `Remove property ${fieldIndex + 1}`,
                      )}
                      onClick={() => removeField(fieldIndex)}
                    >
                      <Trash2 size={15} aria-hidden="true" />
                    </button>
                  </header>
                  <div className="settings-object-type-grid compact">
                    <label className="settings-field stacked">
                      <span>{t('字段键', 'Field key')}</span>
                      <input
                        value={field.key}
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(event) => patchField(fieldIndex, { key: event.target.value })}
                      />
                    </label>
                    <div className="choice-field settings-field stacked">
                      <span>{t('字段类型', 'Field type')}</span>
                      <Select
                        aria-label={t('字段类型', 'Field type')}
                        value={field.kind}
                        onValueChange={(value) => {
                          const kind = value as ObjectFieldKind;
                          patchField(fieldIndex, {
                            kind,
                            options:
                              kind === 'select'
                                ? field.options?.length
                                  ? field.options
                                  : [{ value: 'option-1', zh: '选项 1', en: 'Option 1' }]
                                : undefined,
                            defaultValue: kind === 'relation' ? undefined : field.defaultValue,
                            allowedTypeIds: kind === 'relation' ? field.allowedTypeIds : undefined,
                          });
                        }}
                      >
                        <option value="text">{t('文本', 'Text')}</option>
                        <option value="number">{t('数字', 'Number')}</option>
                        <option value="date">{t('日期', 'Date')}</option>
                        <option value="select">{t('单选', 'Choice')}</option>
                        <option value="relation">{t('关联笔记', 'Linked notes')}</option>
                      </Select>
                    </div>
                    <label className="settings-field stacked">
                      <span>{t('中文标签', 'Chinese label')}</span>
                      <input
                        value={field.labelZh}
                        onChange={(event) =>
                          patchField(fieldIndex, { labelZh: event.target.value })
                        }
                      />
                    </label>
                    <label className="settings-field stacked">
                      <span>{t('英文标签', 'English label')}</span>
                      <input
                        value={field.labelEn}
                        onChange={(event) =>
                          patchField(fieldIndex, { labelEn: event.target.value })
                        }
                      />
                    </label>
                    {field.kind !== 'relation' && (
                      <>
                        <label className="settings-field stacked">
                          <span>{t('中文占位', 'Chinese placeholder')}</span>
                          <input
                            value={field.placeholderZh ?? ''}
                            onChange={(event) =>
                              patchField(fieldIndex, { placeholderZh: event.target.value })
                            }
                          />
                        </label>
                        <label className="settings-field stacked">
                          <span>{t('英文占位', 'English placeholder')}</span>
                          <input
                            value={field.placeholderEn ?? ''}
                            onChange={(event) =>
                              patchField(fieldIndex, { placeholderEn: event.target.value })
                            }
                          />
                        </label>
                        <label className="settings-field stacked">
                          <span>{t('默认值', 'Default value')}</span>
                          <input
                            value={field.defaultValue ?? ''}
                            onChange={(event) =>
                              patchField(fieldIndex, { defaultValue: event.target.value })
                            }
                          />
                        </label>
                      </>
                    )}
                    {field.kind === 'relation' && (
                      <p className="settings-description settings-field full">
                        {t(
                          '关联字段保存稳定笔记 ID 与标题，可从当前知识库搜索并添加最多 12 篇笔记。',
                          'Relation fields keep a stable note ID and title, searchable within this workspace, with up to 12 linked notes.',
                        )}
                      </p>
                    )}
                    {field.kind === 'relation' && (
                      <fieldset className="settings-field full settings-relation-targets">
                        <legend>{t('允许关联的对象类型', 'Allowed object types')}</legend>
                        <small>
                          {t(
                            '不选择表示可关联任意类型；选择后引用选择器只展示这些类型。',
                            'Leave every type unchecked to allow all notes; otherwise the picker only shows the selected types.',
                          )}
                        </small>
                        <div className="settings-relation-target-grid">
                          {[...OBJECT_TYPES, ...types].map((type) => {
                            const selected = (field.allowedTypeIds ?? []).includes(type.id);
                            return (
                              <label className="settings-relation-target" key={type.id}>
                                <input
                                  type="checkbox"
                                  checked={selected}
                                  onChange={(event) => {
                                    const next = event.target.checked
                                      ? [...new Set([...(field.allowedTypeIds ?? []), type.id])]
                                      : (field.allowedTypeIds ?? []).filter(
                                          (candidate) => candidate !== type.id,
                                        );
                                    patchField(fieldIndex, {
                                      allowedTypeIds: next.length ? next : undefined,
                                    });
                                  }}
                                />
                                <ObjectTypeIcon type={type.icon} size={14} />
                                <span>{t(type.labelZh, type.labelEn)}</span>
                              </label>
                            );
                          })}
                        </div>
                      </fieldset>
                    )}
                  </div>
                  {field.kind === 'select' && (
                    <div className="settings-object-type-options">
                      <div className="settings-object-type-options-title">
                        <strong>{t('选项', 'Options')}</strong>
                        <button
                          type="button"
                          className="text-button"
                          onClick={() => addOption(fieldIndex)}
                        >
                          <Plus size={14} aria-hidden="true" />
                          {t('添加选项', 'Add option')}
                        </button>
                      </div>
                      {(field.options ?? []).map((option, optionIndex) => (
                        <div className="settings-object-type-option" key={optionIndex}>
                          <input
                            aria-label={t('选项值', 'Option value')}
                            value={option.value}
                            spellCheck={false}
                            onChange={(event) =>
                              patchOption(fieldIndex, optionIndex, { value: event.target.value })
                            }
                          />
                          <input
                            aria-label={t('中文选项标签', 'Chinese option label')}
                            value={option.zh}
                            onChange={(event) =>
                              patchOption(fieldIndex, optionIndex, { zh: event.target.value })
                            }
                          />
                          <input
                            aria-label={t('英文选项标签', 'English option label')}
                            value={option.en}
                            onChange={(event) =>
                              patchOption(fieldIndex, optionIndex, { en: event.target.value })
                            }
                          />
                          <button
                            type="button"
                            className="icon-button"
                            aria-label={t('删除选项', 'Remove option')}
                            onClick={() => removeOption(fieldIndex, optionIndex)}
                          >
                            <X size={14} aria-hidden="true" />
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                </article>
              ))}
            </div>
            <button
              type="button"
              className="secondary-button"
              disabled={editor.value.fields.length >= 20}
              onClick={addField}
            >
              <Plus size={15} aria-hidden="true" />
              {t('添加属性', 'Add property')}
            </button>
          </fieldset>

          <fieldset disabled={disabled}>
            <legend>{t('正文章节', 'Note sections')}</legend>
            <p className="settings-description">
              {t(
                '新建这种类型的笔记时，会按顺序生成这些二级标题。',
                'New notes of this type start with these level-two headings in order.',
              )}
            </p>
            <div className="settings-object-type-sections">
              {editor.value.sections.map((section, index) => (
                <div key={index}>
                  <input
                    aria-label={t('中文分节名称', 'Chinese section name')}
                    value={section.zh}
                    onChange={(event) =>
                      patchType({
                        sections: editor.value.sections.map((item, itemIndex) =>
                          itemIndex === index ? { ...item, zh: event.target.value } : item,
                        ),
                      })
                    }
                  />
                  <input
                    aria-label={t('英文分节名称', 'English section name')}
                    value={section.en}
                    onChange={(event) =>
                      patchType({
                        sections: editor.value.sections.map((item, itemIndex) =>
                          itemIndex === index ? { ...item, en: event.target.value } : item,
                        ),
                      })
                    }
                  />
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t('删除分节', 'Remove section')}
                    onClick={() =>
                      patchType({
                        sections: editor.value.sections.filter(
                          (_, itemIndex) => itemIndex !== index,
                        ),
                      })
                    }
                  >
                    <X size={14} aria-hidden="true" />
                  </button>
                </div>
              ))}
            </div>
            <button
              type="button"
              className="secondary-button"
              disabled={editor.value.sections.length >= 8}
              onClick={addSection}
            >
              <Plus size={15} aria-hidden="true" />
              {t('添加分节', 'Add section')}
            </button>
          </fieldset>

          {editorError && (
            <p className="settings-object-type-error" role="alert">
              {editorError}
            </p>
          )}
          <footer className="settings-object-type-editor-actions">
            <button type="button" className="secondary-button" onClick={() => setEditor(null)}>
              {t('取消', 'Cancel')}
            </button>
            <button type="button" className="primary-button" onClick={acceptEditor}>
              <Check size={15} aria-hidden="true" />
              {t('应用到草稿', 'Add to draft')}
            </button>
          </footer>
        </section>
      )}

      <footer className="settings-object-type-savebar">
        <span role="status" aria-live="polite">
          {message ??
            (dirty
              ? t('有未保存的对象类型更改。', 'You have unsaved object type changes.')
              : t('对象类型已保存。', 'Object types are saved.'))}
        </span>
        <div>
          <button
            type="button"
            className="secondary-button"
            disabled={disabled || !dirty}
            onClick={() => {
              setTypes(loaded.types);
              setEditor(null);
              setConfirmDeleteId(null);
              setMessage(t('已放弃未保存的更改。', 'Unsaved changes discarded.'));
            }}
          >
            {t('放弃更改', 'Discard')}
          </button>
          <button
            type="button"
            className="primary-button"
            disabled={disabled || !dirty}
            onClick={() => void saveRegistry()}
          >
            {saving ? t('正在保存…', 'Saving…') : t('保存对象类型', 'Save object types')}
          </button>
        </div>
      </footer>
    </div>
  );
}
