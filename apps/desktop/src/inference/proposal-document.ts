import type { AnswerRecord, DocumentNode, NoteDocument } from '@pkms/contracts';
import { markdownDocument, type MarkdownDocument } from './markdown-document.ts';

/** Append without changing existing node IDs, rich structure, or attachment references. */
export function appendAnswerProposal(
  base: NoteDocument,
  answer: AnswerRecord,
  uuid = () => crypto.randomUUID(),
): NoteDocument {
  return prepareAnswerProposal(base, answer, uuid).document;
}

/** The answer and free-chat proposal paths use the same D3 Markdown semantics. */
export function prepareAnswerProposal(
  base: NoteDocument,
  answer: AnswerRecord,
  uuid = () => crypto.randomUUID(),
): MarkdownDocument {
  const document = structuredClone(base);
  const paragraph = (text: string): DocumentNode => ({
    type: 'paragraph',
    attrs: { id: uuid() },
    content: text ? [{ type: 'text', text }] : [],
  });
  document.content.push(paragraph(`AI · ${answer.request.search.query}`));
  const converted = markdownDocument(answer.text, {
    uuid,
    allowFrontmatter: false,
    contextDocument: document,
  });
  document.content.push(...converted.document.content);
  for (const item of answer.evidence) {
    const target = item.evidence.target;
    const annotation = item.evidence.annotation;
    const reference: DocumentNode = {
      type: 'noteReference' as const,
      attrs: {
        target_id: annotation?.annotation_id ?? target.object_id,
        target_revision_id: target.content_revision,
        ...(annotation ? {} : { target_block_id: target.block_id }),
        occurrence_id: uuid(),
        label: `[${item.citation_id}] ${item.evidence.title}`,
      },
    };
    document.content.push({ type: 'paragraph', attrs: { id: uuid() }, content: [reference] });
  }
  return { document, preserved: converted.preserved };
}
