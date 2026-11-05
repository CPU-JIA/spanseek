import { useI18n } from '../i18n';
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import {
  EditorContent,
  NodeViewWrapper,
  ReactNodeViewRenderer,
  useEditor,
  type NodeViewProps,
} from '@tiptap/react';
import { Node, mergeAttributes, type JSONContent } from '@tiptap/core';
import StarterKit from '@tiptap/starter-kit';
import UniqueID from '@tiptap/extension-unique-id';
import { Table, TableRow } from '@tiptap/extension-table';
import TaskList from '@tiptap/extension-task-list';
import { Selection } from '@tiptap/pm/state';
import type { BibliographyEntry, NoteDocument, WorkspaceIdentity } from '@pkms/contracts';
import { call, choose, imageUrl, message } from '../api';
import type { DraftController } from './draft-controller';
import { joinEditors, shareEditedDocument } from './shared-editors';
import { MIRROR } from './mirror-transactions';
import { needsDocumentSync } from './document-sync';
import { RichDocumentScope } from '../rich/DocumentContent';
import { RICH_BLOCK_TYPES, type RichEditRequest } from './rich-format-model';
import type { ObjectTypeOption } from '../rich/YamlProperties';
import { AlignedTableCell, AlignedTableHeader, richEditorExtensions } from './rich-extensions';
import { richMarkdownPaste } from './rich-markdown';
import { RichFormatting } from './rich-formatting';
import { BibliographyHeadingMetadata, RichOrderedList, RichTaskItem } from './rich-schema';
import './rich-editor.css';
import { noteReferenceExtension } from './reference-extension';
import { ReferencePicker } from './reference-picker';
import { CitationPicker } from '../citations/CitationPicker';
import { BibliographyDialog } from '../citations/BibliographyDialog';
import {
  bibliographyFingerprintFromJson,
  bibliographySection,
} from '../citations/bibliography-section';
import { referenceOccurrenceIDs } from './reference-occurrences';
import { referenceHighlight, showReferenceHighlight } from './reference-highlight-extension';
import { referenceAnchor, referenceNode, type ReferenceLocation } from './reference-model';
import { BlockHistoryDialog } from './BlockHistoryDialog';
import { reconcileSelection } from './reconcile-selection';
import { BlockContinuation } from './block-continuation';
import { headingFoldExtension } from './heading-fold';
import { headingFoldStorageKey } from './heading-fold-model';
import { OrdinaryLinkTools } from './ordinary-link-tools';
import { OrdinaryLinkPresentation } from './link-edit-target';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';

function PictureView({ node, extension, updateAttributes }: NodeViewProps) {
  const { t } = useI18n();
  const { controller, identity } = extension.options as {
    controller: DraftController;
    identity: WorkspaceIdentity;
  };
  const { record } = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const hash = node.attrs.blob_hash as string;
  const commandId = record.imageImports[hash];
  const [preview, setPreview] = useState<{ src: string; error: string | null }>({
    src: '',
    error: null,
  });
  // `title` is part of the persisted image schema (document.rs allows it), so the
  // editor has to be able to write it. `null` means "not editing".
  const [caption, setCaption] = useState<string | null>(null);
  const storedCaption = (node.attrs.title as string | null) ?? '';
  useEffect(() => {
    let live = true;
    if (!commandId) {
      setPreview({
        src: imageUrl(identity, { revision: record.base.revision_id, hash }),
        error: null,
      });
      return;
    }
    void call(
      { kind: 'attachment_resume', payload: { command_id: commandId } },
      'attachment',
      identity,
    )
      .then((result) => {
        if (live)
          setPreview({ src: imageUrl(identity, { token: result.preview_token }), error: null });
      })
      .catch((error) => {
        if (live) setPreview({ src: '', error: message(error) });
      });
    return () => {
      live = false;
    };
  }, [commandId, hash, identity, record.base.revision_id]);
  return (
    <NodeViewWrapper
      className="note-image"
      as="figure"
      data-id={node.attrs.id}
      data-block-id={node.attrs.block_id}
    >
      {preview.src && (
        <img
          src={preview.src}
          alt={node.attrs.alt || node.attrs.original_name || t('笔记图片', 'Note image')}
          onError={() =>
            setPreview((current) => ({
              ...current,
              error: t(
                '图片无法读取，请检查附件是否完整。',
                'Image could not be read. Check that the attachment is available.',
              ),
            }))
          }
        />
      )}
      {preview.error && <figcaption role="alert">{preview.error}</figcaption>}
      <figcaption className="note-image-caption">
        {caption !== null ? (
          <input
            className="note-image-caption-input"
            aria-label={t('图片说明', 'Image caption')}
            autoFocus
            maxLength={200}
            value={caption}
            onChange={(event) => setCaption(event.target.value)}
            onBlur={() => {
              // Escape clears the state before the input unmounts, so never assume a string.
              const value = (caption ?? '').trim();
              updateAttributes({ title: value === '' ? null : value });
              setCaption(null);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                event.currentTarget.blur();
              }
              if (event.key === 'Escape') {
                event.preventDefault();
                setCaption(null);
              }
            }}
          />
        ) : (
          <button type="button" className="text-button" onClick={() => setCaption(storedCaption)}>
            {storedCaption || t('添加图片说明', 'Add an image caption')}
          </button>
        )}
      </figcaption>
    </NodeViewWrapper>
  );
}
export function imageExtension(controller: DraftController, identity: WorkspaceIdentity) {
  return Node.create({
    name: 'image',
    group: 'block',
    atom: true,
    draggable: true,
    addOptions: () => ({ controller, identity }),
    addAttributes: () => ({
      blob_hash: {
        default: null,
        parseHTML: (element: HTMLElement) => element.getAttribute('blob_hash'),
      },
      original_name: {
        default: null,
        parseHTML: (element: HTMLElement) => element.getAttribute('original_name'),
      },
      alt: { default: null, parseHTML: (element: HTMLElement) => element.getAttribute('alt') },
      title: { default: null, parseHTML: (element: HTMLElement) => element.getAttribute('title') },
    }),
    parseHTML: () => [{ tag: 'figure[data-pkms-image]' }],
    renderHTML: ({ HTMLAttributes }) => [
      'figure',
      mergeAttributes(HTMLAttributes, { 'data-pkms-image': 'true' }),
    ],
    addNodeView: () => ReactNodeViewRenderer(PictureView),
  });
}

export function NoteEditor({
  controller,
  identity,
  pane,
  referenceLocation,
  objectTypeOptions,
}: {
  controller: DraftController;
  identity: WorkspaceIdentity;
  pane: string;
  referenceLocation?: ReferenceLocation;
  objectTypeOptions?: readonly ObjectTypeOption[];
}) {
  const { t } = useI18n();
  const bodyLabel = t('笔记正文', 'Note body');
  const snapshot = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [pickingReference, setPickingReference] = useState(false);
  const [blockHistoryId, setBlockHistoryId] = useState<string | null>(null);
  const [pickingCitation, setPickingCitation] = useState(false);
  const [showingBibliography, setShowingBibliography] = useState(false);
  const [pendingBibliography, setPendingBibliography] = useState<{
    entries: BibliographyEntry[];
    missing: string[];
    styleId: string;
    modified: boolean | null;
  } | null>(null);
  const [richRequest, setRichRequest] = useState<RichEditRequest | null>(null);
  const formatToolbar = useRef<HTMLDivElement>(null);
  // The document object this pane's editor already shows, so a self-produced edit
  // is never compared (or written) as if it came from outside.
  const appliedDocument = useRef<NoteDocument | null>(null);
  const [editingLink, setEditingLink] = useState(false);
  const extensions = useMemo(
    () => [
      StarterKit.configure({
        heading: { levels: [1, 2, 3, 4, 5, 6] },
        codeBlock: false,
        orderedList: false,
        trailingNode: false,
        link: {
          openOnClick: false,
          HTMLAttributes: { rel: 'noopener noreferrer', target: '_blank', class: null },
        },
      }),
      UniqueID.configure({
        attributeName: 'id',
        types: RICH_BLOCK_TYPES,
        generateID: () => crypto.randomUUID(),
        filterTransaction: (transaction) => !transaction.getMeta(MIRROR),
      }),
      BibliographyHeadingMetadata,
      RichOrderedList,
      TaskList,
      RichTaskItem.configure({ nested: true }),
      Table.configure({ resizable: false }),
      TableRow,
      AlignedTableCell,
      AlignedTableHeader,
      imageExtension(controller, identity),
      noteReferenceExtension(identity),
      referenceOccurrenceIDs.configure({ types: ['noteReference', 'wikiLink', 'wikiEmbed'] }),
      referenceHighlight,
      ...richEditorExtensions({
        controller,
        identity,
        pane,
        onEdit: setRichRequest,
        onError: setError,
        objectTypeOptions,
      }),
      richMarkdownPaste(setError, (source) => setRichRequest({ kind: 'markdown', source })),
      BlockContinuation,
      headingFoldExtension.configure({
        getStorageKey: () => {
          const note = controller.getSnapshot().record.base;
          return headingFoldStorageKey(identity.local_instance_id, identity.workspace_id, note.id);
        },
      }),
      OrdinaryLinkPresentation,
    ],
    [controller, identity, pane, objectTypeOptions],
  );
  const editor = useEditor(
    {
      extensions,
      enablePasteRules: false,
      content: snapshot.record.document as JSONContent,
      editorProps: {
        // The sticky format toolbar occupies part of the scroll viewport. Keep
        // the caret above it, including when controls wrap in a narrow pane.
        scrollThreshold: { top: 0, right: 0, bottom: 64, left: 0 },
        scrollMargin: { top: 5, right: 5, bottom: 64, left: 5 },
        attributes: {
          class: 'note-prose',
          'aria-label': bodyLabel,
          spellcheck: 'false',
        },
        handleDOMEvents: {
          compositionstart: () => {
            controller.setComposing(pane, true);
            return false;
          },
          compositionend: () => {
            setTimeout(() => controller.setComposing(pane, false), 0);
            return false;
          },
        },
      },
      onUpdate: ({ editor }) => {
        if (!editor.isEditable) return;
        try {
          const produced = editor.getJSON() as NoteDocument;
          controller.update(controller.getSnapshot().record.title, produced);
          appliedDocument.current = produced;
          // Every joined pane mirrored this edit, and mirroring suppresses their
          // own update event, so record the object they now hold as well.
          shareEditedDocument(controller, editor, produced);
          setError(null);
        } catch (error) {
          setError(message(error));
        }
      },
    },
    [controller, identity, pane, objectTypeOptions],
  );
  useLayoutEffect(() => {
    if (!editor) return;
    // useEditor keeps this instance while its document dependencies are unchanged.
    const editorProps = editor.options.editorProps;
    editor.setOptions({
      editorProps: {
        ...editorProps,
        attributes: { ...editorProps.attributes, 'aria-label': bodyLabel },
      },
    });
  }, [editor, bodyLabel]);
  useLayoutEffect(() => {
    const toolbar = formatToolbar.current;
    if (!editor || !toolbar) return;
    let previous = -1;
    const measure = () => {
      if (editor.isDestroyed) return;
      const clearance =
        getComputedStyle(toolbar).position === 'sticky'
          ? Math.ceil(toolbar.getBoundingClientRect().height) + 12
          : 12;
      if (clearance === previous) return;
      previous = clearance;
      editor.setOptions({
        editorProps: {
          ...editor.options.editorProps,
          scrollThreshold: { top: 0, right: 0, bottom: clearance, left: 0 },
          scrollMargin: { top: 5, right: 5, bottom: clearance, left: 5 },
        },
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(toolbar);
    window.addEventListener('resize', measure);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [editor]);
  useLayoutEffect(() => {
    if (!editor || editor.isDestroyed) return;
    return joinEditors(controller, editor, {
      fail: (error) => setError(message(error)),
      applied: (document) => {
        appliedDocument.current = document;
      },
    });
  }, [editor, controller]);
  useLayoutEffect(() => {
    if (!editor || editor.isDestroyed) return;
    let canceled = false;
    // New React NodeViews flush synchronously; create them after React's commit.
    queueMicrotask(() => {
      if (canceled || editor.isDestroyed || !editor.isEditable || editor.view.composing) return;
      // A newer generation or selection may have arrived since this effect ran.
      const { record } = controller.getSnapshot();
      if (
        record.identity.local_instance_id !== identity.local_instance_id ||
        record.identity.workspace_id !== identity.workspace_id
      )
        return;
      try {
        if (
          needsDocumentSync(
            record.document,
            appliedDocument.current,
            () => editor.getJSON() as NoteDocument,
          )
        ) {
          const selection = editor.state.selection;
          const applied = editor
            .chain()
            .setContent(record.document as JSONContent, { emitUpdate: false })
            .command(({ tr }) => {
              tr.setSelection(reconcileSelection(selection, tr.doc))
                .setMeta(MIRROR, true)
                .setMeta('addToHistory', false);
              return true;
            })
            .run();
          // setContent suppresses the update event, so record the applied object here.
          if (applied) appliedDocument.current = record.document;
        }
      } catch (failure) {
        // A stale editor must not overwrite the current draft after a failed sync.
        editor.setEditable(false, false);
        setError(message(failure));
      }
    });
    return () => {
      canceled = true;
    };
  }, [editor, controller, identity, snapshot.record.document]);
  useEffect(() => () => controller.setComposing(pane, false), [controller, pane]);
  useEffect(() => {
    if (!editor || !referenceLocation || editor.isDestroyed) return;
    if (referenceLocation.target.target_id !== controller.getSnapshot().record.base.id) return;
    if (!referenceLocation.target.target_block_id && !referenceLocation.occurrenceId) return;
    let clear: (() => void) | undefined;
    const frame = requestAnimationFrame(() => {
      if (editor.isDestroyed) return;
      const anchor = referenceAnchor(
        editor.view.dom,
        referenceLocation.target.target_block_id,
        referenceLocation.occurrenceId,
      );
      if (anchor)
        clear = showReferenceHighlight(editor.view, {
          requestId: referenceLocation.requestId,
          blockId: referenceLocation.target.target_block_id,
          occurrenceId: referenceLocation.occurrenceId,
        });
      if (clear) {
        // A collapsed callout opens while its selection is inside, without changing its saved folding preference.
        const target = referenceAnchor(
          editor.view.dom,
          referenceLocation.target.target_block_id,
          referenceLocation.occurrenceId,
        );
        if (target?.closest('.rich-editor-callout [hidden]')) {
          const at = editor.view.posAtDOM(target, 0);
          editor.view.dispatch(
            editor.state.tr
              .setSelection(Selection.near(editor.state.doc.resolve(at)))
              .setMeta('addToHistory', false),
          );
        }
        // Resolve again after the decoration transaction in case a NodeView rerendered.
        referenceAnchor(
          editor.view.dom,
          referenceLocation.target.target_block_id,
          referenceLocation.occurrenceId,
        )?.scrollIntoView({ block: 'center', behavior: 'instant' });
      } else
        setError(
          t(
            '目标段落或这处引用不在当前草稿中；未跳转到其他位置。',
            'The paragraph or reference occurrence is absent from this draft. No approximate location was selected.',
          ),
        );
    });
    return () => {
      cancelAnimationFrame(frame);
      clear?.();
    };
  }, [editor, controller, referenceLocation?.requestId]);
  async function insertImage() {
    setBusy(true);
    setError(null);
    let finish: (() => void) | undefined;
    try {
      finish = controller.beginAction();
      const grant = await choose('import_file', identity);
      if (!grant) {
        setNotice(t('已取消选择图片。', 'Image selection cancelled.'));
        return;
      }
      setNotice(null);
      const commandId = crypto.randomUUID();
      const imported = await call(
        { kind: 'attachment_import', payload: { command_id: commandId, token: grant.token } },
        'attachment',
        identity,
      );
      await controller.addImageImport(imported.blob.hash, commandId);
      editor
        ?.chain()
        .focus()
        .insertContent({
          type: 'image',
          attrs: {
            id: crypto.randomUUID(),
            blob_hash: imported.blob.hash,
            original_name: imported.original_name,
            alt: imported.original_name,
          },
        })
        .run();
    } catch (error) {
      setError(message(error));
    } finally {
      finish?.();
      setBusy(false);
    }
  }
  function applyBibliography(entries: BibliographyEntry[], missing: string[], styleId: string) {
    if (!editor || editor.isDestroyed || !editor.isEditable) return;
    const heading = t('参考文献', 'References');
    const finish = controller.beginAction();
    try {
      const content: JSONContent[] = [
        {
          type: 'heading',
          attrs: {
            level: 2,
            bibliography_style_id: styleId,
            bibliography_fingerprint: '',
          },
          content: [{ type: 'text', text: heading }],
        },
        {
          type: 'bulletList',
          content: entries.map((entry) => ({
            type: 'listItem',
            content: [{ type: 'paragraph', content: [{ type: 'text', text: entry.formatted }] }],
          })),
        },
        ...(missing.length
          ? [
              {
                type: 'paragraph',
                content: [
                  {
                    type: 'text',
                    text: `${t('缺少引用目标：', 'Missing targets: ')}${missing.join(', ')}`,
                  },
                ],
              },
            ]
          : []),
      ];
      const fingerprint = bibliographyFingerprintFromJson(content);
      content[0].attrs = {
        ...content[0].attrs,
        bibliography_fingerprint: fingerprint,
      };
      const section = bibliographySection(editor.state.doc);
      if (section)
        editor.chain().insertContentAt({ from: section.from, to: section.to }, content).run();
      else
        editor
          .chain()
          .focus('end')
          .insertContent([{ type: 'paragraph', content: [] }, ...content])
          .run();
    } finally {
      finish();
    }
  }
  function requestBibliography(entries: BibliographyEntry[], missing: string[], styleId: string) {
    if (!editor || editor.isDestroyed || !editor.isEditable) return;
    const section = bibliographySection(editor.state.doc);
    if (section) {
      setPendingBibliography({ entries, missing, styleId, modified: section.modified });
      return;
    }
    applyBibliography(entries, missing, styleId);
  }
  if (!editor) return <p className="muted">{t('正在准备编辑器…', 'Preparing editor…')}</p>;
  const currentBibliography = bibliographySection(editor.state.doc);
  return (
    <RichDocumentScope
      identity={identity}
      note={{
        ...snapshot.record.base,
        title: snapshot.record.title,
        document: snapshot.record.document,
      }}
      imageImports={snapshot.record.imageImports}
    >
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="muted" role="status">
          {notice}
        </p>
      )}
      {pickingReference && (
        <ReferencePicker
          identity={identity}
          onClose={() => {
            setPickingReference(false);
            editor.commands.focus();
          }}
          onSelect={(target, label, alias, embedded) => {
            if (editor.isDestroyed || !editor.isEditable)
              throw new Error(
                t('编辑器已关闭，请重新插入。', 'The editor closed. Insert the reference again.'),
              );
            const finish = controller.beginAction();
            try {
              if (
                !editor
                  .chain()
                  .focus()
                  .insertContent(referenceNode(target, label, alias, embedded) as JSONContent)
                  .run()
              )
                throw new Error(
                  t(
                    '当前位置不能插入引用或嵌入，请选择正文段落。',
                    'References and embeds can be inserted into text paragraphs.',
                  ),
                );
            } finally {
              finish();
            }
          }}
        />
      )}
      {pickingCitation && (
        <CitationPicker
          identity={identity}
          onClose={() => {
            setPickingCitation(false);
            editor.commands.focus();
          }}
          onSelect={(marker) => {
            if (editor.isDestroyed || !editor.isEditable)
              throw new Error(
                t(
                  '编辑器已关闭，请重新插入引用。',
                  'The editor closed. Insert the citation again.',
                ),
              );
            const finish = controller.beginAction();
            try {
              if (!editor.chain().focus().insertContent(marker).run())
                throw new Error(
                  t(
                    '当前位置不能插入引用或嵌入，请选择正文段落。',
                    'Citations can be inserted into text paragraphs.',
                  ),
                );
            } finally {
              finish();
            }
          }}
        />
      )}
      {showingBibliography && (
        <BibliographyDialog
          identity={identity}
          noteId={snapshot.record.base.id}
          initialStyleId={currentBibliography?.styleId ?? undefined}
          onInsert={(entries, missing, styleId) => requestBibliography(entries, missing, styleId)}
          onClose={() => {
            setShowingBibliography(false);
            editor.commands.focus();
          }}
        />
      )}
      {pendingBibliography && (
        <ActionConfirmDialog
          title={t('更新已有文献表？', 'Update the existing bibliography?')}
          description={
            pendingBibliography.modified
              ? t(
                  '检测到文献表区块有手工修改。继续会按所选 CSL 样式重新生成标题、条目和缺失提示，手工修改将丢失；正文其他内容保持不变。',
                  'Manual edits were detected in the bibliography section. Continuing regenerates its heading, entries and missing-target notice with the selected CSL style, so those edits will be lost; the rest of the note is unchanged.',
                )
              : t(
                  '当前正文已有文献表。继续会整体替换该区块的标题、条目和缺失提示；正文其他内容保持不变。',
                  'This note already has a bibliography. Continuing replaces that section heading, entries and missing-target notice; the rest of the note is unchanged.',
                )
          }
          confirmLabel={t('更新文献表', 'Update bibliography')}
          onCancel={() => {
            setPendingBibliography(null);
            editor.commands.focus();
          }}
          onConfirm={() => {
            const pending = pendingBibliography;
            setPendingBibliography(null);
            if (pending) {
              applyBibliography(pending.entries, pending.missing, pending.styleId);
              requestAnimationFrame(() => editor.commands.focus());
            }
          }}
        />
      )}
      {blockHistoryId && (
        <BlockHistoryDialog
          identity={identity}
          noteId={snapshot.record.base.id}
          blockId={blockHistoryId}
          onClose={() => {
            setBlockHistoryId(null);
            editor.commands.focus();
          }}
        />
      )}
      <EditorContent className="editor-body" editor={editor} />
      <RichFormatting
        editor={editor}
        controller={controller}
        pane={pane}
        toolbarRef={formatToolbar}
        busy={busy}
        request={richRequest}
        onRequest={setRichRequest}
        onNotice={setError}
        onLink={() => setEditingLink(true)}
        onImage={() => void insertImage()}
        onReference={() => setPickingReference(true)}
        onCitation={() => setPickingCitation(true)}
        onBibliography={() => setShowingBibliography(true)}
        onBlockHistory={setBlockHistoryId}
        objectTypeOptions={objectTypeOptions}
      />

      <OrdinaryLinkTools
        editor={editor}
        controller={controller}
        pane={pane}
        editing={editingLink}
        onEdit={() => setEditingLink(true)}
        onClose={() => setEditingLink(false)}
        onNotice={setError}
      />
    </RichDocumentScope>
  );
}
