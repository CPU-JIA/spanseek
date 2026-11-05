import { markdownDocument, type MarkdownOptions } from '../inference/markdown-document.ts';

self.onmessage = (event: MessageEvent<{ source: string; options: MarkdownOptions }>) => {
  try {
    self.postMessage({ value: markdownDocument(event.data.source, event.data.options) });
  } catch (cause) {
    self.postMessage({
      error: cause instanceof Error ? cause.message : '正文转换失败，输入已保留。',
    });
  }
};
