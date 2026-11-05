import type {
  DocumentNode,
  MarkType,
  NodeAttributes,
  NoteDocument,
  NodeType,
} from '@pkms/contracts';
import { safeColor } from '../rich/colors.ts';
import { footnoteKey } from '../rich/footnotes.ts';

export const PREVIEW_LIMITS = {
  blocks: 8,
  nodes: 120,
  characters: 1200,
  depth: 8,
  rows: 3,
  columns: 4,
  images: 1,
} as const;

type PreviewAttributes = Pick<
  NodeAttributes,
  | 'level'
  | 'start'
  | 'checked'
  | 'blob_hash'
  | 'alt'
  | 'label'
  | 'colspan'
  | 'rowspan'
  | 'source'
  | 'title'
  | 'callout_type'
  | 'collapsed'
  | 'align'
  | 'task_marker'
  | 'language'
> & { text_color?: string; highlight_color?: string; footnote_number?: number };
export type PreviewNode = {
  type: NodeType | 'unsupported';
  attrs?: PreviewAttributes;
  content?: PreviewNode[];
  text?: string;
  marks?: MarkType[];
};
/** A disposable display fragment. `empty` describes only this prefix when truncated. */
export type PreviewDocument = { nodes: PreviewNode[]; truncated: boolean; empty: boolean };

function plainText(node: PreviewNode): string {
  if (node.type === 'text') return node.text ?? '';
  return (node.content ?? []).map(plainText).join('');
}

function normalizeTitle(value: string): string {
  return value.replace(/\s+/g, ' ').trim().toLocaleLowerCase();
}

const structuredPreviewNodes = new Set<PreviewNode['type']>([
  'image',
  'noteReference',
  'wikiLink',
  'wikiEmbed',
  'inlineMath',
  'blockMath',
  'footnoteReference',
  'horizontalRule',
  'table',
  'callout',
  'taskItem',
  'listItem',
  'unsupported',
]);

/**
 * Frontmatter and the duplicate title may be removed from a card preview, but
 * the remaining body still has to distinguish a blank paragraph from visible
 * structure such as an unchecked task marker or an empty callout.
 */
function hasVisiblePreviewContent(node: PreviewNode): boolean {
  if (node.type === 'frontmatter') return false;
  if (node.type === 'text') return Boolean(node.text?.trim());
  if (structuredPreviewNodes.has(node.type)) return true;
  return (node.content ?? []).some(hasVisiblePreviewContent);
}

/**
 * Card previews show the note body, not the document metadata. Frontmatter is
 * removed before looking for the leading H1 that repeats the card title. The
 * editor keeps that H1 in the body, so a card would otherwise print the title
 * twice.
 */
export function withoutLeadingTitle(document: PreviewDocument, title: string): PreviewDocument {
  let nodes = document.nodes.filter((node) => node.type !== 'frontmatter');
  const wanted = normalizeTitle(title);
  const first = nodes[0];
  if (
    wanted &&
    first?.type === 'heading' &&
    (first.attrs?.level ?? 1) === 1 &&
    normalizeTitle(plainText(first)) === wanted
  ) {
    nodes = nodes.slice(1);
  }
  return {
    ...document,
    nodes,
    empty: !nodes.some(hasVisiblePreviewContent),
  };
}
/** The placeholder excerpt is derived from the body text, which starts with the title. */
export function previewExcerpt(preview: string, title: string): string {
  let raw = (preview ?? '').trim();
  const wanted = (title ?? '').trim();
  if (raw.startsWith('---')) {
    const end = raw.indexOf('\n---', 3);
    if (end >= 0) raw = raw.slice(end + 4).trim();
  }
  if (!raw || !wanted) return raw;
  if (!raw.toLocaleLowerCase().startsWith(wanted.toLocaleLowerCase())) return raw;
  return raw.slice(wanted.length).trim();
}

const containers = new Set<NodeType>([
  'paragraph',
  'heading',
  'bulletList',
  'orderedList',
  'listItem',
  'taskList',
  'taskItem',
  'blockquote',
  'codeBlock',
  'tableCell',
  'tableHeader',
  'callout',
  'footnoteDefinition',
]);
const allowedMarks = new Set<MarkType>([
  'bold',
  'italic',
  'strike',
  'code',
  'link',
  'underline',
  'superscript',
  'subscript',
  'highlight',
  'textStyle',
]);

function inlineFormatting(node: DocumentNode): Pick<PreviewNode, 'marks' | 'attrs'> {
  const source = (node.marks ?? []).slice(0, 12);
  const marks = [
    ...new Set(source.map((mark) => mark.type).filter((type) => allowedMarks.has(type))),
  ];
  const text_color = safeColor(source.find((mark) => mark.type === 'textStyle')?.attrs?.color);
  const highlight_color = safeColor(source.find((mark) => mark.type === 'highlight')?.attrs?.color);
  return {
    ...(marks.length ? { marks } : {}),
    ...(text_color || highlight_color
      ? {
          attrs: {
            ...(text_color ? { text_color } : {}),
            ...(highlight_color ? { highlight_color } : {}),
          },
        }
      : {}),
  };
}

export function projectPreview(document: NoteDocument): PreviewDocument {
  let visited = 0,
    remaining = PREVIEW_LIMITS.characters as number,
    images = 0;
  let truncated = false,
    empty = true;
  const footnotes = new Map<string, number>();
  function takeText(value: string | null | undefined, maximum = remaining): string {
    if (!value) return '';
    let result = '',
      offset = 0,
      count = 0;
    while (offset < value.length && remaining > 0 && count < maximum) {
      const point = value.codePointAt(offset)!;
      result += String.fromCodePoint(point);
      offset += point > 0xffff ? 2 : 1;
      --remaining;
      ++count;
    }
    if (offset < value.length) truncated = true;
    if (result.trim()) empty = false;
    return result;
  }
  function reserve(depth: number): boolean {
    if (visited >= PREVIEW_LIMITS.nodes || depth > PREVIEW_LIMITS.depth || remaining === 0) {
      truncated = true;
      return false;
    }
    ++visited;
    return true;
  }
  function children(
    nodes: DocumentNode[] | undefined,
    depth: number,
    limit: number = PREVIEW_LIMITS.nodes,
  ): PreviewNode[] {
    const result: PreviewNode[] = [];
    if (!nodes) return result;
    for (let i = 0; i < nodes.length; ++i) {
      if (
        i >= limit ||
        visited >= PREVIEW_LIMITS.nodes ||
        remaining === 0 ||
        depth > PREVIEW_LIMITS.depth
      ) {
        truncated = true;
        break;
      }
      const next = visit(nodes[i], depth);
      if (next) result.push(next);
    }
    return result;
  }
  function table(node: DocumentNode, depth: number): PreviewNode {
    const rows = node.content ?? [],
      output: PreviewNode[] = [];
    const occupiedUntil = Array<number>(PREVIEW_LIMITS.columns).fill(0);
    if (rows.length > PREVIEW_LIMITS.rows) truncated = true;
    for (let r = 0; r < Math.min(rows.length, PREVIEW_LIMITS.rows); ++r) {
      if (!reserve(depth + 1)) break;
      const row = rows[r],
        cells = row.content ?? [],
        projected: PreviewNode[] = [];
      let column = 0;
      for (let c = 0; c < cells.length; ++c) {
        while (column < PREVIEW_LIMITS.columns && occupiedUntil[column] > r) ++column;
        if (
          column >= PREVIEW_LIMITS.columns ||
          visited >= PREVIEW_LIMITS.nodes ||
          remaining === 0
        ) {
          truncated = true;
          break;
        }
        const cell = cells[c];
        let free = 0;
        while (column + free < PREVIEW_LIMITS.columns && occupiedUntil[column + free] <= r) ++free;
        const rawColspan = positive(cell.attrs?.colspan),
          rawRowspan = positive(cell.attrs?.rowspan);
        const colspan = Math.min(rawColspan, free),
          rowspan = Math.min(rawRowspan, PREVIEW_LIMITS.rows - r);
        if (colspan !== rawColspan || rowspan !== rawRowspan) truncated = true;
        const value = visit(cell, depth + 2);
        if (!value) break;
        projected.push({ ...value, attrs: { ...value.attrs, colspan, rowspan } });
        for (let i = 0; i < colspan; ++i) occupiedUntil[column + i] = r + rowspan;
        column += colspan;
      }
      output.push({ type: 'tableRow', content: projected });
    }
    empty = false;
    return { type: 'table', content: output };
  }
  function visit(node: DocumentNode, depth: number): PreviewNode | null {
    if (!reserve(depth)) return null;
    const { type } = node;
    if (type === 'comment' || type === 'commentBlock') return null;
    if (type === 'text') {
      const text = takeText(node.text);
      return { type, text, ...inlineFormatting(node) };
    }
    if (type === 'image') {
      if (images >= PREVIEW_LIMITS.images) {
        truncated = true;
        return null;
      }
      ++images;
      empty = false;
      const hash = node.attrs?.blob_hash;
      return {
        type,
        attrs: {
          blob_hash: hash && /^[a-f0-9]{64}$/.test(hash) ? hash : '',
          alt: takeText(node.attrs?.alt ?? node.attrs?.original_name, 120),
        },
      };
    }
    if (type === 'noteReference' || type === 'wikiLink' || type === 'wikiEmbed') {
      empty = false;
      const format = inlineFormatting(node);
      return {
        type,
        ...format,
        attrs: {
          ...format.attrs,
          label: takeText(node.attrs?.label ?? node.attrs?.wiki_target, 240),
        },
      };
    }
    if (type === 'inlineMath' || type === 'blockMath' || type === 'frontmatter') {
      empty = false;
      const format = inlineFormatting(node);
      return {
        type,
        ...format,
        attrs: {
          ...format.attrs,
          source: takeText(node.attrs?.source, type === 'frontmatter' ? 400 : 600),
        },
      };
    }
    if (type === 'footnoteReference') {
      empty = false;
      const label = takeText(node.attrs?.label, 120);
      const key = footnoteKey(label);
      if (!footnotes.has(key)) footnotes.set(key, footnotes.size + 1);
      const format = inlineFormatting(node);
      return {
        type,
        ...format,
        attrs: { ...format.attrs, label, footnote_number: footnotes.get(key) },
      };
    }
    if (type === 'hardBreak' || type === 'horizontalRule') {
      if (type === 'horizontalRule') empty = false;
      return { type, ...inlineFormatting(node) };
    }
    if (type === 'table') return table(node, depth);
    if (!containers.has(type)) {
      empty = false;
      return { type: 'unsupported' };
    }
    const attrs: PreviewAttributes = {};
    if (type === 'heading') attrs.level = Math.min(6, positive(node.attrs?.level));
    if (type === 'orderedList') attrs.start = Math.min(1_000_000, positive(node.attrs?.start));
    if (type === 'taskItem') attrs.checked = node.attrs?.checked === true;
    if (type === 'taskItem' && node.attrs?.task_marker)
      attrs.task_marker = takeText(node.attrs.task_marker, 4);
    if (type === 'tableCell' || type === 'tableHeader') {
      if (['left', 'center', 'right'].includes(node.attrs?.align ?? ''))
        attrs.align = node.attrs!.align;
    }
    if (type === 'codeBlock' && node.attrs?.language?.toLowerCase() === 'mermaid')
      attrs.language = 'mermaid';
    if (type === 'callout') {
      attrs.callout_type = takeText(node.attrs?.callout_type ?? 'note', 40);
      attrs.title = takeText(node.attrs?.title, 120);
      if (typeof node.attrs?.collapsed === 'boolean') attrs.collapsed = node.attrs.collapsed;
      empty = false;
    }
    if (type === 'footnoteDefinition') {
      attrs.label = takeText(node.attrs?.label, 120);
      attrs.footnote_number = footnotes.get(footnoteKey(attrs.label));
    }
    // List markers and task state remain visible even without any text.
    if (type === 'taskItem' || type === 'listItem') empty = false;
    return {
      type,
      ...(Object.keys(attrs).length ? { attrs } : {}),
      content:
        type === 'callout' && attrs.collapsed === true ? [] : children(node.content, depth + 1),
    };
  }
  const nodes = children(document.content, 1, PREVIEW_LIMITS.blocks);
  return { nodes, truncated, empty };
}

function positive(value: number | null | undefined): number {
  return value && Number.isSafeInteger(value) && value > 0 ? value : 1;
}
