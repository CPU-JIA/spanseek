import { Extension } from '@tiptap/core';
import type { Node as ProseMirrorNode } from '@tiptap/pm/model';
import { Plugin, PluginKey, TextSelection, type EditorState } from '@tiptap/pm/state';
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view';
import { activeLanguage } from '../language.ts';
import {
  HEADING_FOLD_ID,
  headingFoldSectionAt,
  headingFoldSections,
  loadHeadingFoldIds,
  saveHeadingFoldIds,
  storedHeadingFoldIds,
  type HeadingFoldBlock,
  type HeadingFoldSection,
} from './heading-fold-model';

export type HeadingFoldState = {
  collapsed: ReadonlySet<string>;
  decorations: DecorationSet;
};

export type HeadingFoldOptions = {
  getStorageKey: () => string;
};

type HeadingFoldAction =
  | { kind: 'toggle'; id: string }
  | { kind: 'set'; id: string; collapsed: boolean }
  | { kind: 'all'; collapsed: boolean };

export const headingFoldKey = new PluginKey<HeadingFoldState>('pkmsHeadingFold');

export function headingFoldBlocks(document: ProseMirrorNode): HeadingFoldBlock[] {
  const blocks: HeadingFoldBlock[] = [];
  document.forEach((node, offset) => {
    const id =
      typeof node.attrs.id === 'string' && HEADING_FOLD_ID.test(node.attrs.id)
        ? node.attrs.id
        : null;
    blocks.push({
      type: node.type.name,
      level: node.type.name === 'heading' ? Number(node.attrs.level ?? 0) : 0,
      id,
      from: offset,
      to: offset + node.nodeSize,
    });
  });
  return blocks;
}

export function headingFoldSelection(state: EditorState) {
  const blocks = headingFoldBlocks(state.doc);
  const section = headingFoldSectionAt(blocks, state.selection.from);
  if (!section) return null;
  const collapsed = headingFoldKey.getState(state)?.collapsed.has(section.id) ?? false;
  return { ...section, collapsed };
}

export function setHeadingFolded(view: EditorView, id: string, collapsed: boolean): boolean {
  if (view.isDestroyed || !headingFoldKey.getState(view.state)) return false;
  const blocks = headingFoldBlocks(view.state.doc);
  const section = headingFoldSections(blocks).find((item) => item.id === id);
  let transaction = view.state.tr
    .setMeta(headingFoldKey, { kind: 'set', id, collapsed } satisfies HeadingFoldAction)
    .setMeta('addToHistory', false)
    .setMeta('preventUpdate', true);
  if (collapsed && section) {
    const heading = blocks[section.headingIndex];
    const end =
      section.nextIndex < blocks.length
        ? blocks[section.nextIndex].from
        : view.state.doc.content.size;
    if (view.state.selection.to >= heading.to && view.state.selection.from < end) {
      transaction = transaction.setSelection(
        TextSelection.near(view.state.doc.resolve(heading.to - 1), -1),
      );
    }
  }
  view.dispatch(transaction);
  return true;
}

export function toggleHeadingFolded(view: EditorView, id: string): boolean {
  const collapsed = headingFoldKey.getState(view.state)?.collapsed.has(id) ?? false;
  return setHeadingFolded(view, id, !collapsed);
}

export function setAllHeadingsFolded(view: EditorView, collapsed: boolean): boolean {
  if (view.isDestroyed || !headingFoldKey.getState(view.state)) return false;
  view.dispatch(
    view.state.tr
      .setMeta(headingFoldKey, { kind: 'all', collapsed } satisfies HeadingFoldAction)
      .setMeta('addToHistory', false)
      .setMeta('preventUpdate', true),
  );
  return true;
}

function headingIds(blocks: readonly HeadingFoldBlock[]): Set<string> {
  return new Set(
    blocks.flatMap((block) => (block.type === 'heading' && block.id ? [block.id] : [])),
  );
}

function foldButton(view: EditorView, id: string, collapsed: boolean): HTMLButtonElement {
  const english = activeLanguage() === 'en';
  const label = collapsed
    ? english
      ? 'Expand section'
      : '展开当前标题'
    : english
      ? 'Collapse section'
      : '收起当前标题';
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'heading-fold-toggle';
  button.contentEditable = 'false';
  button.dataset.headingFoldId = id;
  button.setAttribute('aria-expanded', String(!collapsed));
  button.setAttribute('aria-label', label);
  button.title = label;
  button.innerHTML = `<svg aria-hidden="true" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="${collapsed ? 'm9 18 6-6-6-6' : 'm6 9 6 6 6-6'}"/></svg>`;
  button.addEventListener('mousedown', (event) => event.preventDefault());
  button.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    toggleHeadingFolded(view, id);
  });
  return button;
}

function foldedSummary(count: number): HTMLSpanElement {
  const summary = document.createElement('span');
  summary.className = 'heading-fold-summary';
  summary.contentEditable = 'false';
  summary.textContent =
    activeLanguage() === 'en' ? `${count} blocks folded` : `已折叠 ${count} 个块`;
  return summary;
}

export function buildHeadingFoldDecorations(
  document: ProseMirrorNode,
  collapsed: ReadonlySet<string>,
): DecorationSet {
  const blocks = headingFoldBlocks(document);
  const sections: HeadingFoldSection[] = headingFoldSections(blocks);
  const decorations: Decoration[] = [];
  for (const section of sections) {
    const heading = blocks[section.headingIndex];
    const isCollapsed = collapsed.has(section.id);
    decorations.push(
      Decoration.node(heading.from, heading.to, {
        class: isCollapsed ? 'heading-fold-heading is-collapsed' : 'heading-fold-heading',
      }),
    );
    decorations.push(
      Decoration.widget(heading.from + 1, (view) => foldButton(view, section.id, isCollapsed), {
        side: -1,
        key: `heading-fold-toggle-${section.id}-${isCollapsed ? 'collapsed' : 'open'}`,
        stopEvent: (event) =>
          event.target instanceof Element && !!event.target.closest('.heading-fold-toggle'),
      }),
    );
    if (!isCollapsed || !section.foldedCount) continue;
    decorations.push(
      Decoration.widget(heading.to - 1, () => foldedSummary(section.foldedCount), {
        side: 1,
        key: `heading-fold-summary-${section.id}`,
      }),
    );
    for (let index = section.headingIndex + 1; index < section.nextIndex; index += 1) {
      const block = blocks[index];
      decorations.push(Decoration.node(block.from, block.to, { class: 'heading-fold-hidden' }));
    }
  }
  return DecorationSet.create(document, decorations);
}

function foldPlugin(options: HeadingFoldOptions) {
  return new Plugin<HeadingFoldState>({
    key: headingFoldKey,
    state: {
      init: (_, state) => {
        const key = options.getStorageKey();
        const collapsed = key ? loadHeadingFoldIds(window.localStorage, key) : new Set<string>();
        return {
          collapsed,
          decorations: buildHeadingFoldDecorations(state.doc, collapsed),
        };
      },
      apply(transaction, previous, _oldState, newState) {
        const action = transaction.getMeta(headingFoldKey) as HeadingFoldAction | undefined;
        if (!action && !transaction.docChanged) return previous;
        const blocks = headingFoldBlocks(newState.doc);
        const available = headingIds(blocks);
        let collapsed = new Set([...previous.collapsed].filter((id) => available.has(id)));
        if (action?.kind === 'toggle') {
          if (collapsed.has(action.id)) collapsed.delete(action.id);
          else collapsed.add(action.id);
        } else if (action?.kind === 'set') {
          if (action.collapsed) collapsed.add(action.id);
          else collapsed.delete(action.id);
        } else if (action?.kind === 'all') {
          collapsed = action.collapsed ? new Set(available) : new Set();
        }
        return {
          collapsed,
          decorations: buildHeadingFoldDecorations(newState.doc, collapsed),
        };
      },
    },
    props: {
      decorations: (state) => headingFoldKey.getState(state)?.decorations ?? DecorationSet.empty,
    },
    view(view) {
      let saved = storedHeadingFoldIds(
        headingFoldKey.getState(view.state)?.collapsed ?? [],
        headingIds(headingFoldBlocks(view.state.doc)),
      ).join('\n');
      return {
        update(current) {
          const state = headingFoldKey.getState(current.state);
          if (!state) return;
          const available = headingIds(headingFoldBlocks(current.state.doc));
          const next = storedHeadingFoldIds(state.collapsed, available).join('\n');
          if (next === saved) return;
          saved = next;
          const key = options.getStorageKey();
          if (key) saveHeadingFoldIds(window.localStorage, key, state.collapsed, available);
        },
      };
    },
  });
}

export const headingFoldExtension = Extension.create<HeadingFoldOptions>({
  name: 'headingFold',
  addOptions() {
    return { getStorageKey: () => '' };
  },
  addProseMirrorPlugins() {
    return [foldPlugin(this.options)];
  },
});
