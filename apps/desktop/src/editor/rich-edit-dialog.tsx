import { useDeferredValue, useId, useLayoutEffect, useRef, useState } from 'react';
import type { Editor, EditorEvents } from '@tiptap/core';
import { EditorState, TextSelection, type Transaction } from '@tiptap/pm/state';
import { findWrapping } from '@tiptap/pm/transform';
import { X } from 'lucide-react';
import { useI18n } from '../i18n';
import { MathContent, MermaidContent, YamlProperties } from '../rich/FormatContent';
import type { ObjectTypeOption } from '../rich/YamlProperties';
import { CALLOUT_ALIASES, CALLOUT_TYPES } from '../rich/callouts';
import { Combobox } from '../ui/Combobox';
import { Select } from '../ui/Select';
import type { DraftController } from './draft-controller';
import { insertPreparedMarkdown, prepareEditorMarkdown } from './rich-markdown';
import {
  blockAnchorError,
  captureRichEditTarget,
  calloutTypeError,
  calloutTitleError,
  dimensionValue,
  editorColor,
  footnoteLabelError,
  insertFootnote,
  mapRichEditTarget,
  nextFootnoteLabel,
  renameFootnoteDefinition,
  richDocumentError,
  richEditTargetPosition,
  sourceError,
  validTaskMarker,
  wikiTargetError,
  type RichEditRequest,
} from './rich-format-model';

const names: Record<RichEditRequest['kind'], [string, string]> = {
  inlineMath: ['行内公式', 'Inline equation'],
  blockMath: ['公式块', 'Equation block'],
  mermaid: ['Mermaid 图表', 'Mermaid diagram'],
  comment: ['行内隐藏内容', 'Inline hidden content'],
  commentBlock: ['隐藏内容块', 'Hidden content block'],
  wikiLink: ['内部链接', 'Internal link'],
  wikiEmbed: ['嵌入内容', 'Embed content'],
  footnote: ['插入脚注', 'Insert footnote'],
  footnoteReference: ['脚注引用', 'Footnote reference'],
  footnoteDefinition: ['脚注标识', 'Footnote label'],
  callout: ['提示块', 'Callout'],
  frontmatter: ['YAML 属性', 'YAML properties'],
  blockId: ['块属性与标识', 'Block properties'],
  taskMarker: ['任务标记', 'Task marker'],
  markdown: ['插入 Markdown', 'Insert Markdown'],
  highlightColor: ['自定义高亮颜色', 'Custom highlight color'],
  textColor: ['自定义文字颜色', 'Custom text color'],
};

export function RichEditDialog({
  editor,
  controller,
  pane,
  request,
  onClose,
  onNotice,
  objectTypeOptions,
}: {
  editor: Editor;
  controller: DraftController;
  pane: string;
  request: RichEditRequest;
  onClose: () => void;
  onNotice: (notice: string | null) => void;
  objectTypeOptions?: readonly ObjectTypeOption[];
}) {
  const { t } = useI18n();
  const id = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const [initial] = useState(() => captureRichEditTarget(editor.state, request.position));
  const first = useRef(initial);
  const [fields, setFields] = useState(() => {
    const original = initial.node;
    const initialText =
      request.kind === 'highlightColor' || request.kind === 'textColor'
        ? (editor.getAttributes(request.kind === 'highlightColor' ? 'highlight' : 'textStyle')
            .color ?? '#2563EB')
        : (original?.attrs.source ??
          (request.kind === 'mermaid'
            ? (original?.textContent ?? '')
            : editor.state.doc.textBetween(
                editor.state.selection.from,
                editor.state.selection.to,
                '\n',
              )));
    return {
      source: request.source ?? String(initialText ?? ''),
      target: String(original?.attrs.wiki_target ?? ''),
      label: String(
        original?.attrs.label ??
          (request.kind === 'footnote' ? nextFootnoteLabel(editor.state.doc) : ''),
      ),
      title: String(original?.attrs.title ?? ''),
      type: String(original?.attrs.callout_type ?? 'note'),
      collapsed:
        original?.attrs.collapsed == null ? 'none' : original.attrs.collapsed ? 'closed' : 'open',
      width: String(original?.attrs.embed_width ?? ''),
      height: String(original?.attrs.embed_height ?? ''),
      anchor: String(original?.attrs.block_id ?? ''),
      marker: String(original?.attrs.task_marker ?? ''),
    };
  });
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const activeParse = useRef<AbortController | null>(null);
  const composing = useRef(false);
  const compositionTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const applied = useRef(false);
  const actionReady = useRef(false);
  const token = pane + '/rich-dialog/' + id;
  const deferredSource = useDeferredValue(fields.source);
  const kind = request.kind;
  const hasSource = [
    'inlineMath',
    'blockMath',
    'mermaid',
    'comment',
    'commentBlock',
    'frontmatter',
    'footnote',
    'markdown',
  ].includes(kind);
  const title =
    kind === 'markdown' && request.source !== undefined
      ? t('粘贴内容', 'Paste content')
      : t(names[kind][0], names[kind][1]);
  function field(key: keyof typeof fields, value: string) {
    setFields((current) => ({ ...current, [key]: value }));
    setError(null);
  }
  useLayoutEffect(() => {
    let finish: (() => void) | undefined;
    try {
      finish = controller.beginAction();
      actionReady.current = true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '编辑器暂时不可写。');
    }
    const element = dialog.current!;
    element.showModal();
    element.querySelector<HTMLInputElement | HTMLTextAreaElement>('input, textarea')?.focus();
    const map = ({ transaction, appendedTransactions }: EditorEvents['transaction']) => {
      for (const change of [transaction, ...appendedTransactions])
        first.current = mapRichEditTarget(first.current, change);
    };
    editor.on('transaction', map);
    return () => {
      editor.off('transaction', map);
      activeParse.current?.abort();
      clearTimeout(compositionTimer.current);
      controller.setComposing(token, false);
      finish?.();
      actionReady.current = false;
      element.close();
      if (!editor.isDestroyed) {
        if (!applied.current) {
          try {
            editor.view.dispatch(
              editor.state.tr
                .setSelection(first.current.bookmark.resolve(editor.state.doc))
                .setMeta('addToHistory', false),
            );
          } catch {
            /* A removed selection cannot be restored; keep the editor's mapped selection. */
          }
        }
        editor.commands.focus(undefined, { scrollIntoView: false });
      }
    };
  }, [controller, editor, token]);

  function close() {
    if (composing.current) {
      setError(t('请先完成当前输入法输入。', 'Finish the current input composition first.'));
      return;
    }
    activeParse.current?.abort();
    onClose();
  }
  function targetPosition() {
    return richEditTargetPosition(editor.state.doc, first.current);
  }
  async function apply() {
    if (working || composing.current || editor.view.composing) return;
    if (!actionReady.current || editor.isDestroyed || !editor.isEditable) {
      setError(t('编辑器暂时不可写。', 'The editor is not editable right now.'));
      return;
    }
    try {
      function dispatch(transaction: Transaction) {
        const failure = richDocumentError(transaction.doc);
        if (failure) throw new Error(failure);
        editor.view.dispatch(transaction);
      }
      const at = targetPosition();
      const current = at === undefined ? null : editor.state.doc.nodeAt(at);
      const selection = first.current.bookmark.resolve(editor.state.doc);
      const attributes: Record<string, unknown> = {};
      if (hasSource && kind !== 'markdown') {
        const failure = sourceError(fields.source, kind === 'inlineMath' || kind === 'blockMath');
        if (failure) throw new Error(failure);
        attributes.source = fields.source;
      }
      if (kind === 'wikiLink' || kind === 'wikiEmbed') {
        const failure = wikiTargetError(fields.target);
        if (failure) throw new Error(failure);
        if ([...fields.label].length > 512 || /[\u0000-\u001f\u007f-\u009f]/.test(fields.label))
          throw new Error('显示别名不能超过 512 个字符，且不能包含控制字符。');
        attributes.wiki_target = fields.target;
        attributes.label = fields.label || null;
        if (fields.target !== current?.attrs.wiki_target) {
          attributes.target_id = null;
          attributes.target_block_id = null;
          attributes.target_revision_id = null;
        }
        if (kind === 'wikiEmbed') {
          attributes.embed_width = dimensionValue(fields.width);
          attributes.embed_height = dimensionValue(fields.height);
        }
      }
      if (kind === 'footnoteReference' || kind === 'footnoteDefinition') {
        const failure = footnoteLabelError(fields.label);
        if (failure) throw new Error(failure);
        attributes.label = fields.label;
      }
      if (kind === 'callout') {
        const failure = calloutTypeError(fields.type) || calloutTitleError(fields.title);
        if (failure) throw new Error(failure);
        attributes.callout_type = fields.type;
        attributes.title = fields.title || null;
        attributes.collapsed = fields.collapsed === 'none' ? null : fields.collapsed === 'closed';
      }
      if (kind === 'blockId') {
        const failure = blockAnchorError(fields.anchor, editor.state.doc, current?.attrs.id);
        if (failure) throw new Error(failure);
        attributes.block_id = fields.anchor || null;
      }
      if (kind === 'taskMarker') {
        if (fields.marker && !validTaskMarker(fields.marker))
          throw new Error('任务标记须为一个非控制字符，且不能是方括号。');
        attributes.task_marker = fields.marker || null;
        if (fields.marker) attributes.checked = fields.marker !== ' ';
      }
      if (kind === 'markdown') {
        const operation = new AbortController();
        activeParse.current = operation;
        setWorking(true);
        const baseDocument = editor.state.doc;
        const parsed = await prepareEditorMarkdown(
          editor,
          fields.source,
          selection,
          operation.signal,
        );
        if (operation.signal.aborted) return;
        if (editor.isDestroyed || !editor.isEditable)
          throw new Error('编辑器暂时不可写，输入已保留。');
        if (!editor.state.doc.eq(baseDocument))
          throw new Error('正文在准备期间已改变，请再次应用；输入已保留。');
        insertPreparedMarkdown(
          editor,
          parsed,
          onNotice,
          first.current.bookmark.resolve(editor.state.doc),
        );
      } else if (kind === 'footnote') {
        const state = EditorState.create({
          schema: editor.schema,
          doc: editor.state.doc,
          selection,
          storedMarks: editor.state.storedMarks,
        });
        dispatch(insertFootnote(state, fields.label, fields.source));
      } else if (kind === 'highlightColor' || kind === 'textColor') {
        const color = editorColor(fields.source);
        if (!color) throw new Error('请输入有效的 #RRGGBB 颜色。');
        const mark = editor.schema.marks[
          kind === 'highlightColor' ? 'highlight' : 'textStyle'
        ].create({ color });
        const transaction = editor.state.tr.setSelection(selection);
        if (selection.empty)
          transaction.setStoredMarks(
            mark.addToSet(editor.state.storedMarks ?? selection.$from.marks()),
          );
        else transaction.addMark(selection.from, selection.to, mark);
        dispatch(transaction);
      } else if (current && at !== undefined) {
        const transaction =
          kind === 'footnoteDefinition'
            ? renameFootnoteDefinition(editor.state, at, fields.label)
            : editor.state.tr.setNodeMarkup(at, undefined, { ...current.attrs, ...attributes });
        if (kind === 'taskMarker') transaction.setMeta('pkmsExplicitTaskMarker', true);
        dispatch(transaction.scrollIntoView());
      } else {
        const schema = editor.schema;
        const transaction = editor.state.tr.setSelection(selection);
        const uuid = crypto.randomUUID();
        if (kind === 'frontmatter') {
          if (editor.state.doc.firstChild?.type.name === 'frontmatter')
            throw new Error('文档顶部已有 YAML 属性，请编辑现有属性。');
          transaction.insert(0, schema.nodes.frontmatter.create({ ...attributes, id: uuid }));
        } else if (kind === 'callout') {
          const range = selection.$from.blockRange(selection.$to);
          const wrapping =
            range && findWrapping(range, schema.nodes.callout, { ...attributes, id: uuid });
          if (!range || !wrapping) throw new Error('当前位置不能创建提示块，请选择正文段落。');
          transaction.wrap(range, wrapping);
        } else {
          const type = schema.nodes[kind === 'mermaid' ? 'codeBlock' : kind];
          if (!type) throw new Error('此格式当前无法插入。');
          const attrs: Record<string, unknown> = {
            ...attributes,
            ...(type.isBlock ? { id: uuid } : {}),
            ...(kind === 'wikiLink' || kind === 'wikiEmbed'
              ? { occurrence_id: crypto.randomUUID() }
              : {}),
            ...(kind === 'mermaid' ? { language: 'mermaid' } : {}),
          };
          if (kind === 'mermaid') delete attrs.source;
          const node = type.create(
            attrs,
            kind === 'mermaid' && fields.source ? schema.text(fields.source) : null,
            type.isInline ? (editor.state.storedMarks ?? selection.$from.marks()) : undefined,
          );
          transaction.replaceSelectionWith(node, false);
          if (type.isBlock && transaction.doc.lastChild?.attrs.id === uuid) {
            const end = transaction.doc.content.size;
            transaction.insert(end, schema.nodes.paragraph.create({ id: crypto.randomUUID() }));
            transaction.setSelection(TextSelection.near(transaction.doc.resolve(end + 1)));
          }
        }
        dispatch(transaction.scrollIntoView());
      }
      applied.current = true;
      onClose();
    } catch (cause) {
      if (!activeParse.current?.signal.aborted)
        setError(
          cause instanceof Error
            ? cause.message
            : t(
                '内容未能应用，输入已保留。',
                'The content could not be applied. Your input is retained.',
              ),
        );
    } finally {
      if (!activeParse.current?.signal.aborted) {
        activeParse.current = null;
        setWorking(false);
      }
    }
  }
  return (
    <dialog
      ref={dialog}
      className="rich-edit-dialog"
      aria-labelledby={id + '-title'}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onCompositionStart={() => {
        clearTimeout(compositionTimer.current);
        composing.current = true;
        controller.setComposing(token, true);
      }}
      onCompositionEnd={() => {
        compositionTimer.current = setTimeout(() => {
          composing.current = false;
          controller.setComposing(token, false);
        }, 0);
      }}
      onKeyDown={(event) => {
        event.stopPropagation();
        if ((event.nativeEvent.isComposing || event.keyCode === 229) && event.key === 'Escape')
          event.preventDefault();
      }}
    >
      <form
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void apply();
        }}
      >
        <header>
          <h2 id={id + '-title'}>{title}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label={t('关闭格式编辑', 'Close format editor')}
            onClick={close}
          >
            <X size={18} aria-hidden="true" />
          </button>
        </header>
        <fieldset className="rich-edit-fields" disabled={working} aria-busy={working}>
          {(kind === 'highlightColor' || kind === 'textColor') && (
            <label>
              {t('颜色', 'Color')}
              <div className="rich-edit-row">
                <input
                  type="color"
                  aria-label={t('选择颜色', 'Choose color')}
                  value={editorColor(fields.source) ?? '#2563EB'}
                  onChange={(event) => field('source', event.target.value)}
                />
                <input
                  aria-label={t('十六进制颜色', 'Hex color')}
                  value={fields.source}
                  onChange={(event) => field('source', event.target.value)}
                  placeholder="#RRGGBB"
                />
              </div>
            </label>
          )}
          {(kind === 'wikiLink' || kind === 'wikiEmbed') && (
            <>
              <label>
                {t('目标名称或路径', 'Target name or path')}
                <input
                  value={fields.target}
                  onChange={(event) => field('target', event.target.value)}
                  placeholder={t('笔记#标题 或 笔记#^块标识', 'Note#Heading or Note#^block-id')}
                />
              </label>
              <label>
                {t('显示别名（可选）', 'Display alias (optional)')}
                <input
                  value={fields.label}
                  onChange={(event) => field('label', event.target.value)}
                />
              </label>
              {kind === 'wikiEmbed' && (
                <div className="rich-edit-row">
                  <label>
                    {t('宽度（可选）', 'Width (optional)')}
                    <input
                      inputMode="numeric"
                      value={fields.width}
                      onChange={(event) => field('width', event.target.value)}
                    />
                  </label>
                  <label>
                    {t('高度（可选）', 'Height (optional)')}
                    <input
                      inputMode="numeric"
                      value={fields.height}
                      onChange={(event) => field('height', event.target.value)}
                    />
                  </label>
                </div>
              )}
              <p className="rich-edit-help">
                {t(
                  '目标由当前知识库解析；不存在或同名不明时会明确提示。',
                  'The target is resolved within this knowledge base. Missing or ambiguous targets are shown explicitly.',
                )}
              </p>
            </>
          )}
          {['footnote', 'footnoteReference', 'footnoteDefinition'].includes(kind) && (
            <label>
              {t('脚注标识', 'Footnote label')}
              <input
                value={fields.label}
                onChange={(event) => field('label', event.target.value)}
              />
            </label>
          )}
          {kind === 'callout' && (
            <>
              <label>
                {t('类型', 'Type')}
                <Combobox
                  aria-label={t('提示块类型', 'Callout type')}
                  allowCustom
                  value={fields.type}
                  onValueChange={(value) => field('type', value)}
                  options={[
                    ...CALLOUT_TYPES.map((item) => ({
                      value: item.type,
                      label: `${t(item.label, item.en)} · ${item.type}`,
                      searchText: Object.entries(CALLOUT_ALIASES)
                        .filter(([, type]) => type === item.type)
                        .map(([alias]) => alias)
                        .join(' '),
                    })),
                    ...Object.entries(CALLOUT_ALIASES).map(([alias, type]) => ({
                      value: alias,
                      label: alias,
                      description: t('别名', 'Alias') + ' · ' + type,
                      searchText: type,
                    })),
                  ]}
                  placeholder={t(
                    '选择、搜索或输入自定义类型',
                    'Choose, search or enter a custom type',
                  )}
                />
              </label>
              <label>
                {t('标题（可选）', 'Title (optional)')}
                <input
                  value={fields.title}
                  onChange={(event) => field('title', event.target.value)}
                />
              </label>
              <div className="choice-field">
                {t('折叠', 'Folding')}
                <Select
                  aria-label={t('折叠', 'Folding')}
                  value={fields.collapsed}
                  onValueChange={(value) => field('collapsed', value)}
                >
                  <option value="none">{t('不可折叠', 'Not collapsible')}</option>
                  <option value="open">{t('默认展开', 'Expanded by default')}</option>
                  <option value="closed">{t('默认收起', 'Collapsed by default')}</option>
                </Select>
              </div>
              <p className="rich-edit-help">
                {t(
                  '支持 1–64 个字母、数字、下划线或连字符。未知类型保留原名称，使用笔记提示块的外观。空末段 Enter 或 Ctrl+Enter 可在块外继续写。',
                  'Use 1–64 letters, digits, underscores or hyphens. Custom types keep their name and use the note appearance. Enter in an empty final paragraph, or Ctrl+Enter, continues outside the block.',
                )}
              </p>
            </>
          )}
          {kind === 'blockId' && (
            <>
              <label>
                {t('块标识（留空移除）', 'Block identifier (empty removes it)')}
                <input
                  value={fields.anchor}
                  onChange={(event) => field('anchor', event.target.value)}
                  placeholder="my-block"
                />
              </label>
              <dl className="rich-edit-block-info">
                <div>
                  <dt>{t('块 UUID', 'Block UUID')}</dt>
                  <dd>{initial.node?.attrs.id ?? t('未保存', 'Unsaved')}</dd>
                </div>
                <div>
                  <dt>{t('块类型', 'Block type')}</dt>
                  <dd>{initial.node?.type.name ?? t('未知', 'Unknown')}</dd>
                </div>
              </dl>
            </>
          )}
          {kind === 'taskMarker' && (
            <label>
              {t('任务标记（留空使用复选框状态）', 'Task marker (empty uses checkbox state)')}
              <input
                value={fields.marker}
                onChange={(event) => field('marker', event.target.value)}
              />
            </label>
          )}
          {hasSource && (
            <label>
              {kind === 'footnote'
                ? t('脚注正文', 'Footnote content')
                : kind === 'markdown'
                  ? 'Markdown'
                  : t('源码', 'Source')}
              <textarea
                rows={kind === 'inlineMath' || kind === 'comment' ? 3 : 7}
                spellCheck={false}
                value={fields.source}
                onChange={(event) => field('source', event.target.value)}
              />
            </label>
          )}
          {kind === 'frontmatter' && (
            <YamlProperties
              source={fields.source}
              objectTypeOptions={objectTypeOptions}
              onChange={(value) => field('source', value)}
            />
          )}
          {(kind === 'inlineMath' || kind === 'blockMath') && deferredSource.trim() && (
            <div className="rich-edit-preview">
              <MathContent source={deferredSource} display={kind === 'blockMath'} />
            </div>
          )}
          {kind === 'mermaid' && deferredSource.trim() && (
            <div className="rich-edit-preview">
              <MermaidContent source={deferredSource} />
            </div>
          )}
          {(kind === 'comment' || kind === 'commentBlock') && (
            <p className="rich-edit-help">
              {t(
                '隐藏内容保存为 %% 注释。编辑与阅读时默认模糊，点击可显隐；正文搜索和 HTML 导出仍隐藏，Markdown 保留原注释语法。显隐不会修改正文。',
                'Hidden content is stored as a %% comment. It starts blurred in the editor and reader; click to reveal or conceal. Body search and HTML exports keep it hidden, and Markdown retains the comment syntax. Revealing does not change the document.',
              )}
            </p>
          )}
        </fieldset>
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <div className="button-row">
          <button type="button" className="secondary-button" onClick={close}>
            {t('取消', 'Cancel')}
          </button>
          <button type="submit" className="primary-button" disabled={working}>
            {working ? t('正在准备…', 'Preparing…') : t('应用', 'Apply')}
          </button>
        </div>
      </form>
    </dialog>
  );
}
