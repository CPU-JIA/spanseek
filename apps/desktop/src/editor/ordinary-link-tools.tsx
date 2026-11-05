import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { createPortal } from 'react-dom';
import type { Editor, EditorEvents } from '@tiptap/core';
import { useEditorState } from '@tiptap/react';
import { Copy, ExternalLink, Link2Off, Pencil, X } from 'lucide-react';
import { message } from '../api';
import { useI18n } from '../i18n';
import { Tooltip } from '../ui/Tooltip';
import { openDocumentLink } from '../rich/DocumentLink';
import type { DraftController } from './draft-controller';
import {
  captureLinkEditTarget,
  editOrdinaryLink,
  mapLinkEditTarget,
  selectedOrdinaryLink,
} from './link-edit-target';

function getMountedEditorView(editor: Editor) {
  // Tiptap also reports an unmounted, reusable editor as destroyed; its view proxy has no DOM.
  return editor.isDestroyed ? null : editor.view;
}

function useMountedEditorView(editor: Editor) {
  const subscribe = useCallback(
    (changed: () => void) => {
      editor.on('mount', changed);
      editor.on('unmount', changed);
      editor.on('destroy', changed);
      return () => {
        editor.off('mount', changed);
        editor.off('unmount', changed);
        editor.off('destroy', changed);
      };
    },
    [editor],
  );
  const snapshot = useCallback(() => getMountedEditorView(editor), [editor]);
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

function LinkEditDialog({
  editor,
  controller,
  pane,
  onClose,
}: {
  editor: Editor;
  controller: DraftController;
  pane: string;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const id = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  const [initial] = useState(() => captureLinkEditTarget(editor.state));
  const target = useRef(initial);
  const [value, setValue] = useState(initial.href);
  const [error, setError] = useState<string | null>(null);
  const ready = useRef(false);
  const composing = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const applied = useRef(false);
  const token = pane + '/link-dialog/' + id;
  useLayoutEffect(() => {
    let finish: (() => void) | undefined;
    try {
      finish = controller.beginAction();
      ready.current = true;
    } catch (cause) {
      setError(message(cause));
    }
    const element = dialog.current!;
    element.showModal();
    const input = element.querySelector('input');
    input?.focus();
    input?.select();
    const map = ({ transaction, appendedTransactions }: EditorEvents['transaction']) => {
      for (const change of [transaction, ...appendedTransactions])
        target.current = mapLinkEditTarget(target.current, change);
    };
    editor.on('transaction', map);
    return () => {
      editor.off('transaction', map);
      clearTimeout(timer.current);
      controller.setComposing(token, false);
      finish?.();
      element.close();
      const view = getMountedEditorView(editor);
      if (view) {
        if (!applied.current)
          view.dispatch(
            editor.state.tr
              .setSelection(target.current.original.resolve(editor.state.doc))
              .setMeta('addToHistory', false),
          );
        editor.commands.focus(undefined, { scrollIntoView: false });
      }
    };
  }, [editor, controller, token]);
  function close() {
    if (composing.current) {
      setError(t('请先完成当前输入法输入。', 'Finish the current input composition first.'));
      return;
    }
    onClose();
  }
  function apply(remove = false) {
    if (composing.current) return;
    const view = getMountedEditorView(editor);
    if (!ready.current || !view || !editor.isEditable) {
      setError(t('编辑器暂时不可写。', 'The editor is not editable right now.'));
      return;
    }
    if (view.composing) return;
    try {
      const transaction = editOrdinaryLink(editor.state, target.current, remove ? null : value);
      view.dispatch(transaction.scrollIntoView());
      applied.current = true;
      onClose();
    } catch (cause) {
      setError(message(cause));
    }
  }
  return (
    <dialog
      ref={dialog}
      className="rich-edit-dialog ordinary-link-dialog"
      aria-labelledby={id}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
      onKeyDown={(event) => {
        event.stopPropagation();
        if ((event.nativeEvent.isComposing || event.keyCode === 229) && event.key === 'Escape')
          event.preventDefault();
      }}
      onCompositionStart={() => {
        clearTimeout(timer.current);
        composing.current = true;
        controller.setComposing(token, true);
      }}
      onCompositionEnd={() => {
        timer.current = setTimeout(() => {
          composing.current = false;
          controller.setComposing(token, false);
        }, 0);
      }}
    >
      <form
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          apply();
        }}
      >
        <header>
          <h2 id={id}>
            {initial.href ? t('编辑链接', 'Edit link') : t('插入链接', 'Insert link')}
          </h2>
          <button
            type="button"
            className="icon-button"
            aria-label={t('关闭链接编辑', 'Close link editor')}
            onClick={close}
          >
            <X size={18} />
          </button>
        </header>
        <div className="rich-edit-fields">
          <label>
            {t('链接地址', 'Link URL')}
            <input
              value={value}
              onChange={(event) => {
                setValue(event.target.value);
                setError(null);
              }}
              placeholder="https://example.com"
              inputMode="url"
              spellCheck={false}
            />
          </label>
          <p className="rich-edit-help">
            {t(
              '编辑时 Ctrl+点击或 Alt+Enter 打开；阅读时直接点击。本文锚点请使用内部链接。',
              'Use Ctrl+click or Alt+Enter while editing; click directly while reading. Use an internal link for a document anchor.',
            )}
          </p>
        </div>
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <div className="button-row">
          {initial.href && (
            <button type="button" className="secondary-button" onClick={() => apply(true)}>
              {t('移除链接', 'Remove link')}
            </button>
          )}
          <button type="button" className="secondary-button" onClick={close}>
            {t('取消', 'Cancel')}
          </button>
          <button type="submit" className="primary-button">
            {t('保存链接', 'Save link')}
          </button>
        </div>
      </form>
    </dialog>
  );
}

export function OrdinaryLinkTools({
  editor,
  controller,
  pane,
  editing,
  onEdit,
  onClose,
  onNotice,
}: {
  editor: Editor;
  controller: DraftController;
  pane: string;
  editing: boolean;
  onEdit: () => void;
  onClose: () => void;
  onNotice: (notice: string | null) => void;
}) {
  const { t } = useI18n();
  const view = useMountedEditorView(editor);
  const link = useEditorState({
    editor,
    selector: ({ editor: current }) => selectedOrdinaryLink(current.state),
  });
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const key = link ? `${link.from}/${link.to}/${link.href}` : '';
  const visible = !!view && !!link && !editing && dismissed !== key;
  const callbacks = useRef({ onEdit, onNotice });
  callbacks.current = { onEdit, onNotice };
  const opening = useRef(false);
  useEffect(() => setNotice(null), [key]);
  async function copy(href: string) {
    try {
      if (!navigator.clipboard)
        throw new Error(
          t(
            '当前环境无法复制地址，请在链接编辑框中复制。',
            'Clipboard is unavailable. Copy the URL from the link editor.',
          ),
        );
      await navigator.clipboard.writeText(href);
      setNotice(t('地址已复制', 'Address copied'));
    } catch (cause) {
      setNotice(message(cause));
    }
  }
  async function open(href: string) {
    const currentView = getMountedEditorView(editor);
    if (opening.current || !currentView || currentView.composing) return;
    opening.current = true;
    try {
      await openDocumentLink(href, currentView.dom);
      callbacks.current.onNotice(null);
    } catch (cause) {
      callbacks.current.onNotice(message(cause));
    } finally {
      opening.current = false;
    }
  }
  useEffect(() => {
    if (!view || getMountedEditorView(editor) !== view) return;
    const root = view.dom;
    const pointer = (event: MouseEvent) => {
      if (
        getMountedEditorView(editor) !== view ||
        !(event.ctrlKey || event.metaKey) ||
        event.button !== 0
      )
        return;
      const anchor = event.target instanceof Element ? event.target.closest('a[href]') : null;
      if (!anchor || !root.contains(anchor)) return;
      // ProseMirror's Mod+mousedown selects a whole node. Opening a link must not replace the editing selection.
      event.preventDefault();
      event.stopPropagation();
    };
    const click = (event: MouseEvent) => {
      if (getMountedEditorView(editor) !== view) return;
      const anchor =
        event.target instanceof Element ? event.target.closest<HTMLAnchorElement>('a[href]') : null;
      if (!anchor || !root.contains(anchor)) return;
      event.preventDefault();
      setDismissed(null);
      if (event.ctrlKey || event.metaKey) void open(anchor.getAttribute('href') ?? '');
    };
    const keyboard = (event: KeyboardEvent) => {
      if (
        getMountedEditorView(editor) !== view ||
        event.isComposing ||
        event.keyCode === 229 ||
        view.composing
      )
        return;
      if ((event.ctrlKey || event.metaKey) && !event.shiftKey && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        event.stopPropagation();
        callbacks.current.onEdit();
      } else if (event.altKey && event.key === 'Enter') {
        const target = selectedOrdinaryLink(editor.state);
        if (target) {
          event.preventDefault();
          event.stopPropagation();
          void open(target.href);
        }
      }
    };
    root.addEventListener('click', click);
    root.addEventListener('mousedown', pointer, true);
    root.addEventListener('keydown', keyboard);
    return () => {
      root.removeEventListener('click', click);
      root.removeEventListener('mousedown', pointer, true);
      root.removeEventListener('keydown', keyboard);
    };
  }, [editor, view]);
  useLayoutEffect(() => {
    if (!visible || !link || !view) {
      setPosition(null);
      return;
    }
    const update = () => {
      if (getMountedEditorView(editor) !== view || !panel.current) return;
      const start = view.coordsAtPos(Math.min(link.from, editor.state.doc.content.size));
      const end = view.coordsAtPos(Math.min(link.to, editor.state.doc.content.size));
      const bounds = panel.current.getBoundingClientRect();
      const above = start.top - bounds.height - 7;
      setPosition({
        left: Math.max(8, Math.min(start.left, window.innerWidth - bounds.width - 8)),
        top: Math.max(
          8,
          Math.min(above >= 8 ? above : end.bottom + 7, window.innerHeight - bounds.height - 8),
        ),
      });
    };
    update();
    window.addEventListener('resize', update);
    window.addEventListener('scroll', update, true);
    return () => {
      window.removeEventListener('resize', update);
      window.removeEventListener('scroll', update, true);
    };
  }, [editor, view, visible, key, notice]);
  useEffect(() => {
    if (!visible || !view || getMountedEditorView(editor) !== view) return;
    const root = view.dom;
    const outside = (event: Event) => {
      if (getMountedEditorView(editor) !== view) return;
      const target = event.target as Node;
      if (!root.contains(target) && !panel.current?.contains(target)) setDismissed(key);
    };
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('focusin', outside);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('focusin', outside);
    };
  }, [editor, view, visible, key]);
  function remove() {
    const currentView = getMountedEditorView(editor);
    if (!currentView || !editor.isEditable || currentView.composing) return;
    let finish: (() => void) | undefined;
    try {
      finish = controller.beginAction();
      currentView.dispatch(
        editOrdinaryLink(editor.state, captureLinkEditTarget(editor.state), null),
      );
      editor.commands.focus(undefined, { scrollIntoView: false });
    } catch (cause) {
      onNotice(message(cause));
    } finally {
      finish?.();
    }
  }
  return (
    <>
      {visible &&
        link &&
        createPortal(
          <div
            ref={panel}
            className="ordinary-link-popover"
            role="group"
            aria-label={t('链接操作', 'Link actions')}
            style={{
              left: position?.left ?? 8,
              top: position?.top ?? 8,
              visibility: position ? 'visible' : 'hidden',
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault();
                event.stopPropagation();
                setDismissed(key);
                if (getMountedEditorView(editor))
                  editor.commands.focus(undefined, { scrollIntoView: false });
              }
            }}
          >
            <span className="ordinary-link-address" title={link.href}>
              {link.href || t('链接地址为空', 'Empty link URL')}
            </span>
            <div className="ordinary-link-actions">
              <button
                type="button"
                className="ordinary-link-open"
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => void open(link.href)}
              >
                <ExternalLink size={14} />
                {t('打开链接', 'Open link')}
              </button>
              <Tooltip label={t('编辑链接（Ctrl+K）', 'Edit link (Ctrl+K)')}>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t('编辑链接', 'Edit link')}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => {
                    const currentView = getMountedEditorView(editor);
                    if (currentView && !currentView.composing && editor.isEditable) onEdit();
                  }}
                >
                  <Pencil size={14} />
                </button>
              </Tooltip>
              <Tooltip label={t('复制地址', 'Copy address')}>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t('复制地址', 'Copy address')}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => void copy(link.href)}
                >
                  <Copy size={14} />
                </button>
              </Tooltip>
              <Tooltip label={t('移除链接', 'Remove link')}>
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t('移除链接', 'Remove link')}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={remove}
                >
                  <Link2Off size={14} />
                </button>
              </Tooltip>
            </div>
            {notice && (
              <span className="ordinary-link-notice" role="status">
                {notice}
              </span>
            )}
          </div>,
          document.body,
        )}
      {editing && (
        <LinkEditDialog editor={editor} controller={controller} pane={pane} onClose={onClose} />
      )}
    </>
  );
}
