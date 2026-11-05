import type { Annotation } from '@pkms/contracts';

function folded(value: string): string {
  return value.normalize('NFKC').toLocaleLowerCase().replace(/\s+/g, ' ').trim();
}

/** Filters only the annotation records already loaded from the paged query. */
export function filterAnnotations(annotations: Annotation[], query: string): Annotation[] {
  const needle = folded(query);
  if (!needle) return annotations;
  return annotations.filter((annotation) => {
    const locator = annotation.locator as {
      quote?: string;
      page_label?: string | null;
      page_index?: number | null;
    };
    const page = [
      locator.page_label ?? '',
      typeof locator.page_index === 'number' ? String(locator.page_index + 1) : '',
    ].join(' ');
    return folded(`${locator.quote ?? ''}\n${annotation.comment}\n${page}`).includes(needle);
  });
}
