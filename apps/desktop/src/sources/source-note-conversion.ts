import type { SourceFormat } from '@pkms/contracts';
import {
  markdownDocument,
  plainTextDocument,
  type MarkdownDocument,
} from '../inference/markdown-document.ts';

export function convertSourceNote(text: string, format: SourceFormat): MarkdownDocument {
  if (format !== 'text' && format !== 'markdown')
    throw new Error('Only UTF-8 TXT and Markdown can become editable notes.');
  // The saved original stays byte-for-byte intact; the editor uses logical line breaks.
  const source = text.replace(/^\uFEFF/, '').replace(/\r\n?/g, '\n');
  return format === 'markdown'
    ? markdownDocument(source)
    : { document: plainTextDocument(source), preserved: [] };
}
