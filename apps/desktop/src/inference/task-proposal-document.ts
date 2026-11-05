import type { DocumentNode, NoteDocument, SearchEvidence } from '@pkms/contracts';
import { markdownDocument, plainTextDocument, type MarkdownDocument } from './markdown-document.ts';

/** Bind only explicit numbered citations in new prose; existing notes and code stay unchanged. */
export function prepareTaskProposal(
  base: NoteDocument,
  source: string,
  materials: readonly SearchEvidence[],
  format: 'rich' | 'source',
  uuid = () => crypto.randomUUID(),
): MarkdownDocument {
  const parsed = markdownDocument(source, { allowFrontmatter: false, contextDocument: base, uuid });
  const used = new Set<number>();
  const unresolved = new Set<number>();
  function reference(index: number, marks?: DocumentNode['marks']): DocumentNode {
    const material = materials[index - 1];
    const target = material.annotation?.annotation_id ?? material.target.object_id;
    return {
      type: 'wikiLink',
      attrs: {
        wiki_target: target,
        target_id: target,
        target_revision_id: material.target.content_revision,
        ...(material.annotation ? {} : { target_block_id: material.target.block_id }),
        occurrence_id: uuid(),
        label: `[${index}]`,
      },
      ...(marks ? { marks } : {}),
    };
  }
  function nodes(input: DocumentNode[]): DocumentNode[] {
    return input.flatMap((node) => {
      if (
        ['codeBlock', 'inlineMath', 'mathBlock', 'comment', 'commentBlock'].includes(node.type) ||
        node.marks?.some((mark) => mark.type === 'code' || mark.type === 'link')
      )
        return [node];
      if (node.type !== 'text')
        return [{ ...node, ...(node.content ? { content: nodes(node.content) } : {}) }];
      const text = node.text ?? '';
      const output: DocumentNode[] = [];
      let offset = 0;
      for (const match of text.matchAll(/\[([1-9]\d{0,3})\]/g)) {
        const index = Number(match[1]);
        if (index > materials.length) {
          unresolved.add(index);
          continue;
        }
        if (match.index! > offset) output.push({ ...node, text: text.slice(offset, match.index) });
        used.add(index);
        output.push(reference(index, node.marks));
        offset = match.index! + match[0].length;
      }
      if (offset === 0) return [node];
      if (offset < text.length) output.push({ ...node, text: text.slice(offset) });
      return output;
    });
  }
  const bound = nodes(parsed.document.content);
  const additions =
    format === 'rich'
      ? bound
      : [
          ...plainTextDocument(source).content,
          ...[...used].map(
            (index) =>
              ({
                type: 'paragraph',
                attrs: { id: uuid() },
                content: [
                  reference(index),
                  { type: 'text', text: ` ${materials[index - 1].title}` },
                ],
              }) as DocumentNode,
          ),
        ];
  return {
    document: { type: 'doc', content: [...structuredClone(base.content), ...additions] },
    preserved: parsed.preserved,
    unresolvedCitations: [...unresolved],
  };
}
