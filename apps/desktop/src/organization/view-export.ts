import type { SavedView, ViewNote } from '@pkms/contracts';

export function viewCsv(view: SavedView, items: ViewNote[]): string {
  const customProperties = view.config?.custom_properties ?? [];
  const headers = [
    'view',
    'note_id',
    'title',
    'inbox_state',
    'tags',
    'archived',
    'search_enabled',
    'updated_at',
    'preview',
    ...customProperties.map((key) => `property:${key}`),
    ...(customProperties.length ? ['property_warnings'] : []),
  ];
  const rows = items.map((item) => {
    const warnings: string[] = [];
    if (item.properties_error) warnings.push('frontmatter=unreadable');
    if (item.properties_truncated) warnings.push('properties=truncated');
    const customValues = customProperties.map((key) => {
      const property = item.properties?.find((candidate) => candidate.key === key);
      if (!property) {
        warnings.push(`${key}=missing`);
        return '';
      }
      if (property.kind === 'nested') warnings.push(`${key}=nested`);
      if (property.truncated) warnings.push(`${key}=truncated`);
      return property.truncated ? `${property.value}…` : property.value;
    });
    return [
      view.name,
      item.note.id,
      item.note.title,
      item.note.inbox_state ?? 'pending',
      item.metadata.tags.map((tag) => tag.display_name).join('|'),
      String(item.metadata.archived),
      String(item.metadata.search_enabled),
      item.note.updated_at,
      item.note.preview,
      ...customValues,
      ...(customProperties.length ? [warnings.join('|')] : []),
    ];
  });
  return `\uFEFF${[headers, ...rows].map((row) => row.map(csvCell).join(',')).join('\r\n')}\r\n`;
}

function csvCell(value: string) {
  return `"${value.replaceAll('"', '""')}"`;
}

export function downloadText(filename: string, text: string, type = 'text/csv;charset=utf-8') {
  const blob = new Blob([text], { type });
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = href;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(href), 0);
}

export function safeFilename(name: string) {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '-').trim();
  return cleaned || 'spanseek-view';
}
