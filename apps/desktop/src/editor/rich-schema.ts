import { Extension, InputRule, Mark, Node, mergeAttributes } from '@tiptap/core';
import { Plugin, type Transaction } from '@tiptap/pm/state';
import { ReplaceStep, findWrapping } from '@tiptap/pm/transform';
import { TableCell, TableHeader } from '@tiptap/extension-table';
import { OrderedList } from '@tiptap/extension-list';
import TaskItem from '@tiptap/extension-task-item';
import type { Mark as ProseMirrorMark, Node as ProseMirrorNode, NodeType } from '@tiptap/pm/model';
import type { ExtendedRegExpMatchArray } from '@tiptap/core';
import {
  RICH_BLOCK_TYPES,
  blockAnchorError,
  calloutTypeError,
  editorColor,
  footnoteLabelError,
  richDocumentCheckForReplacement,
  richDocumentFacts,
  sourceError,
  validTaskMarker,
  wikiTargetError,
  type RichDocumentFacts,
} from './rich-format-model.ts';
import { MIRROR } from './mirror-transactions.ts';
import { DEFAULT_HIGHLIGHT, HIGHLIGHT_COLORS, highlightInk } from '../rich/colors.ts';

const SOURCE = {
  default: '',
  parseHTML: (element: HTMLElement) =>
    element.getAttribute('data-source') ?? element.textContent ?? '',
  renderHTML: (attrs: Record<string, unknown>) => ({ 'data-source': attrs.source }),
};
const highlightColors: Record<string, string> = {
  '🟡': HIGHLIGHT_COLORS[0].color,
  '🔴': HIGHLIGHT_COLORS[1].color,
  '🟠': HIGHLIGHT_COLORS[2].color,
  '🟢': HIGHLIGHT_COLORS[3].color,
  '🔵': HIGHLIGHT_COLORS[4].color,
  '🟣': HIGHLIGHT_COLORS[5].color,
};

/**
 * Bibliography metadata is stored on the generated `References` heading. It is
 * intentionally a global attribute so every heading round-trips through Tiptap
 * without replacing the heading node supplied by StarterKit.
 */
export const BibliographyHeadingMetadata = Extension.create({
  name: 'bibliographyHeadingMetadata',
  addGlobalAttributes() {
    return [
      {
        types: ['heading'],
        attributes: {
          bibliography_style_id: {
            default: null,
            parseHTML: (element: HTMLElement) => element.getAttribute('data-bibliography-style'),
            renderHTML: (attrs: Record<string, unknown>) =>
              typeof attrs.bibliography_style_id === 'string'
                ? { 'data-bibliography-style': attrs.bibliography_style_id }
                : {},
          },
          bibliography_fingerprint: {
            default: null,
            parseHTML: (element: HTMLElement) =>
              element.getAttribute('data-bibliography-fingerprint'),
            renderHTML: (attrs: Record<string, unknown>) =>
              typeof attrs.bibliography_fingerprint === 'string'
                ? { 'data-bibliography-fingerprint': attrs.bibliography_fingerprint }
                : {},
          },
        },
      },
    ];
  },
});

function inCode(marks: readonly ProseMirrorMark[], code: boolean | undefined) {
  return !!code || marks.some((mark) => mark.type.name === 'code');
}

export const Highlight = Mark.create({
  name: 'highlight',
  priority: 110,
  addAttributes() {
    return {
      color: {
        default: null,
        parseHTML: (element: HTMLElement) =>
          editorColor(element.getAttribute('data-color')) ||
          editorColor(element.style.backgroundColor),
        renderHTML: (attrs) => {
          const color = editorColor(attrs.color) || DEFAULT_HIGHLIGHT;
          return {
            'data-color': color,
            style: 'background-color: ' + color + ';color: ' + highlightInk(color),
          };
        },
      },
    };
  },
  parseHTML: () => [{ tag: 'mark' }],
  renderHTML: ({ HTMLAttributes }) => ['mark', HTMLAttributes, 0],
  addKeyboardShortcuts() {
    return { 'Mod-Shift-h': () => this.editor.commands.toggleMark(this.name) };
  },
  addInputRules() {
    return [
      new InputRule({
        find: /(?<![\\=])==([^=\n]+)==$/u,
        handler: ({ state, range, match }) => {
          const marks = state.storedMarks ?? state.selection.$from.marks();
          if (inCode(marks, state.selection.$from.parent.type.spec.code)) return null;
          const [first] = [...match[1]];
          const color = highlightColors[first] ?? DEFAULT_HIGHLIGHT;
          const text = highlightColors[first] ? match[1].slice(first.length) : match[1];
          if (!text) return null;
          state.tr.replaceWith(
            range.from,
            range.to,
            state.schema.text(text, this.type.create({ color }).addToSet(marks)),
          );
        },
      }),
    ];
  },
});

export const TextStyle = Mark.create({
  name: 'textStyle',
  addAttributes: () => ({
    color: {
      default: null,
      parseHTML: (element: HTMLElement) => editorColor(element.style.color),
      renderHTML: (attrs) => ({ style: 'color: ' + (editorColor(attrs.color) || 'inherit') }),
    },
  }),
  parseHTML: () => [
    {
      tag: 'span[style]',
      getAttrs: (element: HTMLElement) =>
        editorColor(element.style.color) ? { color: editorColor(element.style.color) } : false,
    },
  ],
  renderHTML: ({ HTMLAttributes }) => ['span', HTMLAttributes, 0],
});
export const Superscript = Mark.create({
  name: 'superscript',
  excludes: 'subscript',
  parseHTML: () => [{ tag: 'sup:not([data-footnote-reference])' }],
  renderHTML: ({ HTMLAttributes }) => ['sup', HTMLAttributes, 0],
});
export const Subscript = Mark.create({
  name: 'subscript',
  excludes: 'superscript',
  parseHTML: () => [{ tag: 'sub' }],
  renderHTML: ({ HTMLAttributes }) => ['sub', HTMLAttributes, 0],
});

export const RichBlockAttributes = Extension.create({
  name: 'richBlockAttributes',
  addOptions: () => ({ onError: (_error: string) => {} }),
  addGlobalAttributes() {
    return [
      {
        types: RICH_BLOCK_TYPES,
        attributes: {
          block_id: {
            default: null,
            keepOnSplit: false,
            parseHTML: (element: HTMLElement) => {
              const value = element.getAttribute('data-block-id');
              return value && /^[A-Za-z0-9-]{1,128}$/.test(value) ? value : null;
            },
            renderHTML: (attrs) => (attrs.block_id ? { 'data-block-id': attrs.block_id } : {}),
          },
        },
      },
      {
        types: ['taskItem'],
        attributes: {
          task_marker: {
            default: null,
            keepOnSplit: false,
            parseHTML: (element: HTMLElement) => {
              const marker = element.getAttribute('data-task-marker');
              return marker && validTaskMarker(marker) ? marker : null;
            },
            renderHTML: (attrs) =>
              attrs.task_marker ? { 'data-task-marker': attrs.task_marker } : {},
          },
        },
      },
    ];
  },
  addInputRules() {
    return [
      new InputRule({
        find: /\s\^([A-Za-z0-9-]{1,128}) $/,
        handler: ({ state, range, match }) => {
          const { $from } = state.selection;
          if (!$from.parent.isTextblock || $from.parent.type.spec.code) return null;
          if (blockAnchorError(match[1], state.doc, $from.parent.attrs.id)) return null;
          const at = $from.before();
          state.tr.delete(range.from, range.to).setNodeMarkup(at, undefined, {
            ...$from.parent.attrs,
            block_id: match[1],
          });
        },
      }),
    ];
  },
  addProseMirrorPlugins() {
    return [
      richDocumentGuard(this.options.onError),
      new Plugin({
        appendTransaction(transactions, previous, current) {
          if (
            !transactions.some((tr) => tr.docChanged) ||
            transactions.some((tr) => tr.getMeta(MIRROR) || tr.getMeta('pkmsExplicitTaskMarker'))
          )
            return null;
          const oldTasks = new Map<string, { checked: boolean; marker: string }>();
          previous.doc.descendants((node) => {
            if (node.type.name === 'taskItem' && node.attrs.task_marker)
              oldTasks.set(node.attrs.id, {
                checked: node.attrs.checked,
                marker: node.attrs.task_marker,
              });
          });
          if (!oldTasks.size) return null;
          const transaction = current.tr;
          current.doc.descendants((node, at) => {
            if (node.type.name !== 'taskItem') return;
            const old = oldTasks.get(node.attrs.id);
            if (old && old.checked !== node.attrs.checked && old.marker === node.attrs.task_marker)
              transaction.setNodeMarkup(at, undefined, { ...node.attrs, task_marker: null });
          });
          return transaction.docChanged ? transaction : null;
        },
      }),
    ];
  },
});

/**
 * Reject a transaction that would leave a document the schema cannot express.
 *
 * A plain replacement is settled from the range it replaced: the multisets kept
 * from the last accepted document say what still exists, and re-reading one
 * range says what arrived. Everything else (marks, moves, several steps at once)
 * re-reads the whole document, so the cheap path can only ever remove work that
 * was already unnecessary — never a check. On a 4 MiB note this takes the guard
 * from ~1.6 ms per keystroke to below the measurement noise.
 */
export function richDocumentGuard(report: (error: string) => void) {
  let checked: ProseMirrorNode | null = null;
  let facts: RichDocumentFacts | null = null;
  return new Plugin({
    filterTransaction(transaction) {
      if (!transaction.docChanged) return true;
      if (transaction.getMeta(MIRROR)) {
        // Mirrored transactions are written by this app and were never checked
        // here, so the next change starts from a full pass again.
        checked = null;
        facts = null;
        return true;
      }
      const replacement = replacementOf(transaction);
      const incremental =
        replacement && facts && checked === transaction.before
          ? richDocumentCheckForReplacement(
              transaction.before,
              transaction.doc,
              facts,
              replacement.removed,
              replacement.inserted,
            )
          : null;
      const check = incremental ?? richDocumentFacts(transaction.doc);
      if (check.error) {
        report(check.error);
        return false;
      }
      checked = transaction.doc;
      facts = check.facts;
      return true;
    },
  });
}

/** The single range a plain replacement touched, in both documents' positions. */
function replacementOf(transaction: Transaction) {
  if (transaction.steps.length !== 1) return null;
  const step = transaction.steps[0];
  if (!(step instanceof ReplaceStep)) return null;
  return {
    removed: [step.from, step.to] as const,
    inserted: [step.from, step.from + step.slice.size] as const,
  };
}

function atomRule(
  type: NodeType,
  find: RegExp,
  attrs: (match: ExtendedRegExpMatchArray) => Record<string, unknown> | null,
) {
  return new InputRule({
    find,
    handler: ({ state, range, match }) => {
      const { $from } = state.selection;
      const marks = state.storedMarks ?? $from.marks();
      if (inCode(marks, $from.parent.type.spec.code)) return null;
      const attributes = attrs(match);
      if (!attributes) return null;
      if (type.isInline) {
        state.tr.replaceWith(range.from, range.to, type.create(attributes, null, marks));
      } else {
        if (
          $from.parent.type.name !== 'paragraph' ||
          range.from !== $from.start() ||
          $from.parentOffset !== $from.parent.content.size
        )
          return null;
        state.tr.replaceWith(
          $from.before(),
          $from.after(),
          type.create({ ...$from.parent.attrs, ...attributes }),
        );
      }
    },
  });
}

export const InlineMath = Node.create({
  name: 'inlineMath',
  group: 'inline',
  inline: true,
  atom: true,
  addAttributes: () => ({ source: SOURCE }),
  parseHTML: () => [{ tag: 'span[data-inline-math]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'span',
    mergeAttributes(HTMLAttributes, { 'data-inline-math': '' }),
    '$' + node.attrs.source + '$',
  ],
  addInputRules() {
    return [
      atomRule(this.type, /(?<![\\$])\$([^$\n]+)\$$/, (match) =>
        sourceError(match[1], true) ? null : { source: match[1] },
      ),
    ];
  },
});
export const BlockMath = Node.create({
  name: 'blockMath',
  group: 'block',
  atom: true,
  defining: true,
  addAttributes: () => ({ source: SOURCE }),
  parseHTML: () => [{ tag: 'div[data-block-math]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'div',
    mergeAttributes(HTMLAttributes, { 'data-block-math': '' }),
    '$$' + node.attrs.source + '$$',
  ],
  addInputRules() {
    return [
      atomRule(this.type, /^\$\$(.+)\$\$$/, (match) =>
        sourceError(match[1], true) ? null : { source: match[1] },
      ),
    ];
  },
});

function wikiAttributes() {
  return {
    wiki_target: {
      default: '',
      parseHTML: (element: HTMLElement) => element.getAttribute('data-wiki-target') ?? '',
      renderHTML: (attrs: Record<string, unknown>) => ({ 'data-wiki-target': attrs.wiki_target }),
    },
    label: { default: null, parseHTML: (element: HTMLElement) => element.getAttribute('label') },
    occurrence_id: { default: null },
    target_id: { default: null },
    target_block_id: { default: null },
    target_revision_id: { default: null },
  };
}
function parsedWiki(value: string, embedded = false) {
  const separator = value.lastIndexOf('|');
  const target = separator < 0 ? value : value.slice(0, separator);
  let label: string | null = separator < 0 ? null : value.slice(separator + 1);
  if (wikiTargetError(target) || (label && [...label].length > 512)) return null;
  let width: number | null = null;
  let height: number | null = null;
  if (embedded && label) {
    const dimensions = /^(\d+)(?:x(\d+))?$/.exec(label);
    if (dimensions) {
      width = Number(dimensions[1]);
      height = dimensions[2] ? Number(dimensions[2]) : null;
      if (width < 1 || width > 4096 || (height !== null && (height < 1 || height > 4096)))
        return null;
      label = null;
    }
  }
  return {
    wiki_target: target,
    label,
    occurrence_id: crypto.randomUUID(),
    ...(embedded ? { embed_width: width, embed_height: height } : {}),
  };
}
export const WikiLink = Node.create({
  name: 'wikiLink',
  group: 'inline',
  inline: true,
  atom: true,
  addAttributes: wikiAttributes,
  parseHTML: () => [{ tag: 'span[data-wiki-link]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'span',
    mergeAttributes(HTMLAttributes, { 'data-wiki-link': '' }),
    '[[' + node.attrs.wiki_target + (node.attrs.label == null ? '' : '|' + node.attrs.label) + ']]',
  ],
  addInputRules() {
    return [atomRule(this.type, /(?<![\\!])\[\[([^\]\n]+)\]\]$/, (match) => parsedWiki(match[1]))];
  },
});
export const WikiEmbed = Node.create({
  name: 'wikiEmbed',
  group: 'block',
  atom: true,
  defining: true,
  addAttributes: () => ({
    ...wikiAttributes(),
    embed_width: { default: null },
    embed_height: { default: null },
  }),
  parseHTML: () => [{ tag: 'div[data-wiki-embed]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'div',
    mergeAttributes(HTMLAttributes, { 'data-wiki-embed': '' }),
    '![[' + node.attrs.wiki_target + ']]',
  ],
  addInputRules() {
    return [atomRule(this.type, /^!\[\[([^\]\n]+)\]\]$/, (match) => parsedWiki(match[1], true))];
  },
});

export const Callout = Node.create({
  name: 'callout',
  group: 'block',
  content: 'block*',
  defining: true,
  addAttributes: () => ({
    callout_type: {
      default: 'note',
      parseHTML: (element: HTMLElement) => element.getAttribute('data-callout') ?? 'note',
      renderHTML: (attrs) => ({ 'data-callout': attrs.callout_type }),
    },
    title: { default: null, parseHTML: (element: HTMLElement) => element.getAttribute('title') },
    collapsed: {
      default: null,
      parseHTML: (element: HTMLElement) => {
        const state = element.getAttribute('data-collapsed');
        return state === 'true' ? true : state === 'false' ? false : null;
      },
      renderHTML: (attrs) =>
        attrs.collapsed == null ? {} : { 'data-collapsed': String(attrs.collapsed) },
    },
  }),
  parseHTML: () => [{ tag: 'aside[data-callout]' }],
  renderHTML: ({ HTMLAttributes }) => ['aside', HTMLAttributes, 0],
  addInputRules() {
    return [
      new InputRule({
        // Enter commits the header; a space must remain available for its title.
        find: /^\[!([A-Za-z0-9_-]{1,64})\]([+-])?(?: +(.*))?\n$/,
        handler: ({ state, range, match }) => {
          const { $from } = state.selection;
          const depth = $from.depth - 1;
          if (
            depth < 1 ||
            $from.node(depth).type.name !== 'blockquote' ||
            $from.index(depth) !== 0 ||
            calloutTypeError(match[1])
          )
            return null;
          const quote = $from.node(depth);
          state.tr.delete(range.from, range.to).setNodeMarkup($from.before(depth), this.type, {
            ...quote.attrs,
            callout_type: match[1],
            title: match[3] ?? null,
            collapsed: match[2] ? match[2] === '-' : null,
          });
        },
      }),
    ];
  },
});

export const FootnoteReference = Node.create({
  name: 'footnoteReference',
  group: 'inline',
  inline: true,
  atom: true,
  addAttributes: () => ({
    label: { default: '', parseHTML: (element: HTMLElement) => element.getAttribute('label') },
  }),
  parseHTML: () => [{ tag: 'sup[data-footnote-reference]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'sup',
    mergeAttributes(HTMLAttributes, { 'data-footnote-reference': '' }),
    '[^' + node.attrs.label + ']',
  ],
  addInputRules() {
    return [
      new InputRule({
        // Wait for a space so the user can still type the definition's ":".
        find: /(?<!\\)\[\^([^\]\n]+)\] $/,
        handler: ({ state, range, match }) => {
          const marks = state.storedMarks ?? state.selection.$from.marks();
          if (
            footnoteLabelError(match[1]) ||
            inCode(marks, state.selection.$from.parent.type.spec.code)
          )
            return null;
          state.tr.replaceWith(range.from, range.to, [
            this.type.create({ label: match[1] }, null, marks),
            state.schema.text(' ', marks),
          ]);
        },
      }),
    ];
  },
});
export const FootnoteDefinition = Node.create({
  name: 'footnoteDefinition',
  group: 'block',
  content: 'block*',
  defining: true,
  addAttributes: () => ({
    label: { default: '', parseHTML: (element: HTMLElement) => element.getAttribute('label') },
  }),
  parseHTML: () => [{ tag: 'section[data-footnote-definition]' }],
  renderHTML: ({ HTMLAttributes }) => [
    'section',
    mergeAttributes(HTMLAttributes, { 'data-footnote-definition': '' }),
    0,
  ],
  addInputRules() {
    return [
      new InputRule({
        find: /^\[\^([^\]\n]+)\]: $/,
        handler: ({ state, range, match }) => {
          if (footnoteLabelError(match[1]) || state.selection.$from.parent.type.spec.code)
            return null;
          const transaction = state.tr.delete(range.from, range.to);
          const blockRange = transaction.doc.resolve(range.from).blockRange();
          const wrapping =
            blockRange &&
            findWrapping(blockRange, this.type, {
              label: match[1],
              id: crypto.randomUUID(),
            });
          if (!blockRange || !wrapping) return null;
          transaction.wrap(blockRange, wrapping);
        },
      }),
    ];
  },
});

export const Comment = Node.create({
  name: 'comment',
  group: 'inline',
  inline: true,
  atom: true,
  addAttributes: () => ({ source: SOURCE }),
  parseHTML: () => [{ tag: 'span[data-note-comment]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'span',
    mergeAttributes(HTMLAttributes, { 'data-note-comment': '' }),
    '%%' + node.attrs.source + '%%',
  ],
  addInputRules() {
    return [
      atomRule(this.type, /(?<![\\%])%%([^%\n]+)%%$/, (match) =>
        sourceError(match[1]) ? null : { source: match[1] },
      ),
    ];
  },
});
export const CommentBlock = Node.create({
  name: 'commentBlock',
  group: 'block',
  atom: true,
  defining: true,
  addAttributes: () => ({ source: SOURCE }),
  parseHTML: () => [{ tag: 'div[data-note-comment]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'div',
    mergeAttributes(HTMLAttributes, { 'data-note-comment': '' }),
    '%%\n' + node.attrs.source + '\n%%',
  ],
});
export const Frontmatter = Node.create({
  name: 'frontmatter',
  group: 'block',
  atom: true,
  defining: true,
  addAttributes: () => ({ source: SOURCE }),
  parseHTML: () => [{ tag: 'div[data-frontmatter]' }],
  renderHTML: ({ node, HTMLAttributes }) => [
    'div',
    mergeAttributes(HTMLAttributes, { 'data-frontmatter': '' }),
    '---\n' + node.attrs.source + '\n---',
  ],
});

export const richSchemaExtensions = [
  Highlight,
  TextStyle,
  Superscript,
  Subscript,
  RichBlockAttributes,
  InlineMath,
  BlockMath,
  WikiLink,
  WikiEmbed,
  Callout,
  FootnoteReference,
  FootnoteDefinition,
  Comment,
  CommentBlock,
  Frontmatter,
];

const alignAttribute = {
  default: null,
  parseHTML: (element: HTMLElement) => {
    const value = element.style.textAlign || element.getAttribute('align');
    return value && ['left', 'center', 'right'].includes(value) ? value : null;
  },
  renderHTML: (attrs: Record<string, unknown>) =>
    ['left', 'center', 'right'].includes(String(attrs.align))
      ? { style: 'text-align: ' + attrs.align }
      : {},
};
export const AlignedTableCell = TableCell.extend({
  addAttributes() {
    return { ...this.parent?.(), align: alignAttribute };
  },
});
export const AlignedTableHeader = TableHeader.extend({
  addAttributes() {
    return { ...this.parent?.(), align: alignAttribute };
  },
});

export const RichOrderedList = OrderedList.extend({
  addAttributes() {
    const attributes = { ...this.parent?.() };
    // The document contract supports the start number, not HTML marker styles.
    // Even Tiptap's default `type: null` is an unknown field to core's decoder.
    delete attributes.type;
    return attributes;
  },
});

export const RichTaskItem = TaskItem.extend({
  addInputRules() {
    return [
      new InputRule({
        find: /^\[([^\]\n]+)\] $/u,
        handler: ({ state, range, match }) => {
          const marker = match[1];
          if (!validTaskMarker(marker) || state.selection.$from.parent.type.spec.code) return null;
          const transaction = state.tr.delete(range.from, range.to);
          const blockRange = transaction.doc.resolve(range.from).blockRange();
          const wrapping =
            blockRange &&
            findWrapping(blockRange, this.type, {
              id: crypto.randomUUID(),
              checked: marker !== ' ',
              task_marker: /^[ xX]$/.test(marker) ? null : marker,
            });
          if (!blockRange || !wrapping) return null;
          transaction.wrap(blockRange, wrapping).setMeta('pkmsExplicitTaskMarker', true);
        },
      }),
      ...(this.parent?.() ?? []),
    ];
  },
});
