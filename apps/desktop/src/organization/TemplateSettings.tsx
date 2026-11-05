import { useEffect, useId, useMemo, useState } from 'react';
import { Check, LayoutTemplate, Plus, Trash2, X } from 'lucide-react';
import type { AppPreferences, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { ObjectTypeIcon } from './ObjectTypeControls';
import {
  objectTypeCatalog,
  objectTypeDefaultTitle,
  objectTypeDefinition,
  type ObjectTypeDefinition,
} from './object-types';
import {
  expandNoteTemplateText,
  readNoteTemplateRegistry,
  writeNoteTemplateRegistry,
  type NoteTemplateDefinition,
} from './template-registry';

type TemplateEditor = {
  mode: 'new' | 'edit';
  originalId: string | null;
  value: NoteTemplateDefinition;
};

function nextTemplateId(templates: readonly NoteTemplateDefinition[]): string {
  const used = new Set(templates.map((template) => template.id));
  let index = 1;
  while (used.has(`template-${index}`)) index += 1;
  return `template-${index}`;
}

function newTemplate(
  templates: readonly NoteTemplateDefinition[],
  language: string,
): NoteTemplateDefinition {
  const zh = language === 'zh-CN';
  return {
    id: nextTemplateId(templates),
    nameZh: zh ? '新模板' : 'New template',
    nameEn: zh ? 'New template' : 'New template',
    descriptionZh: '说明这个模板适合创建什么笔记。',
    descriptionEn: 'Describe the notes this template is meant to create.',
    typeId: null,
    titleZh: zh ? '未命名笔记 {{date}}' : 'Untitled note {{date}}',
    titleEn: zh ? 'Untitled note {{date}}' : 'Untitled note {{date}}',
    bodyZh: zh ? '## 概览\n\n' : '## Overview\n\n',
    bodyEn: zh ? '## Overview\n\n' : '## Overview\n\n',
  };
}

function cleanTemplate(value: NoteTemplateDefinition): NoteTemplateDefinition {
  return {
    id: value.id.trim().toLocaleLowerCase('en-US'),
    nameZh: value.nameZh.trim(),
    nameEn: value.nameEn.trim(),
    descriptionZh: value.descriptionZh.trim(),
    descriptionEn: value.descriptionEn.trim(),
    typeId: value.typeId?.trim() || null,
    titleZh: value.titleZh.trim(),
    titleEn: value.titleEn.trim(),
    bodyZh: value.bodyZh.trim(),
    bodyEn: value.bodyEn.trim(),
  };
}

function cloneTemplate(value: NoteTemplateDefinition): NoteTemplateDefinition {
  return structuredClone(value);
}

export function TemplateSettings({
  registryJson,
  identity,
  customTypes,
  update,
  busy,
}: {
  registryJson: string;
  identity: WorkspaceIdentity | null;
  customTypes: readonly ObjectTypeDefinition[];
  update: (change: Partial<AppPreferences>) => Promise<boolean>;
  busy: boolean;
}) {
  const { t, language } = useI18n();
  const baseId = useId();
  const loaded = useMemo(
    () => readNoteTemplateRegistry(registryJson, identity?.workspace_id),
    [identity?.workspace_id, registryJson],
  );
  const [templates, setTemplates] = useState<NoteTemplateDefinition[]>(loaded.templates);
  const [editor, setEditor] = useState<TemplateEditor | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setTemplates(loaded.templates);
    setEditor(null);
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }, [loaded.templates, registryJson, identity?.workspace_id]);

  const dirty = JSON.stringify(templates) !== JSON.stringify(loaded.templates);
  const disabled = busy || saving || !identity || Boolean(loaded.error);
  const catalog = objectTypeCatalog(customTypes);
  const preview = useMemo(() => {
    if (!editor) return null;
    const target = editor.value.typeId
      ? catalog.find((type) => type.id === editor.value.typeId)
      : null;
    if (editor.value.typeId && !target)
      return {
        ok: false as const,
        error: t(
          `对象类型“${editor.value.typeId}”已不可用。`,
          `Object type “${editor.value.typeId}” is unavailable.`,
        ),
      };
    const typeLabel = target ? t(target.labelZh, target.labelEn) : t('笔记', 'Note');
    const titleSource =
      language === 'en'
        ? editor.value.titleEn || editor.value.titleZh
        : editor.value.titleZh || editor.value.titleEn;
    return {
      ok: true as const,
      title:
        expandNoteTemplateText(titleSource, { typeLabel, now: new Date() }) ||
        (target
          ? objectTypeDefaultTitle(target.id, t, customTypes)
          : t('未命名笔记', 'Untitled note')),
      bodyChars: [
        ...(language === 'en'
          ? editor.value.bodyEn || editor.value.bodyZh
          : editor.value.bodyZh || editor.value.bodyEn),
      ].length,
    };
  }, [catalog, customTypes, editor, language, t]);

  function beginNew() {
    setEditor({
      mode: 'new',
      originalId: null,
      value: newTemplate(templates, language),
    });
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }

  function beginEdit(template: NoteTemplateDefinition) {
    setEditor({
      mode: 'edit',
      originalId: template.id,
      value: cloneTemplate(template),
    });
    setEditorError(null);
    setConfirmDeleteId(null);
    setMessage(null);
  }

  function patchTemplate(patch: Partial<NoteTemplateDefinition>) {
    setEditor((current) =>
      current ? { ...current, value: { ...current.value, ...patch } } : current,
    );
  }

  function acceptEditor() {
    if (!editor || !identity) return;
    setEditorError(null);
    try {
      const candidate = cleanTemplate(editor.value);
      const nextTemplates =
        editor.mode === 'new'
          ? [...templates, candidate]
          : templates.map((template) => (template.id === editor.originalId ? candidate : template));
      const nextJson = writeNoteTemplateRegistry(
        registryJson,
        identity.workspace_id,
        nextTemplates,
      );
      const checked = readNoteTemplateRegistry(nextJson, identity.workspace_id);
      if (checked.error) throw new Error(checked.error);
      setTemplates(checked.templates);
      setEditor(null);
      setMessage(
        t(
          '模板已加入草稿；保存设置后才会跨会话生效。',
          'Template added to the draft. Save settings to keep it across sessions.',
        ),
      );
    } catch (failure) {
      setEditorError(failure instanceof Error ? failure.message : String(failure));
    }
  }

  function deleteTemplate(template: NoteTemplateDefinition) {
    setTemplates((current) => current.filter((candidate) => candidate.id !== template.id));
    setConfirmDeleteId(null);
    setMessage(
      t(
        '模板已从草稿中移除；已经创建的笔记不受影响。保存设置后生效。',
        'Template removed from the draft. Notes already created from it are unchanged. Save settings to apply.',
      ),
    );
  }

  async function saveRegistry() {
    if (!identity || disabled) return;
    setSaving(true);
    setMessage(null);
    try {
      const nextJson = writeNoteTemplateRegistry(registryJson, identity.workspace_id, templates);
      const saved = await update({ note_template_registry: nextJson });
      setMessage(
        saved
          ? t('模板设置已保存。', 'Template settings saved.')
          : t('模板设置尚未保存，请重试。', 'Template settings were not saved. Try again.'),
      );
    } catch (failure) {
      setMessage(
        t('无法保存模板：', 'Could not save templates: ') +
          (failure instanceof Error ? failure.message : String(failure)),
      );
    } finally {
      setSaving(false);
    }
  }

  if (!identity) {
    return (
      <p className="settings-description">
        {t('打开知识库后即可管理笔记模板。', 'Open a workspace to manage note templates.')}
      </p>
    );
  }

  return (
    <div className="settings-object-types settings-template-manager">
      <div className="settings-object-types-heading">
        <div>
          <h3>{t('笔记模板', 'Note templates')}</h3>
          <p className="settings-description">
            {t(
              '把常用结构保存为模板，从命令面板或新建菜单直接开始。模板可以绑定对象类型，并使用日期、时间和类型名称占位符。',
              'Save a reusable structure and start from it in the command palette or create menu. Templates can target an object type and use date, time and type placeholders.',
            )}
          </p>
        </div>
        <button
          type="button"
          className="secondary-button"
          disabled={disabled || templates.length >= 32 || Boolean(editor)}
          onClick={beginNew}
        >
          <Plus size={15} aria-hidden="true" />
          {t('新建模板', 'New template')}
        </button>
      </div>

      {loaded.error && (
        <p className="settings-object-type-error" role="alert">
          {t('无法读取模板设置：', 'Could not read template settings: ')}
          {loaded.error}
        </p>
      )}

      {templates.length === 0 ? (
        <div className="settings-object-type-empty">
          <LayoutTemplate size={22} aria-hidden="true" />
          <strong>{t('还没有模板', 'No templates yet')}</strong>
          <span>
            {t(
              '从“新建模板”开始保存可复用的标题与正文结构。',
              'Start with New template to save a reusable title and note structure.',
            )}
          </span>
        </div>
      ) : (
        <ul className="settings-object-type-list">
          {templates.map((template) => {
            const definition = template.typeId
              ? objectTypeDefinition(template.typeId, customTypes)
              : null;
            return (
              <li
                key={template.id}
                className={definition ? `tone-${definition.tone}` : 'tone-neutral'}
              >
                <span className="settings-object-type-icon">
                  {definition ? (
                    <ObjectTypeIcon type={definition.icon} size={18} />
                  ) : (
                    <LayoutTemplate size={18} aria-hidden="true" />
                  )}
                </span>
                <div className="settings-object-type-copy">
                  <strong>{t(template.nameZh, template.nameEn)}</strong>
                  <span>{template.id}</span>
                  <small>
                    {(definition
                      ? t(definition.labelZh, definition.labelEn)
                      : t('无对象类型', 'No object type')) +
                      ' · ' +
                      t(
                        `${[...template.bodyZh].length} 字中文模板 · ${[...template.bodyEn].length} 字英文模板`,
                        `${[...template.bodyEn].length} English chars · ${[...template.bodyZh].length} Chinese chars`,
                      )}
                  </small>
                </div>
                {confirmDeleteId === template.id ? (
                  <div className="settings-object-type-delete" role="alert">
                    <span>
                      {t(
                        '删除模板？已经创建的笔记不受影响。',
                        'Delete this template? Notes already created from it stay unchanged.',
                      )}
                    </span>
                    <button
                      type="button"
                      className="text-button danger"
                      disabled={disabled}
                      onClick={() => deleteTemplate(template)}
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
                      onClick={() => beginEdit(template)}
                    >
                      {t('编辑', 'Edit')}
                    </button>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={t(`删除${template.nameZh}`, `Delete ${template.nameEn}`)}
                      title={t('删除模板', 'Delete template')}
                      disabled={disabled || Boolean(editor)}
                      onClick={() => {
                        setConfirmDeleteId(template.id);
                        setEditor(null);
                        setMessage(null);
                      }}
                    >
                      <Trash2 size={16} aria-hidden="true" />
                    </button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {editor && (
        <section
          className="settings-object-type-editor settings-template-editor"
          aria-labelledby={`${baseId}-template-editor`}
          aria-busy={disabled}
        >
          <header>
            <div>
              <span className="settings-object-type-editor-kicker">
                {editor.mode === 'new'
                  ? t('新笔记模板', 'New note template')
                  : t('编辑笔记模板', 'Edit note template')}
              </span>
              <h3 id={`${baseId}-template-editor`}>
                {editor.mode === 'new'
                  ? t('定义模板', 'Define template')
                  : t(editor.value.nameZh, editor.value.nameEn)}
              </h3>
            </div>
            <button
              type="button"
              className="icon-button"
              aria-label={t('关闭模板编辑器', 'Close template editor')}
              onClick={() => setEditor(null)}
            >
              <X size={17} aria-hidden="true" />
            </button>
          </header>

          <fieldset disabled={disabled}>
            <legend>{t('基本信息', 'Basics')}</legend>
            <div className="settings-object-type-grid">
              <label className="settings-field stacked">
                <span>{t('模板标识', 'Template ID')}</span>
                <input
                  value={editor.value.id}
                  readOnly={editor.mode === 'edit'}
                  autoFocus={editor.mode === 'new'}
                  spellCheck={false}
                  autoComplete="off"
                  pattern="[a-z][a-z0-9_-]{1,47}"
                  onChange={(event) => patchTemplate({ id: event.target.value })}
                />
                <small>
                  {editor.mode === 'edit'
                    ? t(
                        '模板标识保存后不可更改。',
                        'The template identifier is fixed after creation.',
                      )
                    : t(
                        '2–48 位小写字母、数字、下划线或连字符。',
                        '2–48 lowercase letters, numbers, underscores or hyphens.',
                      )}
                </small>
              </label>
              <div className="choice-field settings-field stacked">
                <span>{t('对象类型', 'Object type')}</span>
                <Select
                  aria-label={t('模板对象类型', 'Template object type')}
                  value={editor.value.typeId ?? ''}
                  onValueChange={(value) => patchTemplate({ typeId: value || null })}
                >
                  <option value="">{t('无对象类型', 'No object type')}</option>
                  {catalog.map((type) => (
                    <option key={type.id} value={type.id}>
                      {t(type.labelZh, type.labelEn)}
                    </option>
                  ))}
                </Select>
                <small>
                  {t(
                    '绑定类型后，模板会继承该类型的 frontmatter 和默认属性。',
                    'A target type contributes its frontmatter and default properties.',
                  )}
                </small>
              </div>
              <label className="settings-field stacked">
                <span>{t('中文名称', 'Chinese name')}</span>
                <input
                  value={editor.value.nameZh}
                  onChange={(event) => patchTemplate({ nameZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked">
                <span>{t('英文名称', 'English name')}</span>
                <input
                  value={editor.value.nameEn}
                  onChange={(event) => patchTemplate({ nameEn: event.target.value })}
                />
              </label>
              <label className="settings-field stacked full">
                <span>{t('中文说明', 'Chinese description')}</span>
                <textarea
                  rows={2}
                  value={editor.value.descriptionZh}
                  onChange={(event) => patchTemplate({ descriptionZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked full">
                <span>{t('英文说明', 'English description')}</span>
                <textarea
                  rows={2}
                  value={editor.value.descriptionEn}
                  onChange={(event) => patchTemplate({ descriptionEn: event.target.value })}
                />
              </label>
            </div>
          </fieldset>

          <fieldset disabled={disabled}>
            <legend>{t('标题模板', 'Title template')}</legend>
            <p className="settings-description">
              {t(
                '可用占位符：{{date}} 日期、{{time}} 时间、{{type}} 对象类型名称。留空则使用类型的默认标题。',
                'Available placeholders: {{date}}, {{time}} and {{type}}. Leave empty to use the object type default title.',
              )}
            </p>
            <div className="settings-object-type-grid">
              <label className="settings-field stacked">
                <span>{t('中文默认标题', 'Chinese default title')}</span>
                <input
                  value={editor.value.titleZh}
                  onChange={(event) => patchTemplate({ titleZh: event.target.value })}
                />
              </label>
              <label className="settings-field stacked">
                <span>{t('英文默认标题', 'English default title')}</span>
                <input
                  value={editor.value.titleEn}
                  onChange={(event) => patchTemplate({ titleEn: event.target.value })}
                />
              </label>
            </div>
          </fieldset>

          <fieldset disabled={disabled}>
            <legend>{t('正文模板', 'Body template')}</legend>
            <p className="settings-description">
              {t(
                '支持 Markdown 标题、列表、任务、表格、引用、公式与 OFM。创建时按当前界面语言选择对应正文；留空则使用对象类型的分节骨架。',
                'Supports Markdown headings, lists, tasks, tables, quotes, math and OFM. The current interface language chooses the body; an empty body falls back to the object type section skeleton.',
              )}
            </p>
            <div className="settings-object-type-grid">
              <label className="settings-field stacked">
                <span>{t('中文正文', 'Chinese body')}</span>
                <textarea
                  className="settings-template-body"
                  rows={9}
                  spellCheck={false}
                  value={editor.value.bodyZh}
                  onChange={(event) => patchTemplate({ bodyZh: event.target.value })}
                />
                <small>
                  {t(
                    `${[...editor.value.bodyZh].length} / 20,000 字符`,
                    `${[...editor.value.bodyZh].length} / 20,000 characters`,
                  )}
                </small>
              </label>
              <label className="settings-field stacked">
                <span>{t('英文正文', 'English body')}</span>
                <textarea
                  className="settings-template-body"
                  rows={9}
                  spellCheck={false}
                  value={editor.value.bodyEn}
                  onChange={(event) => patchTemplate({ bodyEn: event.target.value })}
                />
                <small>
                  {t(
                    `${[...editor.value.bodyEn].length} / 20,000 字符`,
                    `${[...editor.value.bodyEn].length} / 20,000 characters`,
                  )}
                </small>
              </label>
            </div>
          </fieldset>

          <div className="settings-template-preview" aria-live="polite">
            <span>{t('预览', 'Preview')}</span>
            {preview?.ok === false ? (
              <strong className="settings-object-type-error">{preview.error}</strong>
            ) : (
              <>
                <strong>{preview?.title}</strong>
                <small>
                  {t(
                    `当前语言正文 ${preview?.bodyChars ?? 0} 字符；创建时会解析为正文块。`,
                    `${preview?.bodyChars ?? 0} characters in the current body; parsed into blocks on creation.`,
                  )}
                </small>
              </>
            )}
          </div>

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
              ? t('有未保存的模板更改。', 'You have unsaved template changes.')
              : t('模板已保存。', 'Templates are saved.'))}
        </span>
        <div>
          <button
            type="button"
            className="secondary-button"
            disabled={disabled || !dirty}
            onClick={() => {
              setTemplates(loaded.templates);
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
            {saving ? t('正在保存…', 'Saving…') : t('保存模板', 'Save templates')}
          </button>
        </div>
      </footer>
    </div>
  );
}
