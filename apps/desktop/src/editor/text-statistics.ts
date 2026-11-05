import type { DocumentNode } from '@pkms/contracts';

export type TextStatistics = {
  words: number;
  characters: number;
};

type Language = 'zh-CN' | 'en';

const segmenters = new Map<Language, Intl.Segmenter>();

function collectText(nodes: readonly DocumentNode[], output: string[]) {
  for (const node of nodes) {
    if (node.text) output.push(node.text);
    if (node.type === 'hardBreak') output.push('\n');
    if (node.content?.length) {
      collectText(node.content, output);
      // A content node is a block boundary. Text split by marks remains a text
      // node, so it is joined without a separator and does not invent words.
      output.push('\n');
    }
  }
}

function wordCount(text: string, language: Language) {
  let segmenter = segmenters.get(language);
  if (!segmenter) {
    segmenter = new Intl.Segmenter(language, { granularity: 'word' });
    segmenters.set(language, segmenter);
  }
  let words = 0;
  for (const segment of segmenter.segment(text)) {
    if (segment.isWordLike) words++;
  }
  return words;
}

/** Counts visible prose without allowing block boundaries to merge adjacent words. */
export function documentTextStatistics(
  content: readonly DocumentNode[],
  language: Language,
): TextStatistics {
  const text: string[] = [];
  collectText(content, text);
  const plain = text.join('');
  return {
    words: wordCount(plain, language),
    characters: Array.from(plain.replace(/\s/g, '')).length,
  };
}
