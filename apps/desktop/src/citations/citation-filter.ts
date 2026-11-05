/** Citation picker filtering: every whitespace-separated term must match key, title or formatted line. */
export type CitationEntryLike = { key: string; title: string; formatted: string };

export function filterCitations<T extends CitationEntryLike>(entries: T[], query: string): T[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return entries;
  return entries.filter((entry) => {
    const haystack = `${entry.key} ${entry.title} ${entry.formatted}`.toLocaleLowerCase();
    return terms.every((term) => haystack.includes(term));
  });
}

/** Stable human label for an entry when the formatted reference line is empty. */
export function citationLabel(entry: CitationEntryLike): string {
  return entry.formatted.trim() || entry.title.trim() || entry.key;
}
