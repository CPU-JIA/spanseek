import { useId, useMemo, useRef, type ReactNode } from 'react';
import type { DocumentNode, Note, NoteDocument, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { ReferenceChip } from '../editor/reference-chip';
import { RichDocumentContext, useRichDocument } from './document-context';
import { InlineMarks } from './InlineMarks';
import { footnoteKey } from './footnotes';
import { MathContent } from './MathContent';
import { MermaidContent } from './MermaidContent';
import { YamlProperties } from './YamlProperties';
import { WikiContent } from './WikiContent';
import { CalloutContent } from './CalloutContent';
import { HiddenContent } from './HiddenContent';
import { DocumentImage } from './DocumentImage';
import { PASTE_MARKERS, pasteAttributes } from './paste-markers';
import { richText } from './wiki-model';
import './rich-content.css';

type ScopeProps = {
  note: Note | null;
  identity: WorkspaceIdentity;
  imageImports?: Record<string, string>;
  depth?: number;
  visited?: ReadonlySet<string>;
};
export function RichDocumentScope({
  note,
  identity,
  imageImports,
  depth = 0,
  visited,
  children,
}: ScopeProps & { children: ReactNode }) {
  const trail = useMemo(() => visited ?? new Set(note ? [note.id] : []), [visited, note?.id]);
  const value = useMemo(
    () => ({
      note,
      identity,
      imageImports,
      depth,
      visited: trail,
      renderNote: (nested: Note, nestedDepth: number, nestedVisited: ReadonlySet<string>) => (
        <DocumentContent
          note={nested}
          identity={identity}
          depth={nestedDepth}
          visited={nestedVisited}
        />
      ),
    }),
    [note, identity, imageImports, depth, trail],
  );
  return <RichDocumentContext.Provider value={value}>{children}</RichDocumentContext.Provider>;
}
export function DocumentContent(props: ScopeProps & { note: Note }) {
  return (
    <RichDocumentScope {...props}>
      <DocumentBody
        key={props.note.id + '/' + props.note.revision_id}
        document={props.note.document}
      />
    </RichDocumentScope>
  );
}
/** Unsaved Markdown has no note/revision identity and cannot request its own attachments. */
export function StandaloneDocumentContent({
  document,
  identity,
  renderText,
}: {
  document: NoteDocument;
  identity: WorkspaceIdentity;
  renderText?: (text: string) => ReactNode;
}) {
  return (
    <RichDocumentScope note={null} identity={identity}>
      <DocumentBody document={document} renderText={renderText} />
    </RichDocumentScope>
  );
}
function DocumentBody({
  document,
  renderText,
}: {
  document: NoteDocument;
  renderText?: (text: string) => ReactNode;
}) {
  const { t } = useI18n();
  const { note, identity, imageImports } = useRichDocument()!;
  const root = useRef<HTMLDivElement>(null);
  const footnoteId = useId();
  const footnotes = useMemo(() => {
    const definitions = new Map<string, DocumentNode[]>(),
      numbers = new Map<string, number>();
    const visit = (nodes: DocumentNode[]) =>
      nodes.forEach((node) => {
        if (node.type === 'footnoteReference' && node.attrs?.label) {
          const label = footnoteKey(node.attrs.label);
          if (!numbers.has(label)) numbers.set(label, numbers.size + 1);
        }
        if (node.type === 'footnoteDefinition') {
          const label = footnoteKey(node.attrs?.label ?? '');
          definitions.set(label, [...(definitions.get(label) ?? []), node]);
        }
        if (node.content) visit(node.content);
      });
    visit(document.content);
    return { definitions, numbers };
  }, [document]);
  function footnoteJump(label: string, back = false) {
    const attribute = back ? 'data-footnote-reference' : 'data-footnote-label';
    const element = Array.from(
      root.current?.querySelectorAll<HTMLElement>(`[${attribute}]`) ?? [],
    ).find(
      (item) =>
        item.getAttribute(attribute) === label &&
        item.closest('.rich-document-content') === root.current,
    );
    element?.scrollIntoView({ block: 'center', behavior: 'instant' });
    element?.focus({ preventScroll: true });
  }
  function render(node: DocumentNode, index: number, literal = false): ReactNode {
    const key = node.attrs?.id ?? node.attrs?.occurrence_id ?? index;
    const anchor = {
      'data-id': node.attrs?.id,
      'data-block-id': node.attrs?.block_id ?? undefined,
    };
    if (node.type === 'footnoteDefinition') return null;
    if (node.type === 'comment' || node.type === 'commentBlock')
      return node.type === 'comment' ? (
        <InlineMarks key={key} marks={node.marks} atom>
          <HiddenContent source={node.attrs?.source ?? ''} />
        </InlineMarks>
      ) : (
        <div {...anchor} key={key}>
          <HiddenContent source={node.attrs?.source ?? ''} block />
        </div>
      );
    const content = node.content?.map((child, childIndex) =>
      render(child, childIndex, literal || node.type === 'codeBlock'),
    );
    if (node.type === 'text') {
      const preserveLiteral =
        literal || node.marks?.some((mark) => mark.type === 'code' || mark.type === 'link');
      return (
        <span key={key}>
          <InlineMarks marks={node.marks}>
            {renderText && !preserveLiteral ? renderText(node.text ?? '') : node.text}
          </InlineMarks>
        </span>
      );
    }
    switch (node.type) {
      case 'paragraph':
        return (
          <p {...anchor} key={key}>
            {content?.length ? content : <br />}
          </p>
        );
      case 'heading': {
        const Tag = `h${Math.max(1, Math.min(6, node.attrs?.level ?? 1))}` as
          'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6';
        return (
          <Tag {...anchor} key={key}>
            {content}
          </Tag>
        );
      }
      case 'blockquote':
        return (
          <blockquote {...anchor} key={key}>
            {content}
          </blockquote>
        );
      case 'bulletList':
      case 'taskList':
        return (
          <ul {...anchor} key={key}>
            {content}
          </ul>
        );
      case 'orderedList':
        return (
          <ol {...anchor} key={key} start={node.attrs?.start ?? 1}>
            {content}
          </ol>
        );
      case 'listItem':
        return (
          <li {...anchor} key={key}>
            {content}
          </li>
        );
      case 'taskItem':
        return (
          <li {...anchor} key={key} className="read-task">
            <input type="checkbox" checked={node.attrs?.checked ?? false} readOnly tabIndex={-1} />
            {node.attrs?.task_marker && !/^[ xX]$/.test(node.attrs.task_marker) && (
              <span className="rich-task-marker">{node.attrs.task_marker}</span>
            )}
            {content}
          </li>
        );
      case 'codeBlock':
        return node.attrs?.language?.toLowerCase() === 'mermaid' ? (
          <div {...anchor} key={key}>
            <MermaidContent source={(node.content ?? []).map(richText).join('')} />
          </div>
        ) : (
          <pre {...anchor} key={key}>
            <code>{content}</code>
          </pre>
        );
      case 'hardBreak':
        return (
          <InlineMarks key={key} marks={node.marks}>
            <br />
          </InlineMarks>
        );
      case 'horizontalRule':
        return <hr {...anchor} key={key} />;
      case 'image':
        return (
          // The editor parses `figure[data-pkms-image]` with these attributes, so
          // the read view carries the same markers: copying from reading mode and
          // pasting into the editor keeps the image instead of dropping it.
          <figure
            {...anchor}
            key={key}
            data-pkms-image="true"
            {...pasteAttributes(node.attrs, PASTE_MARKERS.image.attributes)}
            title={node.attrs?.title ?? undefined}
          >
            {note ? (
              <DocumentImage
                identity={identity}
                revision={note.revision_id}
                hash={node.attrs?.blob_hash ?? ''}
                importCommand={imageImports?.[node.attrs?.blob_hash ?? '']}
                alt={node.attrs?.alt ?? node.attrs?.original_name ?? t('笔记图片', 'Note image')}
              />
            ) : (
              <span className="rich-caption">
                {node.attrs?.alt || node.attrs?.original_name || t('图片', 'Image')} —{' '}
                {t('图片尚未关联到已保存的笔记。', 'This image is not linked to a saved note.')}
              </span>
            )}
            {node.attrs?.title ? (
              <figcaption className="rich-caption">{node.attrs.title}</figcaption>
            ) : null}
          </figure>
        );
      case 'noteReference':
        return (
          <InlineMarks key={key} marks={node.marks} atom>
            <span
              data-note-reference="true"
              {...pasteAttributes(node.attrs, PASTE_MARKERS.noteReference.attributes)}
            >
              <ReferenceChip attrs={node.attrs ?? {}} identity={identity} />
            </span>
          </InlineMarks>
        );
      case 'wikiLink':
        return (
          <InlineMarks key={key} marks={node.marks} atom>
            <span
              data-wiki-link="true"
              data-wiki-target={node.attrs?.wiki_target ?? ''}
              {...pasteAttributes(node.attrs, PASTE_MARKERS.wikiLink.attributes)}
            >
              <WikiContent attrs={node.attrs ?? {}} identity={identity} embedded={false} />
            </span>
          </InlineMarks>
        );
      case 'wikiEmbed':
        return (
          <div
            {...anchor}
            key={key}
            data-wiki-embed="true"
            data-wiki-target={node.attrs?.wiki_target ?? ''}
            {...pasteAttributes(node.attrs, PASTE_MARKERS.wikiEmbed.attributes)}
          >
            <WikiContent attrs={node.attrs ?? {}} identity={identity} embedded />
          </div>
        );
      case 'inlineMath':
        return (
          <InlineMarks key={key} marks={node.marks} atom>
            <MathContent source={node.attrs?.source ?? ''} display={false} />
          </InlineMarks>
        );
      case 'blockMath':
        return (
          <div {...anchor} key={key}>
            <MathContent source={node.attrs?.source ?? ''} display />
          </div>
        );
      case 'callout':
        return (
          <div {...anchor} key={key}>
            <CalloutContent
              type={node.attrs?.callout_type}
              title={node.attrs?.title}
              collapsed={node.attrs?.collapsed}
            >
              {content}
            </CalloutContent>
          </div>
        );
      case 'frontmatter':
        return (
          <div {...anchor} key={key}>
            <YamlProperties source={node.attrs?.source ?? ''} />
          </div>
        );
      case 'footnoteReference': {
        const label = footnoteKey(node.attrs?.label ?? ''),
          definitions = footnotes.definitions.get(label);
        const number = footnotes.numbers.get(label);
        return (
          <InlineMarks key={key} marks={node.marks} atom>
            <sup className="rich-footnote-reference">
              <button
                type="button"
                data-footnote-reference={label}
                aria-label={`${t('脚注', 'Footnote')} ${number ?? label}${definitions?.length === 1 ? '' : t('，定义缺失或重复', ', missing or duplicate definition')}`}
                onClick={() => footnoteJump(label)}
                disabled={definitions?.length !== 1}
              >
                [{number ?? label}]
              </button>
              {!definitions && (
                <span title={t('脚注定义缺失', 'Missing footnote definition')}>?</span>
              )}
            </sup>
          </InlineMarks>
        );
      }
      case 'table':
        return (
          <table {...anchor} key={key}>
            <tbody>{content}</tbody>
          </table>
        );
      case 'tableRow':
        return (
          <tr {...anchor} key={key}>
            {content}
          </tr>
        );
      case 'tableCell':
      case 'tableHeader': {
        const Tag = node.type === 'tableCell' ? 'td' : 'th';
        const align = node.attrs?.align;
        return (
          <Tag
            {...anchor}
            key={key}
            colSpan={node.attrs?.colspan ?? 1}
            rowSpan={node.attrs?.rowspan ?? 1}
            style={{
              textAlign:
                align === 'left' || align === 'center' || align === 'right' ? align : undefined,
            }}
          >
            {content}
          </Tag>
        );
      }
      default:
        return (
          <p role="alert" key={key}>
            {t('此内容格式无法显示。', 'This content format cannot be displayed.')}
          </p>
        );
    }
  }
  return (
    <div className="rich-document-content" ref={root}>
      {document.content.map((node, index) => render(node, index))}
      {footnotes.definitions.size > 0 && (
        <section className="rich-footnotes" aria-label={t('脚注', 'Footnotes')}>
          <ol>
            {[...footnotes.definitions]
              .sort(
                ([a], [b]) =>
                  (footnotes.numbers.get(a) ?? Infinity) - (footnotes.numbers.get(b) ?? Infinity),
              )
              .map(([label, definitions]) => (
                <li
                  key={label}
                  id={`${footnoteId}-${encodeURIComponent(label)}`}
                  tabIndex={-1}
                  data-footnote-label={label}
                  value={footnotes.numbers.get(label)}
                >
                  {!footnotes.numbers.has(label) && (
                    <span className="rich-caption">
                      {t('未引用脚注', 'Unreferenced footnote')} [
                      {definitions[0].attrs?.label ?? label}]
                    </span>
                  )}
                  {definitions.length > 1 && (
                    <p role="alert">
                      {t(
                        '脚注标签重复，已保留所有定义。',
                        'Duplicate footnote label; all definitions are preserved.',
                      )}{' '}
                      [{label}]
                    </p>
                  )}
                  {definitions.map((definition, index) => (
                    <div key={definition.attrs?.id ?? index} data-id={definition.attrs?.id}>
                      {definition.content?.map((node, childIndex) => render(node, childIndex))}
                    </div>
                  ))}
                  {footnotes.numbers.has(label) && (
                    <button
                      type="button"
                      className="rich-footnote-back"
                      onClick={() => footnoteJump(label, true)}
                      aria-label={t('返回正文引用', 'Back to reference')}
                    >
                      ↩
                    </button>
                  )}
                </li>
              ))}
          </ol>
        </section>
      )}
    </div>
  );
}
