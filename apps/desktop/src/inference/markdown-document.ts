import type { DocumentMark, DocumentNode, NoteDocument, NodeAttributes } from '@pkms/contracts';
import type { Definition, Nodes, PhrasingContent, RootContent } from 'mdast';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import remarkFrontmatter from 'remark-frontmatter';
import { remarkOfm, wikiParts } from './markdown-ofm.ts';
import { OFM_HIGHLIGHT_COLORS } from '../rich/colors.ts';
import { footnoteKey as labelKey } from '../rich/footnotes.ts';
import {
  combineMarkdownMarks,
  htmlFormatToken,
  htmlText,
  markdownPlainString,
  safeMarkdownLink,
} from './markdown-html.ts';

const parser = unified()
  .use(remarkParse)
  .use(remarkGfm)
  .use(remarkMath)
  .use(remarkFrontmatter, ['yaml', { type: 'ofmCommentBlock', fence: '%%', anywhere: true }])
  .use(remarkOfm);
const encoder = new TextEncoder();
export const MARKDOWN_LIMITS = { bytes: 4 * 1024 * 1024, nodes: 50_000, sourceBytes: 65_536 };

export type PreservedSyntax =
  | 'unsupported'
  | 'heading'
  | 'link'
  | 'image'
  | 'code'
  | 'alignment'
  | 'depth'
  | 'html'
  | 'frontmatter'
  | 'anchor'
  | 'footnote'
  | 'wiki'
  | 'math'
  | 'bounds';
export interface MarkdownDocument {
  document: NoteDocument;
  preserved: PreservedSyntax[];
  unresolvedCitations?: number[];
}
export interface MarkdownOptions {
  uuid?: () => string;
  /** Appends/pastes must opt out unless the insertion is the first document block. */
  allowFrontmatter?: boolean;
  /** Existing content is read only: protects labels and restores already captured assets. */
  contextDocument?: NoteDocument;
}

class Unsupported extends Error {
  readonly reason: PreservedSyntax;
  constructor(reason: PreservedSyntax) {
    super(reason);
    this.reason = reason;
  }
}
const characters = (value: string) => [...value].length;
const hasControl = (value: string) => /[\x00-\x1f\x7f-\x9f]/.test(value);

/** The one DOM-free conversion path for imports, AI proposals and Markdown paste.
 * Source that cannot be represented safely stays visible; it is never executed or downloaded.
 */
export function markdownDocument(source: string, options: MarkdownOptions = {}): MarkdownDocument {
  if (encoder.encode(source).length > MARKDOWN_LIMITS.bytes)
    throw new RangeError('Markdown exceeds the document size limit.');
  if (source.includes('\0'))
    throw new RangeError('Markdown contains a NUL character that cannot be preserved.');
  const uuid = options.uuid ?? (() => crypto.randomUUID());
  const origins = new WeakMap<Nodes, string>();
  const preserved = new Set<PreservedSyntax>();
  const definitions = new Map<string, Definition>();
  const usedDefinitions = new Set<string>();
  const footnoteLabels = new Map<string, string>();
  const occupiedFootnotes = new Set<string>();
  const occupiedAnchors = new Set<string>();
  const emittedFootnotes = new Set<string>();
  const inlineFootnotes: DocumentNode[] = [];
  const capturedImages = new Map<string, DocumentNode>();
  const capturedReferences = new Map<string, DocumentNode>();
  let created = 0,
    inlineNote = 0;

  function scanContext(nodes: DocumentNode[]) {
    for (const node of nodes) {
      if (node.attrs?.block_id) occupiedAnchors.add(node.attrs.block_id);
      if (node.type === 'footnoteDefinition' && node.attrs?.label)
        occupiedFootnotes.add(labelKey(node.attrs.label));
      if (node.type === 'image' && node.attrs?.blob_hash)
        capturedImages.set(node.attrs.blob_hash, node);
      if (node.type === 'noteReference') capturedReferences.set(referenceUrl(node.attrs), node);
      scanContext(node.content ?? []);
    }
  }
  if (options.contextDocument) scanContext(options.contextDocument.content);

  function parse(value: string) {
    const result = parser.parse(value);
    function register(node: Nodes) {
      origins.set(node, value);
      if ('children' in node) node.children.forEach(register);
    }
    register(result);
    return result;
  }
  const root = parse(source);
  function uniqueFootnote(label: string): string {
    if (occupiedFootnotes.size >= 4096)
      throw new RangeError('Markdown contains too many footnotes.');
    let candidate = label,
      suffix = 1;
    while (occupiedFootnotes.has(labelKey(candidate)))
      candidate = `${[...label].slice(0, 490).join('')}-${++suffix}`;
    occupiedFootnotes.add(labelKey(candidate));
    return candidate;
  }
  function collect(node: Nodes) {
    if (node.type === 'definition' && !definitions.has(node.identifier))
      definitions.set(node.identifier, node);
    if (node.type === 'footnoteDefinition') {
      const label = node.label ?? markdownPlainString(node.identifier);
      const key = labelKey(label);
      if (!footnoteLabels.has(key)) footnoteLabels.set(key, uniqueFootnote(label));
    }
    if ('children' in node) node.children.forEach(collect);
  }
  collect(root);
  const raw = (node: Nodes) => {
    const value = origins.get(node) ?? source;
    return value.slice(node.position?.start.offset, node.position?.end.offset);
  };
  // Nested syntax still resolves reference links defined in the outer document.
  const definitionSuffix = [...definitions.values()]
    .map((definition) => raw(definition))
    .join('\n');
  const fragmentRoot = (value: string) =>
    parse(`${value}${definitionSuffix ? `\n\n${definitionSuffix}` : ''}`);
  function reserve() {
    if (++created > MARKDOWN_LIMITS.nodes)
      throw new RangeError('Markdown contains too many nodes.');
  }
  const block = (
    type: DocumentNode['type'],
    content: DocumentNode[] = [],
    attrs: NodeAttributes = {},
  ): DocumentNode => {
    reserve();
    return { type, attrs: { ...attrs, id: uuid() }, content };
  };
  const text = (value: string, marks: DocumentMark[] = []): DocumentNode[] => {
    if (!value) return [];
    reserve();
    return [{ type: 'text', text: value, ...(marks.length ? { marks } : {}) }];
  };
  const atom = (
    type: DocumentNode['type'],
    attrs: NodeAttributes,
    marks: DocumentMark[] = [],
  ): DocumentNode => {
    reserve();
    return { type, attrs, ...(marks.length ? { marks } : {}) };
  };
  const fallback = (node: Nodes, reason: PreservedSyntax): DocumentNode => {
    preserved.add(reason);
    return block('paragraph', text(raw(node)));
  };
  function withMarks(current: DocumentMark[], incoming: DocumentMark[]): DocumentMark[] {
    const result = combineMarkdownMarks(current, incoming);
    if (!result)
      throw new Unsupported(incoming.some((mark) => mark.type === 'code') ? 'code' : 'html');
    return result;
  }
  function checkedSource(value: string, nonempty = false): string {
    if (value.includes('\0') || encoder.encode(value).length > MARKDOWN_LIMITS.sourceBytes)
      throw new Unsupported('bounds');
    if (nonempty && !value.trim()) throw new Unsupported('math');
    return value;
  }
  function checkedLabel(value: string): string {
    if (!value.trim() || characters(value) > 512 || hasControl(value))
      throw new Unsupported('footnote');
    return value;
  }
  function footnoteLabel(identifier: string, label?: string | null): string {
    // GFM has already decoded entities/escapes in `label`; decoding it twice
    // would turn a literal label such as "a&#32;b" into a different identity.
    const value = label ?? markdownPlainString(identifier);
    return checkedLabel(footnoteLabels.get(labelKey(value)) ?? value);
  }
  function wikiNode(value: string, embedded: boolean, marks: DocumentMark[] = []): DocumentNode {
    return createWiki(wikiParts(value, embedded), embedded, marks);
  }
  function createWiki(
    parts: ReturnType<typeof wikiParts>,
    embedded: boolean,
    marks: DocumentMark[],
  ): DocumentNode {
    if (
      !parts.target ||
      characters(parts.target) > 2048 ||
      hasControl(parts.target) ||
      (parts.label !== undefined && (characters(parts.label) > 512 || hasControl(parts.label))) ||
      [parts.width, parts.height].some(
        (size) => size !== undefined && (!Number.isSafeInteger(size) || size < 1 || size > 4096),
      )
    )
      throw new Unsupported('wiki');
    const attrs: NodeAttributes = {
      wiki_target: parts.target,
      occurrence_id: uuid(),
      ...(parts.label !== undefined ? { label: parts.label } : {}),
      ...(parts.width !== undefined ? { embed_width: parts.width } : {}),
      ...(parts.height !== undefined ? { embed_height: parts.height } : {}),
    };
    if (embedded && marks.length) throw new Unsupported('wiki');
    return embedded ? block('wikiEmbed', [], attrs) : atom('wikiLink', attrs, marks);
  }
  function inlineFragment(value: string, marks: DocumentMark[], depth: number) {
    const children = fragmentRoot(value).children.filter((node) => node.type !== 'definition');
    if (children.length !== 1 || children[0].type !== 'paragraph')
      throw new Unsupported('unsupported');
    return inlineSequence(children[0].children, marks, depth + 1);
  }
  function inlineSequence(
    nodes: PhrasingContent[],
    marks: DocumentMark[] = [],
    depth = 1,
  ): DocumentNode[] {
    if (depth > 28) throw new Unsupported('depth');
    let current = marks;
    const stack: { name: string; previous: DocumentMark[] }[] = [];
    const output: DocumentNode[] = [];
    for (const node of nodes) {
      if (node.type === 'html') {
        const token = htmlFormatToken(node.value);
        if (!token) throw new Unsupported('html');
        if (token.kind === 'break') output.push(atom('hardBreak', {}, current));
        else if (token.kind === 'open') {
          stack.push({ name: token.name, previous: current });
          current = withMarks(current, token.marks);
        } else {
          const opened = stack.pop();
          if (!opened || opened.name !== token.name) throw new Unsupported('html');
          current = opened.previous;
        }
        continue;
      }
      output.push(...inline(node, current, depth));
    }
    if (stack.length) throw new Unsupported('html');
    return output;
  }
  function inline(node: PhrasingContent, marks: DocumentMark[], depth: number): DocumentNode[] {
    switch (node.type) {
      case 'text':
        return text(node.value, marks);
      case 'break':
        return [atom('hardBreak', {}, marks)];
      case 'strong':
      case 'emphasis':
      case 'delete': {
        const type =
          node.type === 'strong' ? 'bold' : node.type === 'emphasis' ? 'italic' : 'strike';
        return inlineSequence(node.children, withMarks(marks, [{ type }]), depth + 1);
      }
      case 'inlineCode':
        return text(node.value, withMarks(marks, [{ type: 'code' }]));
      case 'inlineMath':
        return [atom('inlineMath', { source: checkedSource(node.value, true) }, marks)];
      case 'ofmHighlight': {
        const first = node.children[0];
        const emoji = first?.type === 'text' ? [...first.value][0] : undefined;
        const color = emoji && OFM_HIGHLIGHT_COLORS[emoji];
        const children =
          color && first?.type === 'text'
            ? [{ ...first, value: first.value.slice(emoji!.length) }, ...node.children.slice(1)]
            : node.children;
        return inlineSequence(
          children,
          withMarks(marks, [{ type: 'highlight', ...(color ? { attrs: { color } } : {}) }]),
          depth + 1,
        );
      }
      case 'ofmComment':
        return [atom('comment', { source: checkedSource(node.value.slice(2, -2)) }, marks)];
      case 'ofmWiki':
      case 'ofmEmbed':
        return [wikiNode(node.value, node.type === 'ofmEmbed', marks)];
      case 'ofmFootnote': {
        const label = uniqueFootnote(`inline-${++inlineNote}`);
        const content = inlineFragment(node.value.slice(2, -1), [], depth + 1);
        inlineFootnotes.push(block('footnoteDefinition', [block('paragraph', content)], { label }));
        return [atom('footnoteReference', { label }, marks)];
      }
      case 'footnoteReference':
        return [
          atom('footnoteReference', { label: footnoteLabel(node.identifier, node.label) }, marks),
        ];
      case 'ofmFootnoteReference': {
        const value = markdownPlainString(node.value.slice(2, -1));
        return [atom('footnoteReference', { label: footnoteLabel(value, value) }, marks)];
      }
      case 'link': {
        const known = capturedReferences.get(node.url);
        if (known) {
          if (node.children.some((part) => part.type !== 'text')) throw new Unsupported('link');
          const label = node.children.map((part) => (part as { value: string }).value).join('');
          return [atom('noteReference', { ...known.attrs, label, occurrence_id: uuid() }, marks)];
        }
        if (node.url && !/^[a-z][a-z0-9+.-]*:/i.test(node.url) && !/^[\\/]/.test(node.url)) {
          if (node.children.some((part) => part.type !== 'text')) throw new Unsupported('link');
          const label = node.children.map((part) => (part as { value: string }).value).join('');
          return [createWiki({ target: decodeFragment(node.url), label }, false, marks)];
        }
        if (!safeMarkdownLink(node.url)) throw new Unsupported('link');
        return inlineSequence(
          node.children,
          withMarks(marks, [
            {
              type: 'link',
              attrs: { href: node.url, ...(node.title ? { title: node.title } : {}) },
            },
          ]),
          depth + 1,
        );
      }
      case 'linkReference': {
        const definition = definitions.get(node.identifier);
        if (!definition || !safeMarkdownLink(definition.url)) throw new Unsupported('link');
        const content = inlineSequence(
          node.children,
          withMarks(marks, [
            {
              type: 'link',
              attrs: {
                href: definition.url,
                ...(definition.title ? { title: definition.title } : {}),
              },
            },
          ]),
          depth + 1,
        );
        usedDefinitions.add(node.identifier);
        return content;
      }
      case 'image': {
        const known = capturedImages.get(node.url.replace(/^pkms-blob:/, ''));
        if (!node.url.startsWith('pkms-blob:') || !known || marks.length)
          throw new Unsupported('image');
        return [
          block('image', [], {
            ...known.attrs,
            alt: node.alt ?? '',
            title: node.title ?? undefined,
          }),
        ];
      }
      case 'imageReference':
        throw new Unsupported('image');
      default:
        throw new Unsupported('unsupported');
    }
  }
  function anchor(value: string): string {
    if (!/^[A-Za-z0-9-]{1,128}$/.test(value) || occupiedAnchors.has(value))
      throw new Unsupported('anchor');
    occupiedAnchors.add(value);
    return value;
  }
  function paragraph(node: Extract<Nodes, { type: 'paragraph' | 'heading' }>, depth: number) {
    let children = node.children;
    let blockId: string | undefined;
    const suffix = raw(node).match(/[\t ]\^([A-Za-z0-9-]+)[\t ]*$/);
    const last = children.at(-1);
    if (suffix && last?.type === 'text' && last.value.endsWith(suffix[0])) {
      blockId = anchor(suffix[1]);
      children = [
        ...children.slice(0, -1),
        { ...last, value: last.value.slice(0, -suffix[0].length) },
      ];
    }
    const content = inlineSequence(children, [], depth + 1);
    const attributes = {
      ...(node.type === 'heading' ? { level: node.depth } : {}),
      ...(blockId ? { block_id: blockId } : {}),
    };
    if (
      node.type === 'paragraph' &&
      content.length === 1 &&
      content[0].type === 'comment' &&
      !content[0].marks?.length
    )
      return [
        block('commentBlock', [], {
          source: content[0].attrs?.source,
          ...(blockId ? { block_id: blockId } : {}),
        }),
      ];
    if (node.type === 'heading') {
      if (content.some((part) => part.type === 'wikiEmbed' || part.type === 'image'))
        throw new Unsupported('wiki');
      return [block('heading', content, attributes)];
    }
    const result: DocumentNode[] = [];
    let run: DocumentNode[] = [];
    for (const part of content) {
      if (part.type === 'wikiEmbed' || part.type === 'image') {
        if (run.length) result.push(block('paragraph', run));
        run = [];
        result.push(part);
      } else run.push(part);
    }
    if (run.length || !result.length) result.push(block('paragraph', run, attributes));
    else if (blockId) result.at(-1)!.attrs = { ...result.at(-1)!.attrs, block_id: blockId };
    return result;
  }
  function renderSequence(nodes: RootContent[], depth = 1): DocumentNode[] {
    const content: DocumentNode[] = [];
    // A mixed inline comment spanning a blank paragraph is not defined by the
    // source format. Preserve that complete range instead of formatting its body.
    const looseMarkers: { index: number; offset: number; source: string }[] = [];
    function inspect(node: Nodes, index: number) {
      if (node.type === 'text') {
        const value = raw(node);
        for (const match of value.matchAll(/%%/g)) {
          let slashes = 0;
          for (let cursor = match.index - 1; cursor >= 0 && value[cursor] === '\\'; --cursor)
            ++slashes;
          if (slashes % 2 === 0)
            looseMarkers.push({
              index,
              offset: (node.position?.start.offset ?? 0) + match.index,
              source: origins.get(node) ?? source,
            });
        }
      } else if ('children' in node) node.children.forEach((child) => inspect(child, index));
    }
    nodes.forEach(inspect);
    const rawComments = new Set<number>();
    for (let cursor = 0; cursor + 1 < looseMarkers.length; cursor += 2) {
      const start = looseMarkers[cursor],
        end = looseMarkers[cursor + 1];
      if (
        start.index !== end.index &&
        start.source === end.source &&
        /\r?\n[\t ]*\r?\n/.test(start.source.slice(start.offset, end.offset))
      ) {
        for (let index = start.index; index <= end.index; ++index) rawComments.add(index);
      }
    }
    for (const [index, node] of nodes.entries()) {
      if (rawComments.has(index)) {
        content.push(fallback(node, 'unsupported'));
        continue;
      }
      if (node.type === 'definition') continue;
      const blockId = node.type === 'paragraph' && raw(node).match(/^\^([A-Za-z0-9-]+)[\t ]*$/);
      if (blockId && content.length) {
        try {
          const previous = content.at(-1)!;
          if (previous.attrs?.block_id) throw new Unsupported('anchor');
          previous.attrs = { ...previous.attrs, block_id: anchor(blockId[1]) };
          continue;
        } catch (error) {
          if (!(error instanceof Unsupported)) throw error;
          content.push(fallback(node, error.reason));
          continue;
        }
      }
      content.push(...render(node, depth));
    }
    return content;
  }
  function render(node: RootContent, depth: number): DocumentNode[] {
    if (depth > 27) return [fallback(node, 'depth')];
    const usedBefore = new Set(usedDefinitions);
    const anchorsBefore = new Set(occupiedAnchors);
    const inlineNotesBefore = inlineFootnotes.length;
    try {
      switch (node.type) {
        case 'paragraph':
        case 'heading':
          return paragraph(node, depth);
        case 'thematicBreak':
          return [block('horizontalRule')];
        case 'yaml':
          if (node !== root.children[0] || options.allowFrontmatter === false)
            return [fallback(node, 'frontmatter')];
          return [block('frontmatter', [], { source: checkedSource(node.value) })];
        case 'math':
          return [block('blockMath', [], { source: checkedSource(node.value, true) })];
        case 'ofmCommentBlock': {
          return [block('commentBlock', [], { source: checkedSource(node.value) })];
        }
        case 'html': {
          const parts = htmlText(node.value);
          if (!parts) return [fallback(node, 'html')];
          return [
            block(
              'paragraph',
              parts.flatMap((part) =>
                'hardBreak' in part ? [atom('hardBreak', {})] : text(part.text, part.marks),
              ),
            ),
          ];
        }
        case 'blockquote': {
          if (!node.children.length) return [fallback(node, 'unsupported')];
          const first = node.children[0];
          const header =
            first.type === 'paragraph' &&
            raw(first)
              .split(/\r?\n/, 1)[0]
              .match(/^\[!([A-Za-z0-9_-]{1,64})\]([+-])?(?:[\t ]+(.*))?$/);
          if (header) {
            const title = header[3] === undefined ? undefined : markdownPlainString(header[3]);
            if (title !== undefined && (characters(title) > 2048 || hasControl(title)))
              throw new Unsupported('bounds');
            const body = raw(node)
              .split(/\r?\n/)
              .map((line) => line.replace(/^[\t ]*>[\t ]?/, ''))
              .slice(1)
              .join('\n');
            const children = renderSequence(fragmentRoot(body).children, depth + 1);
            return [
              block('callout', children, {
                callout_type: header[1],
                ...(title !== undefined ? { title } : {}),
                ...(header[2] ? { collapsed: header[2] === '-' } : {}),
              }),
            ];
          }
          return [block('blockquote', renderSequence(node.children, depth + 1))];
        }
        case 'code':
          if (node.meta) return [fallback(node, 'code')];
          return [block('codeBlock', text(node.value), node.lang ? { language: node.lang } : {})];
        case 'list': {
          if (node.ordered && ((node.start ?? 1) < 1 || (node.start ?? 1) > 4294967295))
            return [fallback(node, 'unsupported')];
          const lists: DocumentNode[] = [];
          for (const item of node.children) {
            const first = item.children[0];
            const firstText = first?.type === 'paragraph' ? first.children[0] : undefined;
            const custom =
              item.checked == null && firstText?.type === 'text'
                ? raw(first!).match(/^\[([^\[\]\r\n])\](?:[\t ]+|$)/u)
                : null;
            let itemChildren = item.children;
            if (custom && first?.type === 'paragraph' && firstText?.type === 'text') {
              const changed = { ...firstText, value: firstText.value.slice(custom[0].length) };
              origins.set(changed, origins.get(firstText) ?? source);
              const changedFirst = { ...first, children: [changed, ...first.children.slice(1)] };
              origins.set(changedFirst, origins.get(first) ?? source);
              itemChildren = [changedFirst, ...item.children.slice(1)];
            }
            const task = item.checked != null || !!custom;
            if (custom && hasControl(custom[1])) throw new Unsupported('bounds');
            if (node.ordered && task) throw new Unsupported('unsupported');
            const listType = node.ordered ? 'orderedList' : task ? 'taskList' : 'bulletList';
            let list = lists.at(-1);
            if (list?.type !== listType) {
              list = block(listType, [], node.ordered ? { start: node.start ?? 1 } : {});
              lists.push(list);
            }
            const children = renderSequence(itemChildren, depth + 2);
            if (children[0]?.type !== 'paragraph') children.unshift(block('paragraph'));
            list.content!.push(
              block(
                task ? 'taskItem' : 'listItem',
                children,
                task
                  ? {
                      checked: item.checked ?? custom?.[1] !== ' ',
                      ...(custom ? { task_marker: custom[1] } : {}),
                    }
                  : {},
              ),
            );
          }
          return lists;
        }
        case 'table': {
          const width = node.children[0]?.children.length ?? 0;
          if (!width || node.children.some((row) => row.children.length !== width))
            return [fallback(node, 'unsupported')];
          return [
            block(
              'table',
              node.children.map((row, index) =>
                block(
                  'tableRow',
                  row.children.map((cell, column) => {
                    const content = inlineSequence(cell.children, [], depth + 4);
                    if (content.some((part) => part.type === 'wikiEmbed' || part.type === 'image'))
                      throw new Unsupported('wiki');
                    const align = node.align?.[column];
                    return block(
                      index === 0 ? 'tableHeader' : 'tableCell',
                      [block('paragraph', content)],
                      align ? { align } : {},
                    );
                  }),
                ),
              ),
            ),
          ];
        }
        case 'footnoteDefinition': {
          const key = labelKey(node.label ?? markdownPlainString(node.identifier));
          if (emittedFootnotes.has(key)) return [fallback(node, 'footnote')];
          emittedFootnotes.add(key);
          const children = renderSequence(node.children, depth + 1);
          return [
            block('footnoteDefinition', children, {
              label: footnoteLabel(node.identifier, node.label),
            }),
          ];
        }
        default:
          return [fallback(node, 'unsupported')];
      }
    } catch (error) {
      if (!(error instanceof Unsupported)) throw error;
      usedDefinitions.clear();
      usedBefore.forEach((id) => usedDefinitions.add(id));
      occupiedAnchors.clear();
      anchorsBefore.forEach((id) => occupiedAnchors.add(id));
      inlineFootnotes.length = inlineNotesBefore;
      return [fallback(node, error.reason)];
    }
  }

  const content = renderSequence(root.children);
  content.push(...inlineFootnotes);
  for (const node of root.children)
    if (
      node.type === 'definition' &&
      (!usedDefinitions.has(node.identifier) || definitions.get(node.identifier) !== node)
    )
      content.push(fallback(node, 'unsupported'));
  return {
    document: { type: 'doc', content: content.length ? content : [block('paragraph')] },
    preserved: [...preserved],
  };
}

function decodeFragment(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

export function referenceUrl(attrs: NodeAttributes | undefined): string {
  return `pkms://object/${attrs?.target_id ?? ''}${attrs?.target_revision_id ? `?revision=${attrs.target_revision_id}` : ''}${attrs?.target_block_id ? `#${attrs.target_block_id}` : ''}`;
}

/** Useful to paste a fragment into a note without rewriting its existing blocks. */
export function appendMarkdownDocument(
  base: NoteDocument,
  source: string,
  options: Omit<MarkdownOptions, 'contextDocument'> = {},
): MarkdownDocument {
  const converted = markdownDocument(source, {
    ...options,
    allowFrontmatter: !base.content.length && (options.allowFrontmatter ?? false),
    contextDocument: base,
  });
  return {
    document: {
      type: 'doc',
      content: [...structuredClone(base.content), ...converted.document.content],
    },
    preserved: converted.preserved,
  };
}

/** A conservative paste affordance, not a claim that every matched syntax is supported. */
export function looksLikeMarkdown(source: string): boolean {
  return (
    /^ {0,3}(?:#{1,6}\s|>\s?|(?:[-+*]|\d+[.)])\s|`{3}|~{3}|---\s*$|\$\$|\[\^[^\]]+\]:)/m.test(
      source,
    ) ||
    /(?:^|[^\\])(?:\*\*[^*]+\*\*|\*[^*\n]+\*|_[^_\n]+_|~[^~\n]+~|~~[^~]+~~|==[^=]+==|!?\[\[[^\]\n]+\]\]|\^\[[^\]\n]+\]|%%[\s\S]+%%|`[^`\n]+`|\$[^$\n]+\$|!?\[[^\]\n]+\]\([^\n]+\)|<(?:u|sup|sub|span|mark|strong|em)\b)/i.test(
      source,
    ) ||
    /^[\t ]*\|.*\|[\t ]*\r?\n[\t |:-]+$/m.test(source)
  );
}

export function plainTextDocument(source: string): NoteDocument {
  return {
    type: 'doc',
    content: source.split('\n').map((line) => ({
      type: 'paragraph',
      attrs: { id: crypto.randomUUID() },
      content: line ? [{ type: 'text', text: line }] : [],
    })),
  };
}
