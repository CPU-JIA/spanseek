import { type RefObject } from 'react';
import type { Editor } from '@tiptap/core';
import { useEditorState } from '@tiptap/react';
import {
  AlignCenter,
  AlignLeft,
  ArrowDown,
  ArrowUp,
  AlignRight,
  Bold,
  BookMarked,
  Braces,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Code,
  Code2,
  CopyPlus,
  CornerDownLeft,
  FileCode2,
  FileSymlink,
  Heading1,
  Heading2,
  Heading3,
  Heading4,
  Heading5,
  Heading6,
  Highlighter,
  ImagePlus,
  Italic,
  Library,
  Link2,
  Link2Off,
  List,
  ListChecks,
  ListOrdered,
  MessageSquare,
  Minus,
  MoreHorizontal,
  Plus,
  Quote,
  Redo2,
  Sigma,
  SquareCode,
  Strikethrough,
  Subscript,
  Superscript,
  Table2,
  Type,
  Underline,
  Undo2,
  Eraser,
  Hash,
  History,
  StickyNote,
  Workflow,
} from 'lucide-react';
import { FORMAT_COLORS, HIGHLIGHT_COLORS, DEFAULT_HIGHLIGHT } from '../rich/colors';
import { PopupMenu, type PopupMenuItem } from '../ui/PopupMenu';
import { Tooltip } from '../ui/Tooltip';
import { useI18n } from '../i18n';
import type { DraftController } from './draft-controller';
import type { ObjectTypeOption } from '../rich/YamlProperties';
import { RichEditDialog } from './rich-edit-dialog';
import { selectedBlock, selectedRichEdit, type RichEditRequest } from './rich-format-model';
import { continueOutsideContainer, selectedContainer } from './block-continuation';
import { canMoveBlock, duplicateBlockTransaction, moveBlockTransaction } from './block-operations';
import {
  headingFoldKey,
  headingFoldSelection,
  setAllHeadingsFolded,
  toggleHeadingFolded,
} from './heading-fold';

export function RichFormatting({
  editor,
  controller,
  pane,
  toolbarRef,
  busy,
  request,
  onRequest,
  onNotice,
  onLink,
  onImage,
  onReference,
  onCitation,
  onBibliography,
  onBlockHistory,
  objectTypeOptions,
}: {
  editor: Editor;
  controller: DraftController;
  pane: string;
  toolbarRef: RefObject<HTMLDivElement | null>;
  busy: boolean;
  request: RichEditRequest | null;
  onRequest: (request: RichEditRequest | null) => void;
  onNotice: (notice: string | null) => void;
  onLink: () => void;
  onImage: () => void;
  onReference: () => void;
  onCitation: () => void;
  onBibliography: () => void;
  onBlockHistory: (blockId: string) => void;
  objectTypeOptions?: readonly ObjectTypeOption[];
}) {
  const { t } = useI18n();
  const state = useEditorState({
    editor,
    selector: ({ editor: current }) => ({
      bold: current.isActive('bold'),
      italic: current.isActive('italic'),
      underline: current.isActive('underline'),
      strike: current.isActive('strike'),
      superscript: current.isActive('superscript'),
      subscript: current.isActive('subscript'),
      code: current.isActive('code'),
      table: current.isActive('table'),
      heading: current.isActive('heading') ? Number(current.getAttributes('heading').level) : 0,
      highlight: current.isActive('highlight')
        ? (current.getAttributes('highlight').color ?? DEFAULT_HIGHLIGHT)
        : null,
      color: current.getAttributes('textStyle').color ?? null,
      link: current.isActive('link'),
      undo: current.can().undo(),
      redo: current.can().redo(),
      task: current.isActive('taskItem'),
      selectedEdit: selectedRichEdit(current.state),
      container: !!selectedContainer(current.state),
      fold: headingFoldSelection(current.state),
      folded: headingFoldKey.getState(current.state)?.collapsed.size ?? 0,
      blockId: selectedBlock(current.state)?.node.attrs.id ?? null,
      blockCanUp: canMoveBlock(current.state, -1),
      blockCanDown: canMoveBlock(current.state, 1),
    }),
  });
  function run(action: () => unknown) {
    if (editor.isDestroyed || !editor.isEditable) return;
    if (editor.view.composing) {
      onNotice(t('请先完成当前输入法输入。', 'Finish the current input composition first.'));
      return;
    }
    action();
  }
  function open(kind: RichEditRequest['kind']) {
    run(() => {
      if (kind === 'frontmatter' && editor.state.doc.firstChild?.type.name === 'frontmatter') {
        onRequest({ kind, position: 0 });
        return;
      }
      if (kind === 'blockId' || kind === 'taskMarker') {
        const block = selectedBlock(editor.state, kind === 'taskMarker' ? 'taskItem' : undefined);
        if (!block) {
          onNotice(t('请先选择对应的正文块。', 'Select the corresponding document block first.'));
          return;
        }
        onRequest({ kind, position: block.position });
        return;
      }
      onRequest({ kind });
    });
  }
  const paragraphs = [
    { level: 0, label: t('正文', 'Text'), icon: Type },
    { level: 1, label: t('标题 1', 'Heading 1'), icon: Heading1 },
    { level: 2, label: t('标题 2', 'Heading 2'), icon: Heading2 },
    { level: 3, label: t('标题 3', 'Heading 3'), icon: Heading3 },
    { level: 4, label: t('标题 4', 'Heading 4'), icon: Heading4 },
    { level: 5, label: t('标题 5', 'Heading 5'), icon: Heading5 },
    { level: 6, label: t('标题 6', 'Heading 6'), icon: Heading6 },
  ];
  const marks = [
    {
      id: 'bold',
      label: t('粗体', 'Bold'),
      icon: Bold,
      shortcut: 'Ctrl+B',
      active: state.bold,
      action: () => editor.chain().focus().toggleBold().run(),
    },
    {
      id: 'italic',
      label: t('斜体', 'Italic'),
      icon: Italic,
      shortcut: 'Ctrl+I',
      active: state.italic,
      action: () => editor.chain().focus().toggleItalic().run(),
    },
    {
      id: 'underline',
      label: t('下划线', 'Underline'),
      icon: Underline,
      shortcut: 'Ctrl+U',
      active: state.underline,
      action: () => editor.chain().focus().toggleUnderline().run(),
    },
    {
      id: 'strike',
      label: t('删除线', 'Strikethrough'),
      icon: Strikethrough,
      // Matches the real binding from @tiptap/extension-strike (Mod-Shift-s),
      // not the Ctrl+Shift+X that other editors use.
      shortcut: 'Ctrl+Shift+S',
      active: state.strike,
      action: () => editor.chain().focus().toggleStrike().run(),
    },
  ];
  const structureItems: PopupMenuItem[] = [
    {
      id: 'bullet',
      label: t('项目列表', 'Bullet list'),
      icon: <List size={16} />,
      onSelect: () => run(() => editor.chain().focus().toggleBulletList().run()),
    },
    {
      id: 'ordered',
      label: t('编号列表', 'Numbered list'),
      icon: <ListOrdered size={16} />,
      onSelect: () => run(() => editor.chain().focus().toggleOrderedList().run()),
    },
    {
      id: 'task',
      label: t('任务列表', 'Checklist'),
      icon: <ListChecks size={16} />,
      onSelect: () => run(() => editor.chain().focus().toggleTaskList().run()),
    },
    {
      id: 'quote',
      label: t('引用块', 'Blockquote'),
      icon: <Quote size={16} />,
      separatorBefore: true,
      onSelect: () => run(() => editor.chain().focus().toggleBlockquote().run()),
    },
    {
      id: 'callout',
      label: t('提示块…', 'Callout…'),
      icon: <InfoIcon />,
      onSelect: () => open('callout'),
    },
    {
      id: 'fold-heading',
      label: state.fold?.collapsed
        ? t('展开当前标题', 'Expand section')
        : t('折叠当前标题', 'Collapse section'),
      icon: state.fold?.collapsed ? <ChevronRight size={16} /> : <ChevronDown size={16} />,
      disabled: !state.fold || state.fold.foldedCount === 0,
      onSelect: () =>
        run(() => {
          if (state.fold) toggleHeadingFolded(editor.view, state.fold.id);
        }),
    },
    {
      id: 'block-up',
      label: t('上移当前块', 'Move block up'),
      icon: <ArrowUp size={16} />,
      disabled: !state.blockCanUp,
      onSelect: () =>
        run(() => {
          const transaction = moveBlockTransaction(editor.state, -1);
          if (transaction) editor.view.dispatch(transaction);
          editor.commands.focus();
        }),
    },
    {
      id: 'block-down',
      label: t('下移当前块', 'Move block down'),
      icon: <ArrowDown size={16} />,
      disabled: !state.blockCanDown,
      onSelect: () =>
        run(() => {
          const transaction = moveBlockTransaction(editor.state, 1);
          if (transaction) editor.view.dispatch(transaction);
          editor.commands.focus();
        }),
    },
    {
      id: 'block-duplicate',
      label: t('复制当前块', 'Duplicate block'),
      icon: <CopyPlus size={16} />,
      disabled: !state.blockId,
      onSelect: () =>
        run(() => {
          const transaction = duplicateBlockTransaction(editor.state);
          if (transaction) editor.view.dispatch(transaction);
          editor.commands.focus();
        }),
    },
    {
      id: 'block-history',
      label: t('块历史…', 'Block history…'),
      icon: <History size={16} />,
      disabled: !state.blockId,
      onSelect: () => state.blockId && onBlockHistory(state.blockId),
    },
    {
      id: 'continue-after',
      label: t('在块后继续写', 'Continue after block'),
      icon: <CornerDownLeft size={16} />,
      shortcut: 'Ctrl+Enter',
      disabled: !state.container,
      onSelect: () =>
        run(() => {
          if (
            continueOutsideContainer(editor.state, (transaction) =>
              editor.view.dispatch(transaction),
            )
          )
            editor.commands.focus();
        }),
    },
    {
      id: 'code',
      label: t('代码块', 'Code block'),
      icon: <Code2 size={16} />,
      onSelect: () => run(() => editor.chain().focus().toggleCodeBlock().run()),
    },
    {
      id: 'divider',
      label: t('分隔线', 'Divider'),
      icon: <Minus size={16} />,
      onSelect: () => run(() => editor.chain().focus().setHorizontalRule().run()),
    },
    {
      id: 'table',
      label: t('插入表格', 'Insert table'),
      icon: <Table2 size={16} />,
      separatorBefore: true,
      onSelect: () =>
        run(() =>
          editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(),
        ),
    },
  ];
  const insertionItems: PopupMenuItem[] = [
    {
      id: 'image',
      label: t('插入图片', 'Insert image'),
      icon: <ImagePlus size={16} />,
      disabled: busy,
      onSelect: () => run(onImage),
    },
    {
      id: 'reference',
      label: t('插入笔记引用', 'Insert note reference'),
      icon: <FileSymlink size={16} />,
      onSelect: () => run(onReference),
    },
    {
      id: 'citation',
      label: t('插入文献引用…', 'Insert citation…'),
      icon: <BookMarked size={16} />,
      disabled: busy,
      onSelect: () => run(onCitation),
    },
    {
      id: 'bibliography',
      label: t('参考文献列表…', 'Bibliography…'),
      icon: <Library size={16} />,
      onSelect: () => run(onBibliography),
    },
    {
      id: 'wiki',
      label: t('内部链接…', 'Internal link…'),
      icon: <Link2 size={16} />,
      onSelect: () => open('wikiLink'),
    },
    {
      id: 'embed',
      label: t('嵌入内容…', 'Embed content…'),
      icon: <StickyNote size={16} />,
      onSelect: () => open('wikiEmbed'),
    },
    {
      id: 'inlineMath',
      label: t('行内公式…', 'Inline equation…'),
      icon: <Sigma size={16} />,
      separatorBefore: true,
      groupLabel: t('公式与图表', 'Equations and diagrams'),
      onSelect: () => open('inlineMath'),
    },
    {
      id: 'blockMath',
      label: t('公式块…', 'Equation block…'),
      icon: <SquareCode size={16} />,
      onSelect: () => open('blockMath'),
    },
    {
      id: 'mermaid',
      label: t('Mermaid 图表…', 'Mermaid diagram…'),
      icon: <Workflow size={16} />,
      onSelect: () => open('mermaid'),
    },
    {
      id: 'footnote',
      label: t('脚注…', 'Footnote…'),
      icon: <Superscript size={16} />,
      separatorBefore: true,
      groupLabel: t('注释与属性', 'Notes and properties'),
      onSelect: () => open('footnote'),
    },
    {
      id: 'footnoteReference',
      label: t('引用已有脚注…', 'Reference an existing footnote…'),
      icon: <Hash size={16} />,
      onSelect: () => open('footnoteReference'),
    },
    {
      id: 'comment',
      label: t('行内隐藏内容…', 'Inline hidden content…'),
      icon: <MessageSquare size={16} />,
      onSelect: () => open('comment'),
    },
    {
      id: 'commentBlock',
      label: t('隐藏内容块…', 'Hidden content block…'),
      icon: <MessageSquare size={16} />,
      onSelect: () => open('commentBlock'),
    },
    {
      id: 'frontmatter',
      label: t('YAML 属性…', 'YAML properties…'),
      icon: <Braces size={16} />,
      onSelect: () => open('frontmatter'),
    },
    {
      id: 'markdown',
      label: t('插入 Markdown…', 'Insert Markdown…'),
      icon: <FileCode2 size={16} />,
      separatorBefore: true,
      onSelect: () => open('markdown'),
    },
  ];
  const moreItems: PopupMenuItem[] = [
    {
      id: 'undo',
      label: t('撤销', 'Undo'),
      icon: <Undo2 size={16} />,
      shortcut: 'Ctrl+Z',
      disabled: !state.undo,
      onSelect: () => run(() => editor.chain().focus().undo().run()),
    },
    {
      id: 'redo',
      label: t('重做', 'Redo'),
      icon: <Redo2 size={16} />,
      shortcut: 'Ctrl+Shift+Z',
      disabled: !state.redo,
      onSelect: () => run(() => editor.chain().focus().redo().run()),
    },
    {
      id: 'sup',
      label: t('上标', 'Superscript'),
      icon: <Superscript size={16} />,
      checked: state.superscript,
      separatorBefore: true,
      onSelect: () => run(() => editor.chain().focus().toggleMark('superscript').run()),
    },
    {
      id: 'sub',
      label: t('下标', 'Subscript'),
      icon: <Subscript size={16} />,
      checked: state.subscript,
      onSelect: () => run(() => editor.chain().focus().toggleMark('subscript').run()),
    },
    {
      id: 'code',
      label: t('行内代码', 'Inline code'),
      icon: <Code size={16} />,
      checked: state.code,
      onSelect: () => run(() => editor.chain().focus().toggleCode().run()),
    },
    {
      id: 'clear',
      label: t('清除文字格式', 'Clear text formatting'),
      icon: <Eraser size={16} />,
      separatorBefore: true,
      onSelect: () => run(() => editor.chain().focus().unsetAllMarks().run()),
    },
    {
      id: 'unlink',
      label: t('移除选中链接', 'Remove selected link'),
      icon: <Link2Off size={16} />,
      disabled: !state.link,
      onSelect: () => run(() => editor.chain().focus().extendMarkRange('link').unsetLink().run()),
    },
    {
      id: 'edit',
      label: t('编辑选中格式…', 'Edit selected content…'),
      icon: <FileCode2 size={16} />,
      disabled: !state.selectedEdit,
      separatorBefore: true,
      onSelect: () =>
        run(() => {
          const current = selectedRichEdit(editor.state);
          if (current) onRequest(current);
        }),
    },
    {
      id: 'fold-all',
      label: t('折叠全部标题', 'Collapse all headings'),
      icon: <ChevronDown size={16} />,
      onSelect: () => run(() => setAllHeadingsFolded(editor.view, true)),
    },
    {
      id: 'unfold-all',
      label: t('展开全部标题', 'Expand all headings'),
      icon: <ChevronUp size={16} />,
      disabled: state.folded === 0,
      onSelect: () => run(() => setAllHeadingsFolded(editor.view, false)),
    },
    {
      id: 'anchor',
      label: t('块属性与标识…', 'Block properties…'),
      separatorBefore: true,
      icon: <Hash size={16} />,
      onSelect: () => open('blockId'),
    },
    {
      id: 'taskMarker',
      label: t('自定义任务标记…', 'Custom task marker…'),
      icon: <ListChecks size={16} />,
      disabled: !state.task,
      onSelect: () => open('taskMarker'),
    },
  ];
  const tableItems: PopupMenuItem[] = [
    {
      id: 'addRow',
      label: t('添加行', 'Add row'),
      onSelect: () => run(() => editor.chain().focus().addRowAfter().run()),
    },
    {
      id: 'addColumn',
      label: t('添加列', 'Add column'),
      onSelect: () => run(() => editor.chain().focus().addColumnAfter().run()),
    },
    {
      id: 'deleteRow',
      label: t('删除行', 'Delete row'),
      onSelect: () => run(() => editor.chain().focus().deleteRow().run()),
    },
    {
      id: 'deleteColumn',
      label: t('删除列', 'Delete column'),
      onSelect: () => run(() => editor.chain().focus().deleteColumn().run()),
    },
    {
      id: 'header',
      label: t('切换表头', 'Toggle header'),
      onSelect: () => run(() => editor.chain().focus().toggleHeaderRow().run()),
    },
    ...(['left', 'center', 'right'] as const).map((align, index) => ({
      id: 'align-' + align,
      label: [
        t('单元格左对齐', 'Align cells left'),
        t('单元格居中', 'Center cells'),
        t('单元格右对齐', 'Align cells right'),
      ][index],
      icon:
        index === 0 ? (
          <AlignLeft size={16} />
        ) : index === 1 ? (
          <AlignCenter size={16} />
        ) : (
          <AlignRight size={16} />
        ),
      separatorBefore: index === 0,
      onSelect: () => run(() => editor.chain().focus().setCellAttribute('align', align).run()),
    })),
    {
      id: 'deleteTable',
      label: t('移除表格', 'Remove table'),
      separatorBefore: true,
      onSelect: () => run(() => editor.chain().focus().deleteTable().run()),
    },
  ];
  return (
    <>
      <div
        ref={toolbarRef}
        className="editor-toolbar rich-format-toolbar"
        role="toolbar"
        aria-label={t('正文格式', 'Text formatting')}
      >
        <div className="rich-format-group">
          <PopupMenu
            className="paragraph-style-trigger"
            label={t('段落样式', 'Paragraph style')}
            disabled={!editor.isEditable}
            trigger={
              <>
                <span>{paragraphs[state.heading]?.label ?? paragraphs[0].label}</span>
                <ChevronDown size={14} />
              </>
            }
            items={paragraphs.map(({ level, label, icon: Icon }) => ({
              id: String(level),
              label,
              icon: <Icon size={16} />,
              checked: state.heading === level,
              radio: true,
              onSelect: () =>
                run(() =>
                  level
                    ? editor
                        .chain()
                        .focus()
                        .setHeading({ level: level as 1 | 2 | 3 | 4 | 5 | 6 })
                        .run()
                    : editor.chain().focus().setParagraph().run(),
                ),
            }))}
          />
          {marks.map(({ id, label, shortcut, active, icon: Icon, action }) => (
            <Tooltip key={id} label={label} shortcut={shortcut}>
              <button
                type="button"
                className="icon-button"
                aria-label={label}
                aria-pressed={active}
                disabled={!editor.isEditable}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => run(action)}
              >
                <Icon size={17} strokeWidth={1.6} />
              </button>
            </Tooltip>
          ))}
        </div>
        <div className="rich-format-group">
          <PopupMenu
            label={t('高亮颜色', 'Highlight color')}
            trigger={<Highlighter size={17} />}
            disabled={!editor.isEditable}
            menuClassName="rich-color-menu"
            items={[
              ...HIGHLIGHT_COLORS.map(({ label, en, color }) => ({
                id: color,
                label: t(label, en),
                icon: <span className="rich-color-swatch" style={{ backgroundColor: color }} />,
                checked: state.highlight === color,
                radio: true,
                onSelect: () =>
                  run(() => editor.chain().focus().setMark('highlight', { color }).run()),
              })),
              {
                id: 'custom',
                label: t('自定义颜色…', 'Custom color…'),
                separatorBefore: true,
                onSelect: () => open('highlightColor'),
              },
              {
                id: 'clear',
                label: t('移除高亮', 'Remove highlight'),
                onSelect: () => run(() => editor.chain().focus().unsetMark('highlight').run()),
              },
            ]}
          />
          <PopupMenu
            label={t('文字颜色', 'Text color')}
            trigger={<Type size={17} />}
            disabled={!editor.isEditable}
            menuClassName="rich-color-menu"
            items={[
              {
                id: 'default',
                label: t('默认文字颜色', 'Default text color'),
                checked: !state.color,
                radio: true,
                onSelect: () => run(() => editor.chain().focus().unsetMark('textStyle').run()),
              },
              ...FORMAT_COLORS.map(({ label, en, color }) => ({
                id: color,
                label: t(label, en),
                icon: <span className="rich-color-swatch" style={{ backgroundColor: color }} />,
                checked: state.color === color,
                radio: true,
                onSelect: () =>
                  run(() => editor.chain().focus().setMark('textStyle', { color }).run()),
              })),
              {
                id: 'custom',
                label: t('自定义颜色…', 'Custom color…'),
                separatorBefore: true,
                onSelect: () => open('textColor'),
              },
            ]}
          />
          <PopupMenu
            label={t('列表与块', 'Lists and blocks')}
            trigger={<List size={17} />}
            disabled={!editor.isEditable}
            items={structureItems}
          />
          <Tooltip label={t('编辑链接', 'Edit link')}>
            <button
              type="button"
              className="icon-button"
              aria-label={t('编辑链接', 'Edit link')}
              disabled={!editor.isEditable}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => run(onLink)}
            >
              <Link2 size={17} strokeWidth={1.6} />
            </button>
          </Tooltip>
          {state.table && (
            <PopupMenu
              label={t('表格操作', 'Table actions')}
              trigger={<Table2 size={17} />}
              disabled={!editor.isEditable}
              items={tableItems}
            />
          )}
          <PopupMenu
            className="rich-insert-trigger"
            label={t('插入内容', 'Insert content')}
            trigger={
              <>
                <Plus size={16} />
                <span>{t('插入', 'Insert')}</span>
              </>
            }
            disabled={!editor.isEditable}
            placement="top-end"
            items={insertionItems}
          />
          <PopupMenu
            label={t('更多格式', 'More formatting')}
            trigger={<MoreHorizontal size={18} />}
            disabled={!editor.isEditable}
            placement="top-end"
            items={moreItems}
          />
        </div>
      </div>
      {request && (
        <RichEditDialog
          editor={editor}
          controller={controller}
          pane={pane}
          request={request}
          objectTypeOptions={objectTypeOptions}
          onClose={() => onRequest(null)}
          onNotice={onNotice}
        />
      )}
    </>
  );
}

function InfoIcon() {
  return <StickyNote size={16} />;
}
