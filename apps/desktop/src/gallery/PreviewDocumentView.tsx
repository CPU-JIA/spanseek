import type { ReactNode } from 'react';
import type { DocumentMark, WorkspaceIdentity } from '@pkms/contracts';
import { imageUrl } from '../api';
import { useI18n } from '../i18n';
import { withoutLeadingTitle, type PreviewDocument, type PreviewNode } from './preview-document';
import { InlineMarks } from '../rich/InlineMarks';
import { getCallout } from '../rich/callouts';
import { MathContent } from '../rich/MathContent';

/** Pure card presentation: no editor, reference resolution, navigation or form controls. */
export function PreviewDocumentView({
  document,
  title,
  identity,
  revision,
  imageFailed,
  onImageError,
}: {
  document: PreviewDocument;
  title: string;
  identity: WorkspaceIdentity;
  revision: string;
  imageFailed: boolean;
  onImageError: () => void;
}) {
  const { t } = useI18n();
  const body = withoutLeadingTitle(document, title);
  function marked(node: PreviewNode, value: ReactNode) {
    const marks: DocumentMark[] = (node.marks ?? []).map((type) => ({
      type,
      ...(type === 'textStyle' && node.attrs?.text_color
        ? { attrs: { color: node.attrs.text_color } }
        : type === 'highlight' && node.attrs?.highlight_color
          ? { attrs: { color: node.attrs.highlight_color } }
          : {}),
    }));
    return (
      <InlineMarks marks={marks} atom={node.type !== 'text'} linkClassName="gallery-preview-link">
        {value}
      </InlineMarks>
    );
  }
  function render(node: PreviewNode, index: number): ReactNode {
    const content = node.content?.map(render);
    if (node.type === 'text') {
      return <span key={index}>{marked(node, node.text)}</span>;
    }
    switch (node.type) {
      case 'paragraph':
        return <p key={index}>{content?.length ? content : <br />}</p>;
      case 'heading': {
        const Tag = `h${node.attrs?.level ?? 1}` as 'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6';
        return <Tag key={index}>{content}</Tag>;
      }
      case 'callout': {
        const callout = getCallout(node.attrs?.callout_type);
        return (
          <aside
            key={index}
            className="gallery-preview-callout"
            style={{ borderColor: callout.color }}
          >
            <strong style={{ color: callout.color }}>
              {node.attrs?.collapsed === true ? '▸ ' : ''}
              {node.attrs?.title || t(callout.label, callout.en)}
            </strong>
            {content}
          </aside>
        );
      }
      case 'inlineMath':
        return (
          <span key={index}>
            {marked(node, <MathContent source={node.attrs?.source ?? ''} display={false} />)}
          </span>
        );
      case 'blockMath':
        return <MathContent key={index} source={node.attrs?.source ?? ''} display />;
      case 'frontmatter':
        return (
          <pre key={index} className="gallery-preview-properties">
            <code>{node.attrs?.source}</code>
          </pre>
        );
      case 'footnoteReference':
        return (
          <span key={index}>
            {marked(node, <sup>[{node.attrs?.footnote_number ?? node.attrs?.label}]</sup>)}
          </span>
        );
      case 'footnoteDefinition':
        return (
          <div key={index} className="gallery-preview-footnote">
            <sup>[{node.attrs?.footnote_number ?? node.attrs?.label}]</sup>
            {content}
          </div>
        );
      case 'blockquote':
        return <blockquote key={index}>{content}</blockquote>;
      case 'bulletList':
        return <ul key={index}>{content}</ul>;
      case 'orderedList':
        return (
          <ol key={index} start={node.attrs?.start ?? 1}>
            {content}
          </ol>
        );
      case 'listItem':
        return <li key={index}>{content}</li>;
      case 'taskList':
        return (
          <ul key={index} className="gallery-preview-tasks">
            {content}
          </ul>
        );
      case 'taskItem':
        return (
          <li key={index}>
            <span className="gallery-preview-check" data-checked={node.attrs?.checked === true}>
              {node.attrs?.task_marker && !/^[ xX]$/.test(node.attrs.task_marker)
                ? node.attrs.task_marker
                : node.attrs?.checked
                  ? '✓'
                  : ''}
            </span>
            <span>{content}</span>
          </li>
        );
      case 'hardBreak':
        return <span key={index}>{marked(node, <br />)}</span>;
      case 'horizontalRule':
        return <hr key={index} />;
      case 'codeBlock':
        return (
          <pre key={index}>
            {node.attrs?.language === 'mermaid' && (
              <span className="gallery-preview-unavailable">Mermaid</span>
            )}
            <code>{content}</code>
          </pre>
        );
      case 'noteReference':
      case 'wikiLink':
      case 'wikiEmbed':
        return (
          <span key={index}>
            {marked(
              node,
              <span className="gallery-preview-reference">
                {node.type === 'wikiEmbed' ? '↳ ' : ''}
                {node.attrs?.label || t('笔记引用', 'Note reference')}
              </span>,
            )}
          </span>
        );
      case 'image':
        return (
          <PreviewImage
            key={index}
            identity={identity}
            revision={revision}
            hash={node.attrs?.blob_hash ?? ''}
            alt={node.attrs?.alt ?? ''}
            failed={imageFailed}
            onError={onImageError}
          />
        );
      case 'table':
        return (
          <table key={index}>
            <tbody>{content}</tbody>
          </table>
        );
      case 'tableRow':
        return <tr key={index}>{content}</tr>;
      case 'tableCell':
        return (
          <td
            key={index}
            colSpan={node.attrs?.colspan ?? 1}
            rowSpan={node.attrs?.rowspan ?? 1}
            style={{ textAlign: node.attrs?.align ?? undefined }}
          >
            {content}
          </td>
        );
      case 'tableHeader':
        return (
          <th
            key={index}
            colSpan={node.attrs?.colspan ?? 1}
            rowSpan={node.attrs?.rowspan ?? 1}
            style={{ textAlign: node.attrs?.align ?? undefined }}
          >
            {content}
          </th>
        );
      default:
        return (
          <span key={index} className="gallery-preview-unavailable">
            {t('此内容格式无法预览', 'This content cannot be previewed')}
          </span>
        );
    }
  }
  return (
    <span className="gallery-preview-canvas gallery-preview-body" aria-hidden="true">
      <h1 className="gallery-preview-title">{title || t('未命名笔记', 'Untitled note')}</h1>
      {body.empty ? (
        <span className="gallery-preview-unavailable">
          {document.truncated
            ? t(
                '预览未包含正文，打开笔记查看',
                'Content is outside this preview. Open the note to read it.',
              )
            : t('空白笔记', 'Empty note')}
        </span>
      ) : (
        body.nodes.map(render)
      )}
    </span>
  );
}

function PreviewImage({
  identity,
  revision,
  hash,
  alt,
  failed,
  onError,
}: {
  identity: WorkspaceIdentity;
  revision: string;
  hash: string;
  alt: string;
  failed: boolean;
  onError: () => void;
}) {
  const { t } = useI18n();
  return failed ? (
    <span className="gallery-preview-unavailable">
      {t('缩略图读取失败', 'Thumbnail unavailable')}
    </span>
  ) : (
    <img
      src={imageUrl(identity, { revision, hash, thumbnail: true })}
      alt={alt}
      loading="lazy"
      decoding="async"
      draggable={false}
      onError={() => {
        onError();
      }}
    />
  );
}
