import { Extension, type Editor, type JSONContent } from '@tiptap/core';
import { Plugin, type Selection } from '@tiptap/pm/state';
import type { NoteDocument } from '@pkms/contracts';
import {
  looksLikeMarkdown,
  markdownDocument,
  type MarkdownDocument,
  type MarkdownOptions,
} from '../inference/markdown-document.ts';
import { richDocumentError } from './rich-format-model.ts';

function canInsertFrontmatter(editor: Editor, selection: Selection) {
  if (selection.from > 1) return false;
  const first = editor.state.doc.firstChild;
  return (
    first?.type.name !== 'frontmatter' || (selection.from === 0 && selection.to >= first.nodeSize)
  );
}

export function insertEditorMarkdown(
  editor: Editor,
  source: string,
  report: (notice: string | null) => void,
  selection?: Selection,
): boolean {
  const selected = selection ?? editor.state.selection;
  const parsed = markdownDocument(source, {
    contextDocument: editor.state.doc.toJSON() as NoteDocument,
    allowFrontmatter: canInsertFrontmatter(editor, selected),
  });
  return insertPreparedMarkdown(editor, parsed, report, selected);
}

export function insertPreparedMarkdown(
  editor: Editor,
  parsed: MarkdownDocument,
  report: (notice: string | null) => void,
  selected: Selection = editor.state.selection,
): boolean {
  const inserted = editor
    .chain()
    .command(({ tr }) => {
      tr.setSelection(selected);
      return true;
    })
    .insertContentAt(
      { from: selected.from, to: selected.to },
      parsed.document.content as JSONContent[],
      { errorOnInvalidContent: true },
    )
    .command(({ tr }) => {
      const error = richDocumentError(tr.doc);
      if (error) throw new Error(error);
      return true;
    })
    .run();
  if (!inserted) throw new Error('当前位置不能插入这些内容，请选择正文段落。');
  report(parsed.preserved.length ? '部分语法未转换，已保留为可见源码；请核对新插入的内容。' : null);
  return true;
}

/** Explicit/large inserts parse in a cancellable worker; the dialog owns staleness checks. */
export function prepareEditorMarkdown(
  editor: Editor,
  source: string,
  selection: Selection,
  signal: AbortSignal,
): Promise<MarkdownDocument> {
  const options: MarkdownOptions = {
    contextDocument: editor.state.doc.toJSON() as NoteDocument,
    allowFrontmatter: canInsertFrontmatter(editor, selection),
  };
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException('已取消。', 'AbortError'));
      return;
    }
    const worker = new Worker(new URL('./rich-markdown-worker.ts', import.meta.url), {
      type: 'module',
    });
    const dispose = () => {
      worker.terminate();
      signal.removeEventListener('abort', cancel);
    };
    const cancel = () => {
      dispose();
      reject(new DOMException('已取消。', 'AbortError'));
    };
    signal.addEventListener('abort', cancel, { once: true });
    worker.onmessage = (event: MessageEvent<{ value?: MarkdownDocument; error?: string }>) => {
      dispose();
      if (event.data.error || !event.data.value)
        reject(new Error(event.data.error || '正文转换失败，输入已保留。'));
      else resolve(event.data.value);
    };
    worker.onerror = () => {
      dispose();
      reject(new Error('正文转换失败，输入已保留，请重试。'));
    };
    worker.postMessage({ source, options });
  });
}

export function richMarkdownPaste(
  report: (notice: string | null) => void,
  prepareLargePaste: (source: string) => void,
) {
  return Extension.create({
    name: 'richMarkdownPaste',
    addProseMirrorPlugins() {
      let plainPasteAt = 0;
      return [
        new Plugin({
          props: {
            handleKeyDown(_view, event) {
              if (
                (event.ctrlKey || event.metaKey) &&
                event.shiftKey &&
                event.key.toLowerCase() === 'v'
              )
                plainPasteAt = Date.now();
              return false;
            },
            handlePaste: (view, event) => {
              if (
                !this.editor.isEditable ||
                view.composing ||
                view.state.selection.$from.parent.type.spec.code ||
                (plainPasteAt && Date.now() - plainPasteAt < 1000)
              ) {
                plainPasteAt = 0;
                return false;
              }
              const data = event.clipboardData;
              if (!data || data.files.length || data.getData('text/html')) return false;
              const text = data.getData('text/plain');
              if (!text) return false;
              // Classifying malformed markup may scan more than once. Hand off
              // large input before even running the syntax-recognition regexes.
              if (text.length > 16 * 1024) {
                prepareLargePaste(text);
                return true;
              }
              if (!looksLikeMarkdown(text)) return false;
              if (view.state.doc.content.size > 64 * 1024) {
                prepareLargePaste(text);
                return true;
              }
              try {
                return insertEditorMarkdown(this.editor, text, report);
              } catch (cause) {
                report(
                  (cause instanceof Error ? cause.message : 'Markdown 未能转换。') +
                    ' 已按原始文本粘贴。',
                );
                return false;
              }
            },
          },
        }),
      ];
    },
  });
}
