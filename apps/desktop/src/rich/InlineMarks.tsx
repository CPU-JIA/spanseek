import type { CSSProperties, ReactNode } from 'react';
import type { DocumentMark } from '@pkms/contracts';
import { DEFAULT_HIGHLIGHT, highlightInk, safeColor } from './colors';
import { DocumentLink } from './DocumentLink';

/** Text and inline atoms share the same persisted marks in both readers. */
export function InlineMarks({
  children,
  marks = [],
  atom = false,
  linkClassName = 'document-link',
}: {
  children: ReactNode;
  marks?: DocumentMark[];
  atom?: boolean;
  linkClassName?: string;
}) {
  if (!marks.length) return <>{children}</>;
  let value = children;
  const explicit = safeColor(marks.find((mark) => mark.type === 'textStyle')?.attrs?.color);
  const types = new Set(marks.map((mark) => mark.type));
  const linkColor =
    explicit ??
    (types.has('highlight')
      ? highlightInk(
          safeColor(marks.find((mark) => mark.type === 'highlight')?.attrs?.color) ??
            DEFAULT_HIGHLIGHT,
        )
      : undefined);
  for (const mark of marks) {
    switch (mark.type) {
      case 'bold':
        value = <strong>{value}</strong>;
        break;
      case 'italic':
        value = <em>{value}</em>;
        break;
      case 'strike':
        value = <s>{value}</s>;
        break;
      case 'underline':
        value = <u>{value}</u>;
        break;
      case 'superscript':
        value = <sup>{value}</sup>;
        break;
      case 'subscript':
        value = <sub>{value}</sub>;
        break;
      case 'code':
        value = <code>{value}</code>;
        break;
      case 'highlight': {
        const background = safeColor(mark.attrs?.color) ?? DEFAULT_HIGHLIGHT;
        value = (
          <mark
            style={{ backgroundColor: background, color: explicit ?? highlightInk(background) }}
          >
            {value}
          </mark>
        );
        break;
      }
      case 'textStyle':
        value = <span style={{ color: safeColor(mark.attrs?.color) }}>{value}</span>;
        break;
      case 'link':
        value =
          linkClassName === 'gallery-preview-link' ? (
            <span
              className={linkClassName}
              title={mark.attrs?.href ?? undefined}
              style={{ color: linkColor }}
            >
              {value}
            </span>
          ) : (
            <DocumentLink
              href={mark.attrs?.href ?? ''}
              title={mark.attrs?.title}
              className={linkClassName}
              style={{ color: linkColor }}
              atom={atom}
            >
              {value}
            </DocumentLink>
          );
        break;
    }
  }
  if (!atom) return <>{value}</>;
  const decoration =
    [types.has('underline') && 'underline', types.has('strike') && 'line-through']
      .filter(Boolean)
      .join(' ') || 'none';
  return (
    <span
      className="rich-inline-atom"
      data-colored={explicit || types.has('highlight') ? true : undefined}
      data-highlighted={types.has('highlight') ? true : undefined}
      style={{ '--rich-inline-decoration': decoration } as CSSProperties}
    >
      {value}
    </span>
  );
}
