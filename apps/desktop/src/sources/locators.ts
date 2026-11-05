import type { SourceLocator } from '@pkms/contracts';

/** DOM offsets are UTF-16; persisted source offsets count Unicode code points. */
export function textLocator(
  version: string,
  blockId: string,
  text: string,
  start: number,
  end: number,
): SourceLocator {
  if (
    !Number.isInteger(start) ||
    !Number.isInteger(end) ||
    start < 0 ||
    end <= start ||
    end > text.length
  )
    throw new Error('Invalid text selection');
  for (const offset of [start, end]) {
    const code = text.charCodeAt(offset);
    if (code >= 0xdc00 && code <= 0xdfff) throw new Error('Selection splits a Unicode character');
  }
  return {
    kind: 'text',
    source_version_id: version,
    block_id: blockId,
    start_codepoint: [...text.slice(0, start)].length,
    end_codepoint: [...text.slice(0, end)].length,
    quote: text.slice(start, end),
    prefix: [...text.slice(0, start)].slice(-32).join(''),
    suffix: [...text.slice(end)].slice(0, 32).join(''),
  };
}

/** Intersect DOM ranges with text blocks: triple-click may end at the next block's start. */
export function selectedTextBlock(
  root: HTMLElement,
  range: Range,
): { block: HTMLElement; start: number; end: number } | 'multiple' | null {
  if (!root.contains(range.startContainer)) return null;
  if (!root.contains(range.endContainer)) return 'multiple';
  let selection: { block: HTMLElement; start: number; end: number } | null = null;
  for (const block of root.querySelectorAll<HTMLElement>('[data-block-id]')) {
    const clipped = root.ownerDocument.createRange();
    clipped.selectNodeContents(block);
    const startPosition = clipped.comparePoint(range.startContainer, range.startOffset);
    const endPosition = clipped.comparePoint(range.endContainer, range.endOffset);
    if (startPosition > 0 || endPosition < 0) continue;
    if (startPosition === 0) clipped.setStart(range.startContainer, range.startOffset);
    if (endPosition === 0) clipped.setEnd(range.endContainer, range.endOffset);
    const length = clipped.toString().length;
    if (!length) continue;
    if (selection) return 'multiple';
    const before = root.ownerDocument.createRange();
    before.selectNodeContents(block);
    before.setEnd(clipped.startContainer, clipped.startOffset);
    const start = before.toString().length;
    selection = { block, start, end: start + length };
  }
  return selection;
}

export type Rect = [number, number, number, number];
/** Convert all four corners with PDF.js's inverse viewport (includes crop origin/rotation). */
export function pdfRect(rect: Rect, crop: Rect, convert: (x: number, y: number) => number[]): Rect {
  const points = [
    [rect[0], rect[1]],
    [rect[2], rect[1]],
    [rect[0], rect[3]],
    [rect[2], rect[3]],
  ].map(([x, y]) => convert(x, y));
  const result: Rect = [
    Math.max(crop[0], Math.min(...points.map((p) => p[0]))),
    Math.max(crop[1], Math.min(...points.map((p) => p[1]))),
    Math.min(crop[2], Math.max(...points.map((p) => p[0]))),
    Math.min(crop[3], Math.max(...points.map((p) => p[1]))),
  ];
  if (result.some((v) => !Number.isFinite(v)) || result[2] <= result[0] || result[3] <= result[1])
    throw new Error('Selection is outside the PDF crop box');
  return result;
}
