/**
 * Highlight only the query's word-like terms. The result is plain text parts,
 * never HTML, so titles and excerpts from the library stay inert when rendered.
 */
export type SearchHighlightPart = { text: string; match: boolean };

const MAX_QUERY_CHARS = 4096;
const MAX_TERMS = 32;
const WORD = /[\p{L}\p{N}_]+/gu;

function addTerm(terms: Map<string, string>, raw: string, locale?: string): void {
  const value = raw.trim();
  if (!value || value.length > 128) return;
  let key: string;
  try {
    key = value.toLocaleLowerCase(locale);
  } catch {
    key = value.toLowerCase();
  }
  if (!terms.has(key)) terms.set(key, value);
}

export function searchHighlightTerms(query: string, locale?: string): string[] {
  const source = query.slice(0, MAX_QUERY_CHARS).trim();
  if (!source) return [];
  const terms = new Map<string, string>();
  if (typeof Intl !== 'undefined' && typeof Intl.Segmenter === 'function') {
    try {
      const segmenter = new Intl.Segmenter(locale, { granularity: 'word' });
      for (const part of segmenter.segment(source)) {
        if (part.isWordLike) addTerm(terms, part.segment, locale);
      }
    } catch {
      // Older runtimes fall back to the Unicode word matcher below.
    }
  }
  for (const part of source.match(WORD) ?? []) addTerm(terms, part, locale);
  // A short exact phrase is useful when the user quotes a name or a whole
  // expression; keep it in addition to the individual words.
  if (source.length >= 2 && source.length <= 64 && /\p{L}|\p{N}/u.test(source)) {
    addTerm(terms, source, locale);
  }
  return [...terms.values()]
    .sort((left, right) => right.length - left.length || left.localeCompare(right))
    .slice(0, MAX_TERMS);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function termPattern(term: string): string {
  const escaped = escapeRegExp(term);
  // Latin terms should match whole words, not fragments of another word
  // ("art" inside "earth"). CJK terms intentionally remain substring matches.
  return /[A-Za-z]/.test(term) ? `(?<![A-Za-z0-9_])${escaped}(?![A-Za-z0-9_])` : escaped;
}

export function searchHighlightParts(
  text: string,
  query: string,
  locale?: string,
): SearchHighlightPart[] {
  if (!text) return [];
  const terms = searchHighlightTerms(query, locale);
  if (!terms.length) return [{ text, match: false }];
  let expression: RegExp;
  try {
    expression = new RegExp(terms.map(termPattern).join('|'), 'giu');
  } catch {
    return [{ text, match: false }];
  }
  const parts: SearchHighlightPart[] = [];
  let cursor = 0;
  for (const match of text.matchAll(expression)) {
    const index = match.index ?? 0;
    if (index > cursor) parts.push({ text: text.slice(cursor, index), match: false });
    const value = match[0];
    if (!value) continue;
    const previous = parts.at(-1);
    if (previous?.match) previous.text += value;
    else parts.push({ text: value, match: true });
    cursor = index + value.length;
  }
  if (cursor < text.length) parts.push({ text: text.slice(cursor), match: false });
  return parts.length ? parts : [{ text, match: false }];
}
