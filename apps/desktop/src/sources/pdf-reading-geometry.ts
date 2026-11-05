import type { PdfReadingView, PdfZoomMode, SourceLocator } from '@pkms/contracts';
import type { Rect } from './locators';

export const PDF_MIN_SCALE = 0.25;
export const PDF_MAX_SCALE = 5;
export const PDF_PAGE_GAP = 28;
export const PDF_PADDING = 24;
export const PDF_MAX_RENDERED_PAGES = 12;
/** Pages kept mounted and rasterised beyond the viewport, per direction. */
export const PDF_RENDER_OVERSCAN = 2;

export interface PdfPageGeometry {
  crop: Rect;
  rotation: number;
  userUnit: number;
}
export interface PdfPageLayout {
  index: number;
  top: number;
  left: number;
  width: number;
  height: number;
  geometry: PdfPageGeometry;
}
export interface PdfLayout {
  pages: PdfPageLayout[];
  width: number;
  height: number;
}
export interface PdfReadingPosition {
  locator: SourceLocator;
  view: PdfReadingView;
}

export const clamp = (value: number, minimum: number, maximum: number) =>
  Math.min(maximum, Math.max(minimum, value));
export const pdfRotation = (value: number) => ((value % 360) + 360) % 360;
export const pdfScale = (value: number) =>
  Math.round(clamp(value, PDF_MIN_SCALE, PDF_MAX_SCALE) * 1000) / 1000;

export function pageSize(geometry: PdfPageGeometry, scale: number, rotation: number) {
  const width = (geometry.crop[2] - geometry.crop[0]) * geometry.userUnit * scale;
  const height = (geometry.crop[3] - geometry.crop[1]) * geometry.userUnit * scale;
  return pdfRotation(geometry.rotation + rotation) % 180 === 0
    ? { width, height }
    : { width: height, height: width };
}

/** Geometry is independent of canvas lifetime, so offscreen pages retain their space. */
export function pdfLayout(
  count: number,
  geometries: ReadonlyMap<number, PdfPageGeometry>,
  fallback: PdfPageGeometry,
  scale: number,
  rotation: number,
  viewportWidth: number,
): PdfLayout {
  let top = PDF_PADDING;
  let width = Math.max(viewportWidth, 1);
  const pages: PdfPageLayout[] = [];
  for (let index = 0; index < count; index++) {
    const geometry = geometries.get(index) ?? fallback;
    const size = pageSize(geometry, scale, rotation);
    pages.push({ index, top, left: 0, ...size, geometry });
    top += size.height + PDF_PAGE_GAP;
    width = Math.max(width, size.width + PDF_PADDING * 2);
  }
  for (const page of pages) page.left = (width - page.width) / 2;
  return { pages, width, height: top + PDF_PADDING - (count ? PDF_PAGE_GAP : 0) };
}

export function pageAtOffset(pages: readonly PdfPageLayout[], offset: number) {
  let low = 0;
  let high = pages.length - 1;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (pages[middle].top + pages[middle].height + PDF_PAGE_GAP / 2 < offset) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** Window canvas/text layers, rather than letting a long PDF accumulate backing pixels. */
export function visiblePages(layout: PdfLayout, top: number, height: number) {
  if (!layout.pages.length) return [];
  const start = Math.max(0, pageAtOffset(layout.pages, top) - PDF_RENDER_OVERSCAN);
  const end = Math.min(
    layout.pages.length - 1,
    pageAtOffset(layout.pages, top + height) + PDF_RENDER_OVERSCAN,
  );
  return layout.pages.slice(start, Math.min(end + 1, start + PDF_MAX_RENDERED_PAGES));
}

/** Anchor coordinates refer to the unrotated crop box (PDF y points upwards). */
export function viewportToAnchor(x: number, y: number, rotation: number): [number, number] {
  const nx = clamp(x, 0, 1);
  const ny = clamp(y, 0, 1);
  switch (pdfRotation(rotation)) {
    case 90:
      return [ny, nx];
    case 180:
      return [1 - nx, ny];
    case 270:
      return [1 - ny, 1 - nx];
    default:
      return [nx, 1 - ny];
  }
}

export function anchorToViewport(x: number, y: number, rotation: number): [number, number] {
  switch (pdfRotation(rotation)) {
    case 90:
      return [y, x];
    case 180:
      return [1 - x, y];
    case 270:
      return [1 - y, 1 - x];
    default:
      return [x, 1 - y];
  }
}

export function readingPosition(
  layout: PdfLayout,
  versionId: string,
  scale: number,
  rotation: number,
  scrollTop: number,
  scrollLeft: number,
  viewportWidth: number,
  labels: readonly string[] | null,
  zoomMode: PdfZoomMode = 'width',
): PdfReadingPosition | null {
  if (!layout.pages.length) return null;
  const index = pageAtOffset(layout.pages, scrollTop + PDF_PADDING);
  const page = layout.pages[index];
  const [x, y] = viewportToAnchor(
    (scrollLeft + viewportWidth / 2 - page.left) / page.width,
    (scrollTop + PDF_PADDING - page.top) / page.height,
    page.geometry.rotation + rotation,
  );
  return {
    locator: {
      kind: 'pdf',
      source_version_id: versionId,
      page_index: index,
      page_label: labels?.[index] ?? null,
      crop_box: page.geometry.crop,
      rotation: page.geometry.rotation,
      coordinate_space: 'pdf_user_space',
      rects: [],
      quote: `Page ${labels?.[index] ?? index + 1}`,
      prefix: '',
      suffix: '',
    },
    view: {
      page_index: index,
      anchor_x: Math.round(x * 1e6) / 1e6,
      anchor_y: Math.round(y * 1e6) / 1e6,
      scale,
      rotation,
      zoom_mode: zoomMode,
    },
  };
}

export function viewScrollPosition(layout: PdfLayout, view: PdfReadingView, viewportWidth: number) {
  const page = layout.pages[view.page_index];
  if (!page) return null;
  const [x, y] = anchorToViewport(
    view.anchor_x,
    view.anchor_y,
    page.geometry.rotation + view.rotation,
  );
  return {
    top: Math.max(0, page.top + y * page.height - PDF_PADDING),
    left: Math.max(0, page.left + x * page.width - viewportWidth / 2),
  };
}
