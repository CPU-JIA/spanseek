import {
  appendMarkdownDocument,
  markdownDocument,
  plainTextDocument,
} from './markdown-document.ts';
import { prepareAnswerProposal } from './proposal-document.ts';
import { prepareTaskProposal } from './task-proposal-document.ts';
import type { MarkdownRequest } from './markdown-request.ts';

self.onmessage = (event: MessageEvent<MarkdownRequest>) => {
  try {
    const request = event.data;
    const value =
      request.kind === 'task-preview'
        ? prepareTaskProposal(
            { type: 'doc', content: [] },
            request.source,
            request.materials,
            'rich',
          )
        : request.kind === 'task-proposal'
          ? prepareTaskProposal(request.base, request.source, request.materials, request.format)
          : request.kind === 'parse'
            ? markdownDocument(request.source, { allowFrontmatter: request.allowFrontmatter })
            : request.kind === 'answer-proposal'
              ? prepareAnswerProposal(request.base, request.answer)
              : request.format === 'source'
                ? {
                    document: {
                      type: 'doc' as const,
                      content: [
                        ...structuredClone(request.base.content),
                        ...plainTextDocument(request.source).content,
                      ],
                    },
                    preserved: [],
                  }
                : appendMarkdownDocument(request.base, request.source, {
                    allowFrontmatter: request.allowFrontmatter,
                  });
    self.postMessage({ value });
  } catch (cause) {
    self.postMessage({
      error: cause instanceof Error ? cause.message : 'Markdown conversion failed.',
    });
  }
};
