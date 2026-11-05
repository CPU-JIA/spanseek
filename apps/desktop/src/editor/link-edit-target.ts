import { Extension, getMarkRange } from '@tiptap/core';
import {
  TextSelection,
  type EditorState,
  type SelectionBookmark,
  type Transaction,
} from '@tiptap/pm/state';
import type { Slice } from '@tiptap/pm/model';
import { closeHistory } from '@tiptap/pm/history';
import { storedOrdinaryLink } from '../rich/link-target.ts';

/** Tiptap's built-in link omits title, which already belongs to the persisted link contract. */
export const OrdinaryLinkPresentation = Extension.create({
  name: 'ordinaryLinkPresentation',
  addGlobalAttributes: () => [
    {
      types: ['link'],
      attributes: {
        title: {
          default: null,
          parseHTML: (element: HTMLElement) => element.getAttribute('title'),
          renderHTML: (attributes: Record<string, unknown>) =>
            attributes.title == null ? {} : { title: String(attributes.title) },
        },
      },
    },
  ],
});

export function selectedOrdinaryLink(state: EditorState) {
  const type = state.schema.marks.link;
  if (!type) return null;
  const range = getMarkRange(state.selection.$from, type);
  if (!range || state.selection.to > range.to) return null;
  const mark = state.doc.resolve(range.from).nodeAfter?.marks.find((value) => value.type === type);
  return mark ? { ...range, href: String(mark.attrs.href ?? ''), attrs: mark.attrs } : null;
}

export interface LinkEditTarget {
  from: number;
  to: number;
  original: SelectionBookmark;
  content: Slice;
  href: string;
  removed: boolean;
}
export function captureLinkEditTarget(state: EditorState): LinkEditTarget {
  const link = selectedOrdinaryLink(state);
  const { from, to } = link ?? state.selection;
  return {
    from,
    to,
    original: state.selection.getBookmark(),
    content: state.doc.slice(from, to),
    href: link?.href ?? '',
    removed: false,
  };
}
export function mapLinkEditTarget(
  target: LinkEditTarget,
  transaction: Transaction,
): LinkEditTarget {
  const from = transaction.mapping.mapResult(target.from, 1);
  const to = transaction.mapping.mapResult(target.to, target.from === target.to ? 1 : -1);
  return {
    ...target,
    from: from.pos,
    to: to.pos,
    original: target.original.map(transaction.mapping),
    removed: target.removed || from.deletedAcross || to.deletedAcross,
  };
}
export function editOrdinaryLink(
  state: EditorState,
  target: LinkEditTarget,
  href: string | null,
): Transaction {
  if (
    target.removed ||
    target.to < target.from ||
    target.to > state.doc.content.size ||
    !state.doc.slice(target.from, target.to).eq(target.content)
  )
    throw new Error('链接或所选文字在编辑期间已改变，请关闭后重新打开；输入仍保留。');
  const type = state.schema.marks.link;
  if (!type) throw new Error('当前编辑器无法编辑链接。');
  const value = href === null ? null : storedOrdinaryLink(href);
  if (value !== null) {
    let hasInline =
      target.from === target.to && state.doc.resolve(target.from).parent.inlineContent;
    let unsupported = false;
    const cursorMarks = state.storedMarks ?? state.doc.resolve(target.from).marks();
    if (target.from === target.to && cursorMarks.some((mark) => mark.type.spec.code))
      unsupported = true;
    state.doc.nodesBetween(target.from, target.to, (node, _at, parent) => {
      if (!node.isInline) return;
      hasInline = true;
      if (
        node.marks.some((mark) => mark.type.spec.code) ||
        parent?.type.spec.code ||
        !parent?.type.allowsMarkType(type)
      )
        unsupported = true;
    });
    if (!hasInline || unsupported)
      throw new Error('请在普通正文中插入链接；代码或不支持链接的块不能应用链接。');
  }
  const transaction = closeHistory(
    state.tr.setSelection(
      TextSelection.between(state.doc.resolve(target.from), state.doc.resolve(target.to)),
    ),
  );
  if (value === null)
    return transaction.removeMark(target.from, target.to, type).setStoredMarks(null);
  if (target.from === target.to) {
    if (
      !state.doc.resolve(target.from).parent.inlineContent ||
      state.doc.resolve(target.from).parent.type.spec.code
    )
      throw new Error('请在普通正文中插入链接。');
    const marks = type
      .create({ href: value })
      .addToSet(state.storedMarks ?? state.doc.resolve(target.from).marks());
    return transaction.replaceSelectionWith(state.schema.text(value, marks), false);
  }
  state.doc.nodesBetween(target.from, target.to, (node, at) => {
    if (!node.isInline) return;
    const existing = node.marks.find((mark) => mark.type === type);
    transaction.addMark(
      Math.max(at, target.from),
      Math.min(at + node.nodeSize, target.to),
      type.create({ ...existing?.attrs, href: value }),
    );
  });
  return transaction;
}
