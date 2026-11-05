export const DEFAULT_HIGHLIGHT = '#FFF2A8';
/** Shared OFM emoji syntax; this module stays independent of React/the DOM. */
export const OFM_HIGHLIGHT_COLORS: Readonly<Record<string, string>> = {
  '🔴': '#FECACA',
  '🟠': '#FED7AA',
  '🟡': DEFAULT_HIGHLIGHT,
  '🟢': '#BBF7D0',
  '🔵': '#BFDBFE',
  '🟣': '#DDD6FE',
};
export const FORMAT_COLORS = [
  { label: '灰色', en: 'Gray', color: '#475569' },
  { label: '红色', en: 'Red', color: '#B91C1C' },
  { label: '橙色', en: 'Orange', color: '#C2410C' },
  { label: '黄色', en: 'Yellow', color: '#A16207' },
  { label: '绿色', en: 'Green', color: '#15803D' },
  { label: '青色', en: 'Teal', color: '#0E7490' },
  { label: '蓝色', en: 'Blue', color: '#2563EB' },
  { label: '紫色', en: 'Purple', color: '#7C3AED' },
] as const;
export const HIGHLIGHT_COLORS = [
  { label: '黄色', en: 'Yellow', color: DEFAULT_HIGHLIGHT },
  { label: '红色', en: 'Red', color: '#FECACA' },
  { label: '橙色', en: 'Orange', color: '#FED7AA' },
  { label: '绿色', en: 'Green', color: '#BBF7D0' },
  { label: '蓝色', en: 'Blue', color: '#BFDBFE' },
  { label: '紫色', en: 'Purple', color: '#DDD6FE' },
] as const;

/** Persisted colors are values, never arbitrary CSS declarations. */
export function safeColor(value: unknown): string | undefined {
  return typeof value === 'string' && /^#[\da-f]{6}$/i.test(value)
    ? value.toUpperCase()
    : undefined;
}

export function highlightInk(background: string): string {
  const color = safeColor(background) ?? DEFAULT_HIGHLIGHT;
  const channels = [1, 3, 5].map((offset) => {
    const value = parseInt(color.slice(offset, offset + 2), 16) / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722 > 0.179
    ? '#172033'
    : '#FFFFFF';
}
