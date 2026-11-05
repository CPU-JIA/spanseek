import { Node, mergeAttributes } from '@tiptap/core';
import { NodeViewWrapper, ReactNodeViewRenderer, type NodeViewProps } from '@tiptap/react';
import type { WorkspaceIdentity } from '@pkms/contracts';
import { ReferenceChip } from './reference-chip';

function ReferenceView({ node, extension, editor, getPos }: NodeViewProps) {
  return (
    <NodeViewWrapper
      as="span"
      contentEditable={false}
      data-reference-occurrence={node.attrs.occurrence_id}
    >
      <ReferenceChip
        attrs={node.attrs}
        identity={extension.options.identity as WorkspaceIdentity}
        interaction={editor.isEditable ? 'edit' : 'read'}
        onSelect={() => {
          const position = getPos();
          if (typeof position === 'number' && !editor.isDestroyed)
            editor.chain().focus().setNodeSelection(position).run();
        }}
      />
    </NodeViewWrapper>
  );
}
export function noteReferenceExtension(identity: WorkspaceIdentity) {
  return Node.create({
    name: 'noteReference',
    group: 'inline',
    inline: true,
    atom: true,
    addOptions: () => ({ identity }),
    addAttributes: () => ({
      target_id: { default: null },
      target_block_id: { default: null },
      target_revision_id: { default: null },
      occurrence_id: { default: null, parseHTML: () => crypto.randomUUID() },
      label: { default: null, parseHTML: (element) => element.textContent },
    }),
    parseHTML: () => [
      {
        tag: 'span[data-note-reference]',
        getAttrs: (element) => ({
          target_id: element.getAttribute('target_id'),
          target_block_id: element.getAttribute('target_block_id'),
          target_revision_id: element.getAttribute('target_revision_id'),
          label: element.textContent,
          occurrence_id: crypto.randomUUID(),
        }),
      },
    ],
    renderHTML: ({ node, HTMLAttributes }) => [
      'span',
      mergeAttributes(HTMLAttributes, { 'data-note-reference': 'true', class: 'note-reference' }),
      node.attrs.label || '↗',
    ],
    addNodeView: () => ReactNodeViewRenderer(ReferenceView),
  });
}
