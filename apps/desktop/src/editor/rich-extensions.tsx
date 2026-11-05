import {
  useDeferredValue,
  useEffect,
  useId,
  useRef,
  type CSSProperties,
  type FocusEvent,
} from 'react';
import {
  NodeViewContent,
  NodeViewWrapper,
  ReactNodeViewRenderer,
  useEditorState,
  type NodeViewProps,
} from '@tiptap/react';
import CodeBlock from '@tiptap/extension-code-block';
import { ChevronDown, ChevronRight, CornerDownLeft, Pencil, Plus } from 'lucide-react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { MathContent, MermaidContent, WikiContent, YamlProperties } from '../rich/FormatContent';
import type { ObjectTypeOption } from '../rich/YamlProperties';
import { getCallout } from '../rich/callouts';
import { CalloutIcon } from '../rich/CalloutContent';
import { footnoteKey } from '../rich/footnotes';
import { useI18n } from '../i18n';
import { Tooltip } from '../ui/Tooltip';
import type { DraftController } from './draft-controller';
import {
  Callout,
  Comment,
  CommentBlock,
  FootnoteDefinition,
  FootnoteReference,
  Frontmatter,
  InlineMath,
  BlockMath,
  WikiLink,
  WikiEmbed,
  Highlight,
  TextStyle,
  Superscript,
  Subscript,
  RichBlockAttributes,
} from './rich-schema';
import { footnoteIndex, sourceError, type RichEditRequest } from './rich-format-model';
import { HiddenContent } from '../rich/HiddenContent';
import { continueOutsideContainer } from './block-continuation';
export { AlignedTableCell, AlignedTableHeader } from './rich-schema';

export interface RichNodeOptions {
  identity: WorkspaceIdentity;
  controller: DraftController;
  pane: string;
  onEdit: (request: RichEditRequest) => void;
  onError: (error: string) => void;
  objectTypeOptions?: readonly ObjectTypeOption[];
}

function options(props: NodeViewProps) {
  return props.extension.options as RichNodeOptions;
}
function editNode(props: NodeViewProps, kind = props.node.type.name as RichEditRequest['kind']) {
  const at = props.getPos();
  if (typeof at !== 'number' || props.editor.isDestroyed || !props.editor.isEditable) return;
  if (props.editor.view.composing) {
    options(props).onError('请先完成当前输入法输入。');
    return;
  }
  options(props).onEdit({ kind, position: at });
}
function useNodeComposition(props: NodeViewProps) {
  const { controller, pane } = options(props);
  const id = useId();
  const token = pane + '/rich-node/' + id;
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const finishAction = useRef<(() => void) | undefined>(undefined);
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      controller.setComposing(token, false);
      finishAction.current?.();
    },
    [controller, token],
  );
  return {
    onFocusCapture(event: FocusEvent<HTMLElement>) {
      if (!(event.target instanceof Element) || !event.target.matches('input, textarea, select'))
        return;
      if (finishAction.current) return;
      try {
        finishAction.current = controller.beginAction();
      } catch (cause) {
        options(props).onError(cause instanceof Error ? cause.message : '编辑器暂时不可写。');
      }
    },
    onBlur(event: FocusEvent<HTMLElement>) {
      if (
        event.relatedTarget instanceof globalThis.Node &&
        event.currentTarget.contains(event.relatedTarget)
      )
        return;
      finishAction.current?.();
      finishAction.current = undefined;
    },
    onCompositionStartCapture() {
      clearTimeout(timer.current);
      controller.setComposing(token, true);
    },
    onCompositionEndCapture() {
      timer.current = setTimeout(() => controller.setComposing(token, false), 0);
    },
  };
}
function EditButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <Tooltip label={label}>
      <button
        type="button"
        className="rich-node-edit"
        aria-label={label}
        onMouseDown={(event) => event.preventDefault()}
        onClick={onClick}
      >
        <Pencil size={14} aria-hidden="true" />
      </button>
    </Tooltip>
  );
}

function SourceView(props: NodeViewProps) {
  const { node, editor, updateAttributes } = props;
  const { identity } = options(props);
  const { t } = useI18n();
  const composition = useNodeComposition(props);
  const type = node.type.name;
  const source = String(node.attrs.source ?? '');
  const inline = node.isInline;
  const footnote = useEditorState({
    editor,
    selector: ({ editor: current }) => {
      if (type !== 'footnoteReference') return '';
      const index = footnoteIndex(current.state.doc);
      const label = footnoteKey(String(node.attrs.label ?? ''));
      return (
        String(index.order.get(label) ?? '?') + '/' + String(index.definitions.get(label) ?? 0)
      );
    },
  });
  const editLabel =
    type === 'wikiLink' || type === 'wikiEmbed'
      ? t('编辑内部链接', 'Edit internal link')
      : type === 'footnoteReference'
        ? t('编辑脚注标识', 'Edit footnote label')
        : type === 'frontmatter'
          ? t('编辑 YAML 源码', 'Edit YAML source')
          : type === 'comment' || type === 'commentBlock'
            ? t('编辑隐藏内容', 'Edit hidden content')
            : t('编辑公式源码', 'Edit equation source');
  const Tag = inline ? 'span' : 'div';
  return (
    <NodeViewWrapper
      as={Tag}
      className={'rich-editor-source ' + (inline ? 'rich-editor-inline' : 'rich-editor-block')}
      contentEditable={false}
      data-id={node.attrs.id}
      data-block-id={node.attrs.block_id}
      data-rich-kind={type}
      data-reference-occurrence={node.attrs.occurrence_id}
      {...composition}
    >
      {type === 'inlineMath' || type === 'blockMath' ? (
        <MathContent source={source} display={!inline} />
      ) : type === 'wikiLink' || type === 'wikiEmbed' ? (
        <WikiContent
          attrs={node.attrs}
          identity={identity}
          embedded={!inline}
          interaction={editor.isEditable ? 'edit' : 'read'}
          onSelect={() => {
            const at = props.getPos();
            if (typeof at === 'number') editor.chain().focus().setNodeSelection(at).run();
          }}
        />
      ) : type === 'frontmatter' ? (
        <YamlProperties
          source={source}
          objectTypeOptions={options(props).objectTypeOptions}
          onChange={
            editor.isEditable
              ? (next) => {
                  const error = sourceError(next);
                  if (error) throw new Error(error);
                  if (editor.isDestroyed || !editor.isEditable) throw new Error('编辑器已关闭。');
                  updateAttributes({ source: next });
                }
              : undefined
          }
        />
      ) : type === 'footnoteReference' ? (
        <sup
          className={'rich-editor-footnote-ref' + (footnote.endsWith('/1') ? '' : ' is-unresolved')}
          title={
            String(node.attrs.label) +
            (footnote.endsWith('/1')
              ? ''
              : t('：缺少定义或定义重复', ': missing or duplicate definition'))
          }
        >
          [{footnote.split('/')[0]}]
        </sup>
      ) : (
        <HiddenContent source={source} block={!inline} />
      )}
      {editor.isEditable && type !== 'frontmatter' && (
        <EditButton label={editLabel} onClick={() => editNode(props)} />
      )}
    </NodeViewWrapper>
  );
}

function CalloutView(props: NodeViewProps) {
  const { node, editor, updateAttributes } = props;
  const { t } = useI18n();
  const theme = getCallout(node.attrs.callout_type);
  const inside = useEditorState({
    editor,
    selector: ({ editor: current }) => {
      const at = props.getPos();
      return (
        typeof at === 'number' &&
        current.state.selection.from > at &&
        current.state.selection.to < at + node.nodeSize
      );
    },
  });
  const collapsible = node.attrs.collapsed !== null;
  const open = !node.attrs.collapsed || inside;
  return (
    <NodeViewWrapper
      className="rich-editor-callout"
      data-id={node.attrs.id}
      data-block-id={node.attrs.block_id}
      data-callout={node.attrs.callout_type}
      style={{ '--callout-color': theme.color } as CSSProperties}
    >
      <div className="rich-callout-heading" contentEditable={false}>
        {collapsible ? (
          <button
            type="button"
            className="rich-callout-toggle"
            aria-expanded={open}
            aria-label={
              open ? t('收起提示块', 'Collapse callout') : t('展开提示块', 'Expand callout')
            }
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              const at = props.getPos();
              if (typeof at !== 'number' || !editor.isEditable) return;
              if (editor.view.composing) {
                options(props).onError(
                  t('请先完成当前输入法输入。', 'Finish the current input composition first.'),
                );
                return;
              }
              editor.commands.setNodeSelection(at);
              updateAttributes({ collapsed: open });
            }}
          >
            {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
          </button>
        ) : null}
        <CalloutIcon type={node.attrs.callout_type} size={16} />
        <span className="rich-editor-callout-title">
          {node.attrs.title ??
            (theme.type === 'note' && String(node.attrs.callout_type).toLowerCase() !== 'note'
              ? node.attrs.callout_type
              : t(theme.label, theme.en))}
        </span>
        {editor.isEditable && (
          <EditButton label={t('编辑提示块', 'Edit callout')} onClick={() => editNode(props)} />
        )}
      </div>
      <div hidden={!open}>
        <NodeViewContent className="rich-editor-callout-body" />
        {!node.childCount && editor.isEditable && (
          <button
            type="button"
            className="rich-add-content"
            contentEditable={false}
            onClick={() => {
              const at = props.getPos();
              if (typeof at === 'number')
                editor
                  .chain()
                  .insertContentAt(at + 1, {
                    type: 'paragraph',
                    attrs: { id: crypto.randomUUID() },
                  })
                  .focus(at + 2)
                  .run();
            }}
          >
            <Plus size={14} />
            {t('添加内容', 'Add content')}
          </button>
        )}
      </div>
      {editor.isEditable && (
        <button
          type="button"
          className="rich-continue-after"
          contentEditable={false}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            const at = props.getPos();
            if (typeof at !== 'number' || editor.view.composing) return;
            if (
              continueOutsideContainer(
                editor.state,
                (transaction) => editor.view.dispatch(transaction),
                at,
              )
            )
              editor.commands.focus();
          }}
        >
          <CornerDownLeft size={13} aria-hidden="true" />
          {t('在块后继续写', 'Continue after block')}
          <kbd>Ctrl+Enter</kbd>
        </button>
      )}
    </NodeViewWrapper>
  );
}

function FootnoteDefinitionView(props: NodeViewProps) {
  const { node, editor } = props;
  const { t } = useI18n();
  return (
    <NodeViewWrapper
      className="rich-editor-footnote"
      data-id={node.attrs.id}
      data-block-id={node.attrs.block_id}
      data-footnote-label={node.attrs.label}
    >
      <div className="rich-footnote-heading" contentEditable={false}>
        <span>
          {t('脚注', 'Footnote')} [^{node.attrs.label}]
        </span>
        {editor.isEditable && (
          <EditButton
            label={t('重命名脚注标识', 'Rename footnote label')}
            onClick={() => editNode(props)}
          />
        )}
      </div>
      <NodeViewContent />
      {!node.childCount && editor.isEditable && (
        <button
          type="button"
          className="rich-add-content"
          contentEditable={false}
          onClick={() => {
            const at = props.getPos();
            if (typeof at === 'number')
              editor
                .chain()
                .insertContentAt(at + 1, { type: 'paragraph', attrs: { id: crypto.randomUUID() } })
                .focus(at + 2)
                .run();
          }}
        >
          <Plus size={14} />
          {t('添加脚注内容', 'Add footnote content')}
        </button>
      )}
    </NodeViewWrapper>
  );
}

function CodeView(props: NodeViewProps) {
  const { node, editor, updateAttributes } = props;
  const { t } = useI18n();
  const composition = useNodeComposition(props);
  const source = useDeferredValue(node.textContent);
  const mermaid = String(node.attrs.language ?? '').toLowerCase() === 'mermaid';
  return (
    <NodeViewWrapper
      className="rich-editor-code"
      data-id={node.attrs.id}
      data-block-id={node.attrs.block_id}
    >
      <div className="rich-code-heading" contentEditable={false} {...composition}>
        <label>
          <span>{t('代码语言', 'Code language')}</span>
          <input
            aria-label={t('代码语言', 'Code language')}
            value={node.attrs.language ?? ''}
            disabled={!editor.isEditable}
            maxLength={64}
            placeholder="text"
            onChange={(event) => updateAttributes({ language: event.target.value || null })}
          />
        </label>
        {mermaid && <span className="rich-node-caption">Mermaid</span>}
      </div>
      <pre>
        <NodeViewContent<'code'> as="code" />
      </pre>
      {mermaid && (
        <details className="rich-code-preview" open contentEditable={false}>
          <summary>{t('图表预览', 'Diagram preview')}</summary>
          <MermaidContent source={source} />
        </details>
      )}
    </NodeViewWrapper>
  );
}

export function richEditorExtensions(config: RichNodeOptions) {
  const sourceNodes = [
    InlineMath,
    BlockMath,
    WikiLink,
    WikiEmbed,
    FootnoteReference,
    Comment,
    CommentBlock,
    Frontmatter,
  ].map((node) =>
    node.extend({
      addOptions: () => config,
      addNodeView: () => ReactNodeViewRenderer(SourceView),
    }),
  );
  return [
    Highlight,
    TextStyle,
    Superscript,
    Subscript,
    RichBlockAttributes.configure({ onError: config.onError }),
    ...sourceNodes,
    Callout.extend({
      addOptions: () => config,
      addNodeView: () => ReactNodeViewRenderer(CalloutView),
    }),
    FootnoteDefinition.extend({
      addOptions: () => config,
      addNodeView: () => ReactNodeViewRenderer(FootnoteDefinitionView),
    }),
    CodeBlock.extend({
      addOptions() {
        return { ...CodeBlock.options, ...this.parent?.(), ...config };
      },
      addNodeView: () => ReactNodeViewRenderer(CodeView),
    }),
  ];
}
