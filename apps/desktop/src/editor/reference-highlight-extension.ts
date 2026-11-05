import { Extension } from '@tiptap/core';
import type { Node as ProseMirrorNode } from '@tiptap/pm/model';
import { Plugin, PluginKey, type EditorState } from '@tiptap/pm/state';
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view';

export const REFERENCE_HIGHLIGHT_MS = 2200;

type Anchor = { attribute: 'id' | 'occurrence_id'; value: string };
type HighlightState = {
  requestId: string | null;
  anchor: Anchor | null;
  decorations: DecorationSet;
};
type HighlightAction =
  { kind: 'show'; requestId: string; anchor: Anchor } | { kind: 'clear'; requestId: string };

const empty = (): HighlightState => ({
  requestId: null,
  anchor: null,
  decorations: DecorationSet.empty,
});

export const referenceHighlightKey = new PluginKey<HighlightState>('pkmsReferenceHighlight');

function locate(document: ProseMirrorNode, anchor: Anchor): DecorationSet {
  let decoration: Decoration | undefined;
  document.descendants((node, position) => {
    if (decoration) return false;
    if (node.attrs[anchor.attribute] !== anchor.value) return;
    const attrs = { class: 'reference-highlight' };
    decoration = node.isInline
      ? Decoration.inline(position, position + node.nodeSize, attrs)
      : Decoration.node(position, position + node.nodeSize, attrs);
    return false;
  });
  return decoration ? DecorationSet.create(document, [decoration]) : DecorationSet.empty;
}

function highlightTransaction(state: EditorState, action: HighlightAction) {
  return state.tr
    .setMeta(referenceHighlightKey, action)
    .setMeta('addToHistory', false)
    .setMeta('preventUpdate', true);
}

export function createReferenceHighlightPlugin() {
  return new Plugin<HighlightState>({
    key: referenceHighlightKey,
    state: {
      init: empty,
      apply(transaction, previous) {
        const action = transaction.getMeta(referenceHighlightKey) as HighlightAction | undefined;
        if (action?.kind === 'clear')
          return previous.requestId === action.requestId ? empty() : previous;
        if (action?.kind === 'show') {
          const decorations = locate(transaction.doc, action.anchor);
          return decorations.find().length
            ? { requestId: action.requestId, anchor: action.anchor, decorations }
            : empty();
        }
        if (!transaction.docChanged || !previous.anchor) return previous;
        const decorations = previous.decorations.map(transaction.mapping, transaction.doc);
        const [decoration] = decorations.find();
        const node = decoration && transaction.doc.nodeAt(decoration.from);
        // A deleted/replaced occurrence must never fall through to its neighbour.
        if (
          !decoration ||
          !node ||
          node.attrs[previous.anchor.attribute] !== previous.anchor.value ||
          decoration.to - decoration.from !== node.nodeSize
        )
          return empty();
        return { ...previous, decorations };
      },
    },
    props: {
      decorations: (state) =>
        referenceHighlightKey.getState(state)?.decorations ?? DecorationSet.empty,
    },
    view(view) {
      let timer: ReturnType<typeof setTimeout> | undefined;
      let activeRequest: string | null = null;
      let live = true;
      const clearTimer = () => {
        if (timer !== undefined) clearTimeout(timer);
        timer = undefined;
      };
      const update = (current: EditorView) => {
        const requestId = referenceHighlightKey.getState(current.state)?.requestId ?? null;
        if (requestId === activeRequest) return;
        clearTimer();
        activeRequest = requestId;
        if (requestId === null) return;
        timer = setTimeout(() => {
          timer = undefined;
          if (
            live &&
            !current.isDestroyed &&
            referenceHighlightKey.getState(current.state)?.requestId === requestId
          )
            current.dispatch(highlightTransaction(current.state, { kind: 'clear', requestId }));
        }, REFERENCE_HIGHLIGHT_MS);
      };
      update(view);
      return {
        update,
        destroy() {
          live = false;
          clearTimer();
        },
      };
    },
  });
}

export const referenceHighlight = Extension.create({
  name: 'referenceHighlight',
  addProseMirrorPlugins: () => [createReferenceHighlightPlugin()],
});

/** Editor-only presentation state: no document steps, selection change or history entry. */
export function showReferenceHighlight(
  view: EditorView,
  request: { requestId: string; blockId: string | null; occurrenceId?: string },
): (() => void) | undefined {
  if (view.isDestroyed || !referenceHighlightKey.getState(view.state)) return;
  const value = request.occurrenceId ?? request.blockId;
  if (!value) return;
  const anchor: Anchor = {
    attribute: request.occurrenceId ? 'occurrence_id' : 'id',
    value,
  };
  view.dispatch(
    highlightTransaction(view.state, { kind: 'show', requestId: request.requestId, anchor }),
  );
  if (referenceHighlightKey.getState(view.state)?.requestId !== request.requestId) return;
  return () => {
    if (
      !view.isDestroyed &&
      referenceHighlightKey.getState(view.state)?.requestId === request.requestId
    )
      view.dispatch(
        highlightTransaction(view.state, { kind: 'clear', requestId: request.requestId }),
      );
  };
}
