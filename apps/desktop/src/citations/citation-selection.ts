/** Pure citation-marker construction shared by the picker UI and its tests. */

export type CitationLocatorKind =
  | 'none'
  | 'page'
  | 'chapter'
  | 'section'
  | 'paragraph'
  | 'volume'
  | 'issue'
  | 'figure'
  | 'table'
  | 'note'
  | 'line'
  | 'verse'
  | 'book'
  | 'column'
  | 'appendix'
  | 'supplement'
  | 'equation'
  | 'timestamp'
  | 'opus'
  | 'folio'
  | 'scene'
  | 'canon'
  | 'part'
  | 'article'
  | 'sub-verbo'
  | 'title'
  | 'custom';

export type CitationSelection = {
  key: string;
  locatorKind: CitationLocatorKind;
  locatorValue: string;
};

export type CitationLocatorOption = {
  value: CitationLocatorKind;
  zh: string;
  en: string;
  /** Canonical CSL locator prefix understood by the kernel parser. */
  prefix?: string;
};

export const CITATION_LOCATOR_OPTIONS: readonly CitationLocatorOption[] = [
  { value: 'none', zh: '无定位符', en: 'No locator' },
  { value: 'page', zh: '页码', en: 'Page', prefix: 'p.' },
  { value: 'chapter', zh: '章', en: 'Chapter', prefix: 'ch.' },
  { value: 'section', zh: '节', en: 'Section', prefix: 'sec.' },
  { value: 'paragraph', zh: '段', en: 'Paragraph', prefix: 'para.' },
  { value: 'volume', zh: '卷', en: 'Volume', prefix: 'vol.' },
  { value: 'issue', zh: '期', en: 'Issue', prefix: 'issue' },
  { value: 'figure', zh: '图', en: 'Figure', prefix: 'fig.' },
  { value: 'table', zh: '表', en: 'Table', prefix: 'tab.' },
  { value: 'note', zh: '注', en: 'Note', prefix: 'n.' },
  { value: 'line', zh: '行', en: 'Line', prefix: 'l.' },
  { value: 'verse', zh: '诗节', en: 'Verse', prefix: 'v.' },
  { value: 'book', zh: '书卷', en: 'Book', prefix: 'bk.' },
  { value: 'column', zh: '栏', en: 'Column', prefix: 'col.' },
  { value: 'appendix', zh: '附录', en: 'Appendix', prefix: 'app.' },
  { value: 'supplement', zh: '增补', en: 'Supplement', prefix: 'supp.' },
  { value: 'equation', zh: '公式', en: 'Equation', prefix: 'eq.' },
  { value: 'timestamp', zh: '时间戳', en: 'Timestamp', prefix: 'ts.' },
  { value: 'opus', zh: '作品号', en: 'Opus', prefix: 'op.' },
  { value: 'folio', zh: '对开页', en: 'Folio', prefix: 'fol.' },
  { value: 'scene', zh: '场景', en: 'Scene', prefix: 'sc.' },
  { value: 'canon', zh: '正典', en: 'Canon', prefix: 'can.' },
  { value: 'part', zh: '部分', en: 'Part', prefix: 'pt.' },
  { value: 'article', zh: '条款', en: 'Article', prefix: 'art.' },
  { value: 'sub-verbo', zh: '词条', en: 'Sub verbo', prefix: 's.v.' },
  { value: 'title', zh: '标题', en: 'Title', prefix: 'title' },
  { value: 'custom', zh: '自定义定位符', en: 'Custom locator' },
];

export function locatorOption(kind: CitationLocatorKind): CitationLocatorOption {
  return (
    CITATION_LOCATOR_OPTIONS.find((option) => option.value === kind) ?? CITATION_LOCATOR_OPTIONS[0]
  );
}

/**
 * Converts the picker's typed locator into the canonical text consumed by
 * `citation.rs`. Page ranges/lists use `pp.`; all other page values use `p.`.
 * Custom locators remain verbatim so Chinese forms such as `图 3` survive.
 */
export function formatCitationLocator(kind: CitationLocatorKind, value: string): string | null {
  const trimmed = value.trim();
  if (kind === 'none' || !trimmed) return null;
  if (kind === 'custom') return trimmed;
  const option = locatorOption(kind);
  const prefix = kind === 'page' && /[,\-–—&]/.test(trimmed) ? 'pp.' : option.prefix;
  return prefix ? `${prefix} ${trimmed}` : trimmed;
}

/** Builds one citation cluster such as `[@a, pp. 3-4; @b, ch. 2]`. */
export function citationMarker(selections: readonly CitationSelection[]): string {
  const seen = new Set<string>();
  const specs: string[] = [];
  for (const selection of selections) {
    const key = selection.key.trim();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    const locator = formatCitationLocator(selection.locatorKind, selection.locatorValue);
    specs.push(locator ? `@${key}, ${locator}` : `@${key}`);
  }
  return specs.length ? `[${specs.join('; ')}]` : '';
}
