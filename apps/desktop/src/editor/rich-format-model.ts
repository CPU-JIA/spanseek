import type { Node as ProseMirrorNode } from '@tiptap/pm/model';
import { footnoteKey } from '../rich/footnotes.ts';
import {
  NodeSelection,
  TextSelection,
  type EditorState,
  type SelectionBookmark,
  type Transaction,
} from '@tiptap/pm/state';

export const RICH_SOURCE_BYTES = 64 * 1024;
export const RICH_BLOCK_TYPES = [
  'paragraph',
  'heading',
  'bulletList',
  'orderedList',
  'listItem',
  'taskList',
  'taskItem',
  'blockquote',
  'codeBlock',
  'horizontalRule',
  'image',
  'table',
  'tableRow',
  'tableCell',
  'tableHeader',
  'callout',
  'blockMath',
  'wikiEmbed',
  'footnoteDefinition',
  'commentBlock',
  'frontmatter',
];

export type RichEditKind =
  | 'inlineMath'
  | 'blockMath'
  | 'mermaid'
  | 'comment'
  | 'commentBlock'
  | 'wikiLink'
  | 'wikiEmbed'
  | 'footnote'
  | 'footnoteReference'
  | 'footnoteDefinition'
  | 'callout'
  | 'frontmatter'
  | 'blockId'
  | 'taskMarker'
  | 'markdown'
  | 'highlightColor'
  | 'textColor';

export interface RichEditRequest {
  kind: RichEditKind;
  position?: number;
  source?: string;
}

export interface RichEditTarget {
  node: ProseMirrorNode | null;
  position?: number;
  bookmark: SelectionBookmark;
  deleted: boolean;
}

export function captureRichEditTarget(state: EditorState, position?: number): RichEditTarget {
  return {
    node:
      position !== undefined && position >= 0 && position < state.doc.content.size
        ? state.doc.nodeAt(position)
        : null,
    position,
    bookmark: state.selection.getBookmark(),
    deleted: false,
  };
}

export function mapRichEditTarget(
  target: RichEditTarget,
  transaction: Transaction,
): RichEditTarget {
  const mapped =
    target.position === undefined ? null : transaction.mapping.mapResult(target.position, 1);
  return {
    ...target,
    bookmark: target.bookmark.map(transaction.mapping),
    position: mapped?.pos,
    // Deleting an atom at a range boundary must not retarget its identical neighbour.
    deleted: target.deleted || (mapped?.deleted ?? false),
  };
}

export function richEditTargetPosition(
  document: ProseMirrorNode,
  target: RichEditTarget,
): number | undefined {
  if (target.position === undefined) return undefined;
  const at = target.node?.attrs.id
    ? findNodePosition(document, target.node.attrs.id)
    : target.deleted
      ? undefined
      : target.position;
  const current =
    at !== undefined && at >= 0 && at < document.content.size ? document.nodeAt(at) : null;
  if (
    !target.node ||
    !current ||
    current.type !== target.node.type ||
    JSON.stringify(current.attrs) !== JSON.stringify(target.node.attrs)
  )
    throw new Error('此内容已在编辑期间改变，请关闭后重新打开；输入仍保留在这里。');
  return at;
}

export function editorColor(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const input = value.trim();
  if (/^#[\da-f]{6}$/i.test(input)) return input.toUpperCase();
  if (/^#[\da-f]{3}$/i.test(input))
    return (
      '#' +
      [...input.slice(1)]
        .map((part) => part + part)
        .join('')
        .toUpperCase()
    );
  const rgb = /^rgb\(\s*(\d{1,3})\s*[, ]\s*(\d{1,3})\s*[, ]\s*(\d{1,3})\s*\)$/i.exec(input);
  if (!rgb) return null;
  const channels = rgb.slice(1).map(Number);
  if (channels.some((channel) => channel > 255)) return null;
  return (
    '#' +
    channels
      .map((channel) => channel.toString(16).padStart(2, '0'))
      .join('')
      .toUpperCase()
  );
}

export function sourceError(source: string, nonempty = false): string | null {
  if (source.includes('\u0000')) return '源码不能包含空字符（U+0000），请移除后再保存。';
  if (nonempty && !source.trim()) return '请输入源码。';
  if (new TextEncoder().encode(source).length > RICH_SOURCE_BYTES)
    return '源码超过 64 KiB，请缩短后再保存。';
  return null;
}

export function wikiTargetError(target: string): string | null {
  if (!target.trim() || [...target].length > 2048 || /[\u0000-\u001f\u007f-\u009f]/.test(target))
    return '内部链接目标须为 1–2048 个字符，且不能包含控制字符。';
  return null;
}

export function footnoteLabelError(label: string): string | null {
  if (!label.trim() || [...label].length > 512 || /[\u0000-\u001f\u007f-\u009f]/.test(label))
    return '脚注标识须为 1–512 个字符，且不能包含控制字符。';
  return null;
}

export function calloutTypeError(type: string): string | null {
  return /^[A-Za-z0-9_-]{1,64}$/.test(type)
    ? null
    : '提示块类型须为 1–64 个字母、数字、下划线或连字符。';
}

export function calloutTitleError(title: string): string | null {
  return [...title].length > 2048 || /[\u0000-\u001f\u007f-\u009f]/.test(title)
    ? '提示块标题不能超过 2048 个字符，且不能包含控制字符。'
    : null;
}

export function validTaskMarker(marker: string): boolean {
  return [...marker].length === 1 && !/[\u0000-\u001f\u007f-\u009f\[\]]/.test(marker);
}

export function blockAnchorError(
  value: string,
  document: ProseMirrorNode,
  exceptId?: string | null,
): string | null {
  if (!value) return null;
  if (!/^[A-Za-z0-9-]{1,128}$/.test(value)) return '块标识须为 1–128 个字母、数字或连字符。';
  let duplicate = false;
  document.descendants((node) => {
    if (node.attrs.block_id === value && node.attrs.id !== exceptId) duplicate = true;
  });
  return duplicate ? '此文档已使用该块标识，请选择另一个。' : null;
}

export function dimensionValue(value: string): number | null {
  if (!value.trim()) return null;
  const dimension = Number(value);
  if (!Number.isInteger(dimension) || dimension < 1 || dimension > 4096)
    throw new Error('嵌入尺寸须为 1–4096 的整数。');
  return dimension;
}

/**
 * The schema cannot express document-wide anchors, a top-only YAML block, or the
 * footnote ceiling, so those rules are enforced here, by hand, on every accepted
 * change.
 *
 * `richDocumentFacts` is the whole-document pass. It returns the multisets those
 * rules need, which is also what makes a later change cheap: a transaction that
 * replaced one range can settle them from the range it replaced instead of the
 * whole document (`richDocumentCheckForReplacement`). Anything else — a move, a
 * mark, several steps at once — falls back to the full pass, so a transaction
 * this module does not model can never slip past the guard.
 */
export type RichDocumentFacts = {
  anchors: Map<string, number>;
  footnoteLabels: Map<string, number>;
  frontmatter: number;
};

export type RichDocumentCheck = { error: string | null; facts: RichDocumentFacts };

const SOURCE_CHECKED_TYPES = new Set([
  'inlineMath',
  'blockMath',
  'comment',
  'commentBlock',
  'frontmatter',
]);
const LINK_CHECKED_TYPES = new Set(['wikiLink', 'wikiEmbed']);
const ANCHOR_PATTERN = /^[A-Za-z0-9-]{1,128}$/;
const FOOTNOTE_LIMIT = 4096;
const FRONTMATTER_ERROR = 'YAML 属性只能出现在文档顶部一次。此次操作未应用，内容仍保留在原处。';
const ANCHOR_FORMAT_ERROR = '块标识格式无效。此次操作未应用。';
const ANCHOR_DUPLICATE_ERROR =
  '粘贴或移动的内容含重复块标识。此次操作未应用，请先修改块标识或使用纯文本粘贴。';
const LINK_LABEL_ERROR = '内部链接别名不能超过 512 个字符，且不能包含控制字符。';
const EMBED_SIZE_ERROR = '嵌入尺寸须为 1–4096 的整数。';
const FOOTNOTE_DUPLICATE_ERROR = '此文档已使用该脚注标识，请选择另一个。';
const FOOTNOTE_LIMIT_ERROR = '脚注定义不能超过 4096 项。';

function emptyFacts(): RichDocumentFacts {
  return { anchors: new Map(), footnoteLabels: new Map(), frontmatter: 0 };
}

/**
 * The counts a change touched, so a rejected change can put them back. One
 * account is mutated in place rather than copied: copying a 20 000-entry map per
 * keystroke costs more than the whole-document pass this module exists to avoid.
 */
type Journal = [string, number | null][];

type FactEdits = {
  anchors: Journal;
  footnoteLabels: Journal;
  /** The document's frontmatter count before this change, the first time it moved. */
  frontmatter: number | null;
};

function startEdits(): FactEdits {
  return { anchors: [], footnoteLabels: [], frontmatter: null };
}

/** Counts a node in and returns the count already there: that is a repeat. */
function countIn(counts: Map<string, number>, key: string, journal: Journal | null): number {
  const seen = counts.get(key) ?? 0;
  if (journal) journal.push([key, seen || null]);
  counts.set(key, seen + 1);
  return seen;
}

function countOut(counts: Map<string, number>, key: string, journal: Journal | null): void {
  const seen = counts.get(key);
  if (seen === undefined) return;
  if (journal) journal.push([key, seen]);
  if (seen <= 1) counts.delete(key);
  else counts.set(key, seen - 1);
}

/** Undo a change's counts, newest first, so the account is exactly as it was. */
function rollback(facts: RichDocumentFacts, edits: FactEdits): void {
  for (const [key, previous] of [...edits.anchors].reverse()) {
    if (previous === null) facts.anchors.delete(key);
    else facts.anchors.set(key, previous);
  }
  for (const [key, previous] of [...edits.footnoteLabels].reverse()) {
    if (previous === null) facts.footnoteLabels.delete(key);
    else facts.footnoteLabels.set(key, previous);
  }
  if (edits.frontmatter !== null) facts.frontmatter = edits.frontmatter;
}

/**
 * One node's own rules, in the order a reader meets them; the first complaint is
 * the one the user sees. Counting and checking are the same step, so a repeat is
 * caught whether it arrived by paste or by an inserted range.
 */
function visitNode(
  node: ProseMirrorNode,
  at: number,
  parent: ProseMirrorNode | null,
  document: ProseMirrorNode,
  facts: RichDocumentFacts,
  record: FactEdits | null,
): string | null {
  const name = node.type.name;
  if (name === 'frontmatter') {
    if (record && record.frontmatter === null) record.frontmatter = facts.frontmatter;
    facts.frontmatter += 1;
    if (parent !== document || at !== 0) return FRONTMATTER_ERROR;
  }
  const anchor = node.attrs.block_id;
  if (anchor != null) {
    if (!ANCHOR_PATTERN.test(anchor)) return ANCHOR_FORMAT_ERROR;
    if (countIn(facts.anchors, anchor, record ? record.anchors : null))
      return ANCHOR_DUPLICATE_ERROR;
  }
  if (SOURCE_CHECKED_TYPES.has(name)) {
    const error = sourceError(
      String(node.attrs.source ?? ''),
      name === 'inlineMath' || name === 'blockMath',
    );
    if (error) return error;
  }
  if (LINK_CHECKED_TYPES.has(name)) {
    const error = wikiTargetError(String(node.attrs.wiki_target ?? ''));
    if (error) return error;
    const label = node.attrs.label;
    if (
      label != null &&
      ([...String(label)].length > 512 || /[\u0000-\u001f\u007f-\u009f]/.test(label))
    )
      return LINK_LABEL_ERROR;
    for (const size of [node.attrs.embed_width, node.attrs.embed_height])
      if (size != null && (!Number.isInteger(size) || size < 1 || size > 4096))
        return EMBED_SIZE_ERROR;
    return null;
  }
  if (name === 'footnoteReference' || name === 'footnoteDefinition') {
    const error = footnoteLabelError(String(node.attrs.label ?? ''));
    if (error) return error;
  }
  if (name === 'footnoteDefinition') {
    const label = footnoteKey(String(node.attrs.label ?? ''));
    if (countIn(facts.footnoteLabels, label, record ? record.footnoteLabels : null))
      return FOOTNOTE_DUPLICATE_ERROR;
    if (facts.footnoteLabels.size > FOOTNOTE_LIMIT) return FOOTNOTE_LIMIT_ERROR;
  }
  if (name === 'callout') {
    const error = calloutTypeError(String(node.attrs.callout_type ?? ''));
    if (error) return error;
    return calloutTitleError(String(node.attrs.title ?? ''));
  }
  return null;
}

/** Forget deleted content, so a node that merely moved is never its own repeat. */
function forgetNode(node: ProseMirrorNode, facts: RichDocumentFacts, record: FactEdits): void {
  const anchor = node.attrs.block_id;
  if (anchor != null) countOut(facts.anchors, anchor, record.anchors);
  const name = node.type.name;
  if (name === 'frontmatter') {
    if (record.frontmatter === null) record.frontmatter = facts.frontmatter;
    facts.frontmatter = Math.max(0, facts.frontmatter - 1);
  }
  if (name === 'footnoteDefinition')
    countOut(
      facts.footnoteLabels,
      footnoteKey(String(node.attrs.label ?? '')),
      record.footnoteLabels,
    );
}

/** The whole document once: the verdict plus the multisets a later change needs. */
export function richDocumentFacts(document: ProseMirrorNode): RichDocumentCheck {
  const facts = emptyFacts();
  let error: string | null = null;
  document.descendants((node, at, parent) => {
    // `descendants` keeps walking the siblings after a `false`, so the first
    // verdict has to be kept here rather than overwritten by the next node.
    if (error) return false;
    // Text carries none of the checked attributes and is most of a long note.
    if (node.isText) return true;
    error = visitNode(node, at, parent, document, facts, null);
    return error === null;
  });
  return { error, facts };
}

export function richDocumentError(document: ProseMirrorNode): string | null {
  return richDocumentFacts(document).error;
}

/**
 * The same rules for a change that replaced exactly one range, given in the
 * coordinates of the document before and after the change. The account is updated
 * in place and rolled back if the change is refused, so the caller keeps a valid
 * account of the document that stays open. Returns `null` when the ranges are not
 * a replacement, which tells the caller to do the full pass.
 */
export function richDocumentCheckForReplacement(
  before: ProseMirrorNode,
  document: ProseMirrorNode,
  previous: RichDocumentFacts,
  removed: readonly [number, number],
  inserted: readonly [number, number],
): RichDocumentCheck | null {
  const [from, to] = removed;
  const [start, end] = inserted;
  if (!(from <= to) || !(start <= end)) return null;
  const edits = startEdits();
  let error: string | null = null;
  if (to > from)
    before.nodesBetween(from, to, (node, at) => {
      if (error) return false;
      // A node the range only reached into survives the change untouched.
      if (node.isText || at < from || at + node.nodeSize > to) return true;
      forgetNode(node, previous, edits);
      return true;
    });
  if (!error && end > start)
    document.nodesBetween(start, end, (node, at, parent) => {
      if (error) return false;
      if (node.isText || at < start || at + node.nodeSize > end) return true;
      error = visitNode(node, at, parent, document, previous, edits);
      return error === null;
    });
  // Where a whole document's nodes sit is the one thing a range scan cannot see:
  // what survived plus what arrived may leave the YAML block misplaced or doubled.
  if (
    !error &&
    previous.frontmatter > 0 &&
    (previous.frontmatter > 1 || document.firstChild?.type.name !== 'frontmatter')
  )
    error = FRONTMATTER_ERROR;
  if (error) {
    rollback(previous, edits);
    return { error, facts: previous };
  }
  return { error: null, facts: previous };
}

export function findNodePosition(document: ProseMirrorNode, id: string): number | undefined {
  let position: number | undefined;
  document.descendants((node, at) => {
    if (node.attrs.id === id) position = at;
    return position === undefined;
  });
  return position;
}

export function selectedBlock(state: EditorState, type?: string) {
  const selection = state.selection;
  if (
    selection instanceof NodeSelection &&
    selection.node.isBlock &&
    (!type || selection.node.type.name === type)
  )
    return { node: selection.node, position: selection.from };
  for (let depth = selection.$from.depth; depth > 0; --depth) {
    const node = selection.$from.node(depth);
    if (node.isBlock && (!type || node.type.name === type))
      return { node, position: selection.$from.before(depth) };
  }
  return null;
}

const editableNodeKinds = new Set([
  'inlineMath',
  'blockMath',
  'wikiLink',
  'wikiEmbed',
  'comment',
  'commentBlock',
  'footnoteReference',
  'footnoteDefinition',
  'frontmatter',
  'callout',
]);
export function selectedRichEdit(state: EditorState): RichEditRequest | null {
  const selection = state.selection;
  if (!(selection instanceof NodeSelection) || !editableNodeKinds.has(selection.node.type.name))
    return null;
  return { kind: selection.node.type.name as RichEditKind, position: selection.from };
}

const footnoteCache = new WeakMap<
  ProseMirrorNode,
  { order: Map<string, number>; definitions: Map<string, number> }
>();
export function footnoteIndex(document: ProseMirrorNode) {
  const cached = footnoteCache.get(document);
  if (cached) return cached;
  const order = new Map<string, number>();
  const definitions = new Map<string, number>();
  document.descendants((node) => {
    const label = footnoteKey(String(node.attrs.label ?? ''));
    if (node.type.name === 'footnoteReference' && !order.has(label))
      order.set(label, order.size + 1);
    if (node.type.name === 'footnoteDefinition')
      definitions.set(label, (definitions.get(label) ?? 0) + 1);
  });
  const result = { order, definitions };
  footnoteCache.set(document, result);
  return result;
}

export function nextFootnoteLabel(document: ProseMirrorNode): string {
  const { order, definitions } = footnoteIndex(document);
  let index = 1;
  while (order.has(String(index)) || definitions.has(String(index))) ++index;
  return String(index);
}

export function insertFootnote(
  state: EditorState,
  label: string,
  body: string,
  uuid: () => string = () => crypto.randomUUID(),
): Transaction {
  const error = footnoteLabelError(label) || sourceError(body);
  if (error) throw new Error(error);
  if (footnoteIndex(state.doc).definitions.has(footnoteKey(label)))
    throw new Error('此脚注标识已有定义，请使用其他标识或插入已有脚注引用。');
  const reference = state.schema.nodes.footnoteReference.create(
    { label },
    null,
    state.storedMarks ?? state.selection.$from.marks(),
  );
  const paragraph = state.schema.nodes.paragraph.create(
    { id: uuid() },
    body ? state.schema.text(body) : null,
  );
  const definition = state.schema.nodes.footnoteDefinition.create({ id: uuid(), label }, [
    paragraph,
  ]);
  const transaction = state.tr.replaceSelectionWith(reference, false);
  const cursor = transaction.selection.to;
  transaction.insert(transaction.doc.content.size, definition);
  transaction.setSelection(TextSelection.near(transaction.doc.resolve(cursor)));
  return transaction.scrollIntoView();
}

export function renameFootnoteDefinition(
  state: EditorState,
  position: number,
  label: string,
): Transaction {
  const node = state.doc.nodeAt(position);
  if (!node || node.type.name !== 'footnoteDefinition')
    throw new Error('脚注已改变，请重新打开后编辑。');
  const error = footnoteLabelError(label);
  if (error) throw new Error(error);
  const old = String(node.attrs.label ?? '');
  const oldKey = footnoteKey(old),
    key = footnoteKey(label);
  const { definitions } = footnoteIndex(state.doc);
  if (key !== oldKey && definitions.has(key))
    throw new Error('此脚注标识已有定义，请使用其他标识。');
  if ((definitions.get(oldKey) ?? 0) > 1)
    throw new Error('原脚注标识有多个定义，请先在正文中消除重复定义。');
  const transaction = state.tr.setNodeMarkup(position, undefined, { ...node.attrs, label });
  state.doc.descendants((child, at) => {
    if (
      child.type.name === 'footnoteReference' &&
      footnoteKey(String(child.attrs.label ?? '')) === oldKey
    )
      transaction.setNodeMarkup(at, undefined, { ...child.attrs, label });
  });
  return transaction;
}
