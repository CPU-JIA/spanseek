import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from 'react';
import {
  ChevronLeft,
  ChevronRight,
  Highlighter,
  MessageSquarePlus,
  MoreHorizontal,
  RotateCw,
  Search,
  SquareDashed,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import {
  GlobalWorkerOptions,
  TextLayer,
  type PDFDocumentProxy,
  type PDFPageProxy,
} from 'pdfjs-dist';
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url';
import pdfViewerStyles from 'pdfjs-dist/web/pdf_viewer.css?inline';
import type { Annotation, PdfReadingView, SourceLocator, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { Select } from '../ui/Select';
import { PopupMenu } from '../ui/PopupMenu';
import { message } from '../api';
import { pdfRect, type Rect } from './locators';
import { pdfErrorExcerpt, pdfReaderErrorMessage } from './pdf-reader-errors';
import { openPdfResource, type PdfReaderInput } from './pdf-document-resource';
import {
  anchorToViewport,
  clamp,
  pageAtOffset,
  pageSize,
  pdfLayout,
  pdfRotation,
  pdfScale,
  readingPosition,
  viewScrollPosition,
  viewportToAnchor,
  visiblePages,
  PDF_MAX_SCALE,
  PDF_MIN_SCALE,
  PDF_PADDING,
  type PdfLayout,
  type PdfPageGeometry,
  type PdfPageLayout,
  type PdfReadingPosition,
} from './pdf-reading-geometry';
import './pdf-reader.css';

export type { PdfReaderInput } from './pdf-document-resource';
export type { PdfReadingPosition } from './pdf-reading-geometry';
GlobalWorkerOptions.workerSrc = workerUrl;
// PDF.js includes generic application selectors. Its text-layer CSS stays in this reader.
const scopedPdfViewerStyles = `@scope (.source-pdf) { ${pdfViewerStyles} }`;
const noAnnotations: Annotation[] = [];
type PdfLocator = Extract<SourceLocator, { kind: 'pdf' }>;
type Zoom = { scale: number; rotation: number; mode: 'custom' | 'width' | 'page' };
type ViewAnchor = { index: number; x: number; y: number; screenX: number; screenY: number };
type Jump = { id: number; index: number; locator?: PdfLocator; view?: PdfReadingView | null };
type RenderedPage = {
  page: PDFPageProxy;
  viewport: ReturnType<PDFPageProxy['getViewport']>;
  layer: HTMLDivElement;
  label: string | null;
};

export interface PdfReaderProps {
  identity?: WorkspaceIdentity;
  versionId: string;
  /** A scoped native URL or caller-owned bytes; no workspace is required for temporary reading. */
  input?: PdfReaderInput;
  location?: SourceLocator | null;
  initialView?: PdfReadingView | null;
  onPosition?: (locator: SourceLocator, view: PdfReadingView) => void;
  positionReader?: RefObject<(() => PdfReadingPosition | null) | null>;
  onSelection?: (locator: SourceLocator | null, all?: SourceLocator[]) => void;
  onSelectionAction?: (
    locators: SourceLocator[],
    mode: 'highlight' | 'comment',
  ) => Promise<boolean> | boolean;
  selectionActive?: boolean;
  annotationBusy?: boolean;
  annotations?: Annotation[];
  selectedAnnotationId?: string | null;
  onAnnotationClick?: (annotation: Annotation) => void;
  onRequireLibrary?: () => void;
  /** Mode switch or other reading controls that belong in the single PDF toolbar. */
  startControls?: ReactNode;
}

/** Both saved sources and temporary files use this exact reading surface. */
export function PdfReader(props: PdfReaderProps) {
  const identity = props.identity;
  const url = props.input && 'url' in props.input ? props.input.url : '';
  return (
    <PdfReaderScope
      key={`${identity?.local_instance_id}/${identity?.workspace_id}/${props.versionId}/${url}`}
      {...props}
    />
  );
}

function PdfReaderScope({
  identity,
  versionId,
  input,
  location = null,
  initialView = null,
  onPosition,
  positionReader,
  onSelection,
  onSelectionAction,
  selectionActive,
  annotationBusy = false,
  annotations = noAnnotations,
  selectedAnnotationId,
  onAnnotationClick,
  onRequireLibrary,
  startControls,
}: PdfReaderProps) {
  const { t } = useI18n();
  const validLocation =
    location?.kind === 'pdf' && location.source_version_id === versionId ? location : null;
  const initialPage = validLocation?.page_index ?? initialView?.page_index ?? 0;
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [geometries, setGeometries] = useState<Map<number, PdfPageGeometry>>(() => new Map());
  const [labels, setLabels] = useState<string[] | null>(null);
  const [zoom, setZoom] = useState<Zoom>(() => ({
    scale: pdfScale(initialView?.scale ?? 1),
    rotation: initialView?.rotation ?? 0,
    mode: initialView?.zoom_mode ?? 'width',
  }));
  const [page, setPage] = useState(initialPage + 1);
  const [pageInput, setPageInput] = useState(String(initialPage + 1));
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [scrollPosition, setScrollPosition] = useState({ top: 0, left: 0 });
  const [jump, setJump] = useState<Jump>(() => ({
    id: 0,
    index: initialPage,
    locator: validLocation ?? undefined,
    view: initialView,
  }));
  const appliedJump = useRef(-1);
  // The page indicator follows an explicit jump until the reader scrolls
  // again, so a highlight near a page top still reports the page it is on.
  const pinnedPage = useRef<number | null>(null);
  const jumpNumber = useRef(0);
  const appliedLocation = useRef(location);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [find, setFind] = useState('');
  const [showFind, setShowFind] = useState(false);
  const findInput = useRef<HTMLInputElement>(null);
  const [finding, setFinding] = useState(false);
  const [findResult, setFindResult] = useState<string | null>(null);
  const found = useRef<{ needle: string; index: number } | null>(null);
  const searchRun = useRef(0);
  const [outline, setOutline] = useState<Array<{
    title: string;
    dest: unknown;
    depth: number;
  }> | null>(null);
  const [showOutline, setShowOutline] = useState(false);
  const [regionMode, setRegionMode] = useState(false);
  const [selectionUi, setSelectionUi] = useState<{
    locators: SourceLocator[];
    x: number;
    y: number;
  } | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const scroll = useRef<HTMLDivElement>(null);
  const rendered = useRef(new Map<number, RenderedPage>());
  const geometryRef = useRef(geometries);
  const loadedPdf = useRef(pdf);
  const currentZoom = useRef(zoom);
  // Screen position a pointer-driven zoom has to keep stable. Every other
  // layout change anchors to the top of the viewport, derived when the change
  // is applied rather than when it is scheduled.
  const pendingAnchor = useRef<{ clientX?: number; clientY?: number } | null>(null);
  const callbacks = useRef({ onPosition, onSelection, onSelectionAction, onAnnotationClick });
  callbacks.current = { onPosition, onSelection, onSelectionAction, onAnnotationClick };
  loadedPdf.current = pdf;
  currentZoom.current = zoom;
  geometryRef.current = geometries;
  const fallback = geometries.get(0) ?? {
    crop: [0, 0, 612, 792] as Rect,
    rotation: 0,
    userUnit: 1,
  };
  const layout = useMemo(
    () =>
      pdfLayout(pdf?.numPages ?? 0, geometries, fallback, zoom.scale, zoom.rotation, size.width),
    [pdf, geometries, zoom.scale, zoom.rotation, size.width],
  );
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  // The layout the reader last applied, with the rotation it was laid out for.
  // A layout change shifts content, so the compensation is always computed
  // against this snapshot and added to the live scroll offset; the reader is
  // never dragged back to a position captured before they kept scrolling.
  const appliedLayout = useRef<{ layout: PdfLayout; rotation: number }>({
    layout,
    rotation: zoom.rotation,
  });
  const labelsRef = useRef(labels);
  labelsRef.current = labels;
  const capture = useCallback(() => {
    const container = scroll.current;
    if (!container || !loadedPdf.current || appliedJump.current < 0) return null;
    const result = readingPosition(
      layoutRef.current,
      versionId,
      currentZoom.current.scale,
      currentZoom.current.rotation,
      container.scrollTop,
      container.scrollLeft,
      container.clientWidth,
      labelsRef.current,
      currentZoom.current.mode,
    );
    // Never persist an estimated crop box while the actual target page is still loading.
    return result && geometryRef.current.has(result.view.page_index) ? result : null;
  }, [versionId]);
  // Resolves the document point sitting at a screen position against an
  // explicit layout snapshot, reading the scroll offset that is live now.
  const anchorAt = useCallback(
    (source: { layout: PdfLayout; rotation: number }, clientX?: number, clientY?: number) => {
      const container = scroll.current;
      if (!container || !source.layout.pages.length) return null;
      const bounds = container.getBoundingClientRect();
      const screenX =
        clientX === undefined
          ? container.clientWidth / 2
          : clamp(clientX - bounds.left - container.clientLeft, 0, container.clientWidth);
      const screenY =
        clientY === undefined
          ? PDF_PADDING
          : clamp(clientY - bounds.top - container.clientTop, 0, container.clientHeight);
      const index = pageAtOffset(source.layout.pages, container.scrollTop + screenY);
      const target = source.layout.pages[index];
      if (!target) return null;
      const [x, y] = viewportToAnchor(
        (container.scrollLeft + screenX - target.left) / target.width,
        (container.scrollTop + screenY - target.top) / target.height,
        target.geometry.rotation + source.rotation,
      );
      return { index, x, y, screenX, screenY } as ViewAnchor;
    },
    [],
  );
  const publish = useCallback(() => {
    const position = capture();
    if (!position) return;
    setPage((pinnedPage.current ?? position.view.page_index) + 1);
    callbacks.current.onPosition?.(position.locator, position.view);
  }, [capture]);
  useLayoutEffect(() => {
    if (positionReader) positionReader.current = capture;
    return () => {
      if (positionReader?.current === capture) positionReader.current = null;
    };
  }, [capture, positionReader]);
  useEffect(() => {
    setPageInput(String(page));
  }, [page]);
  const inputUrl = input && 'url' in input ? input.url : undefined;
  const inputData = input && 'data' in input ? input.data : undefined;
  useEffect(() => {
    let live = true;
    setPdf(null);
    setError(null);
    setGeometries(new Map());
    rendered.current.clear();
    appliedJump.current = -1;
    pinnedPage.current = null;
    const resource = openPdfResource(
      versionId,
      identity,
      inputUrl ? { url: inputUrl } : inputData ? { data: inputData } : undefined,
      (failure) => {
        if (live) setError(pdfReaderErrorMessage(failure, t));
      },
    );
    void resource.promise
      .then(async (document) => {
        const wanted = clamp(initialPage, 0, document.numPages - 1);
        const [first, target, pageLabels] = await Promise.all([
          document.getPage(1),
          document.getPage(wanted + 1),
          document.getPageLabels(),
        ]);
        if (!live) return;
        const measured = new Map([
          [0, pageGeometry(first)],
          [wanted, pageGeometry(target)],
        ]);
        // Resolve small page metadata before publishing a normal document. In a
        // mixed-size PDF this keeps its initial restored horizontal/vertical
        // anchor from being clamped against an undersized provisional scroller.
        if (document.numPages <= 256) {
          let next = 0;
          const measure = async () => {
            while (live && next < document.numPages) {
              const index = next++;
              if (measured.has(index)) continue;
              try {
                measured.set(index, pageGeometry(await document.getPage(index + 1)));
              } catch {
                /* An unreadable page keeps a placeholder and its visible retry state. */
              }
            }
          };
          await Promise.all([measure(), measure(), measure(), measure()]);
        }
        if (!live) return;
        setGeometries(measured);
        setLabels(pageLabels);
        setPdf(document);
        if (initialPage >= document.numPages) {
          setNotice(
            t(
              '保存的页码超出此文档范围，已打开原件首页。',
              'The saved page is outside this document; its first page is open.',
            ),
          );
          setJump({ id: ++jumpNumber.current, index: 0 });
        }
      })
      .catch((failure) => {
        if (live) setError(pdfReaderErrorMessage(failure, t));
      });
    return () => {
      live = false;
      searchRun.current++;
      resource.dispose();
    };
  }, [identity?.local_instance_id, identity?.workspace_id, versionId, inputUrl, inputData, retry]);

  const ensureGeometry = useCallback(async (index: number) => {
    const document = loadedPdf.current;
    if (!document || geometryRef.current.has(index)) return;
    const target = await document.getPage(index + 1);
    if (loadedPdf.current !== document) return;
    setGeometries((old) => {
      if (old.has(index)) return old;
      const next = new Map(old);
      next.set(index, pageGeometry(target));
      return next;
    });
  }, []);
  useEffect(() => {
    if (!pdf) return;
    let live = true;
    // Metadata is small; preflight ordinary documents without ever painting their pages.
    // Very large PDFs retain on-demand metadata instead of walking thousands of page trees.
    const queue = pdf.numPages <= 256 ? Array.from({ length: pdf.numPages }, (_, i) => i) : [];
    let cursor = 0;
    const run = async () => {
      while (live && cursor < queue.length) {
        const index = queue[cursor++];
        try {
          await ensureGeometry(index);
        } catch {
          /* Visible pages display their own failure and retry; this is only prefetch. */
        }
      }
    };
    void Promise.all([run(), run()]);
    return () => {
      live = false;
    };
  }, [pdf, ensureGeometry]);
  useLayoutEffect(() => {
    const container = scroll.current;
    if (!container) return;
    // A real resize (window, sidebar, focus mode) must re-fit immediately: the
    // observer only reports changes, so waiting for a repeated measurement would
    // strand the reader on a stale viewport. Transient height changes are
    // prevented at their source instead of being filtered here.
    const resized = () => {
      const next = { width: container.clientWidth, height: container.clientHeight };
      setSize((current) =>
        current.width === next.width && current.height === next.height ? current : next,
      );
    };
    resized();
    const observer = new ResizeObserver(resized);
    observer.observe(container);
    return () => observer.disconnect();
  }, [pdf, error]);
  const activePage = layout.pages[Math.min(page - 1, layout.pages.length - 1)];
  useLayoutEffect(() => {
    if (!activePage || zoom.mode === 'custom' || !size.width || !size.height) return;
    const natural = pageSize(activePage.geometry, 1, zoom.rotation);
    const widthScale = Math.max(1, size.width - PDF_PADDING * 2) / natural.width;
    const next = pdfScale(
      zoom.mode === 'width'
        ? widthScale
        : Math.min(widthScale, Math.max(1, size.height - PDF_PADDING * 2) / natural.height),
    );
    if (next !== zoom.scale) {
      currentZoom.current = { ...zoom, scale: next };
      setZoom(currentZoom.current);
    }
  }, [activePage?.geometry, size.width, size.height, zoom.mode, zoom.rotation, zoom.scale]);
  useEffect(() => {
    if (appliedLocation.current === location) return;
    appliedLocation.current = location;
    if (validLocation)
      setJump({
        id: ++jumpNumber.current,
        index: validLocation.page_index,
        locator: validLocation,
      });
  }, [location, validLocation]);
  useLayoutEffect(() => {
    const container = scroll.current;
    if (!container) return;
    // Fit modes can schedule a new scale in an earlier layout effect. Keep the
    // previous snapshot until the DOM actually reflects that scale.
    if (currentZoom.current.scale !== zoom.scale || currentZoom.current.rotation !== zoom.rotation)
      return;
    const previous = appliedLayout.current;
    appliedLayout.current = { layout, rotation: zoom.rotation };
    if (!layout.pages.length) return;
    if (appliedJump.current !== jump.id) {
      if (!layout.pages[jump.index]) return;
      if (!geometries.has(jump.index)) {
        void ensureGeometry(jump.index).catch((failure) =>
          setNotice(pdfReaderErrorMessage(failure, t)),
        );
        return;
      }
      const target = layout.pages[jump.index];
      let top = Math.max(0, target.top - PDF_PADDING);
      let left = Math.max(0, target.left + target.width / 2 - container.clientWidth / 2);
      if (jump.locator?.rects.length) {
        const box = jump.locator.rects[0],
          crop = target.geometry.crop;
        const [x, y] = anchorToViewport(
          ((box[0] + box[2]) / 2 - crop[0]) / (crop[2] - crop[0]),
          ((box[1] + box[3]) / 2 - crop[1]) / (crop[3] - crop[1]),
          target.geometry.rotation + zoom.rotation,
        );
        top = Math.max(0, target.top + y * target.height - container.clientHeight / 2);
        left = Math.max(0, target.left + x * target.width - container.clientWidth / 2);
      } else if (jump.view?.page_index === jump.index) {
        const restored = viewScrollPosition(
          layout,
          { ...jump.view, rotation: zoom.rotation },
          container.clientWidth,
        );
        if (restored) {
          top = restored.top;
          left = restored.left;
        }
      }
      appliedJump.current = jump.id;
      pinnedPage.current = jump.index;
      pendingAnchor.current = null;
      container.scrollTo({ top, left, behavior: 'instant' });
      setScrollPosition({ top: container.scrollTop, left: container.scrollLeft });
      publish();
      return;
    }
    if (previous.layout !== layout && previous.layout.pages.length) {
      const hint = pendingAnchor.current;
      pendingAnchor.current = null;
      const source = anchorAt(previous, hint?.clientX, hint?.clientY);
      const target = source ? layout.pages[source.index] : null;
      if (source && target) {
        const [x, y] = anchorToViewport(
          source.x,
          source.y,
          target.geometry.rotation + zoom.rotation,
        );
        const top = Math.max(0, target.top + y * target.height - source.screenY);
        const left = Math.max(0, target.left + x * target.width - source.screenX);
        if (
          Math.abs(top - container.scrollTop) > 0.5 ||
          Math.abs(left - container.scrollLeft) > 0.5
        ) {
          container.scrollTo({ top, left, behavior: 'instant' });
          setScrollPosition({ top: container.scrollTop, left: container.scrollLeft });
        }
      }
    }
    publish();
  }, [layout, jump, geometries, zoom.rotation, zoom.mode, ensureGeometry, publish, anchorAt]);
  const visible = useMemo(
    () => visiblePages(layout, scrollPosition.top, size.height),
    [layout, scrollPosition.top, size.height],
  );
  useEffect(() => {
    for (const item of visible)
      if (!geometries.has(item.index)) void ensureGeometry(item.index).catch(() => {});
  }, [visible, geometries, ensureGeometry]);
  const changeZoom = useCallback(
    (scale: number, mode: Zoom['mode'] = 'custom', clientX?: number, clientY?: number) => {
      pinnedPage.current = null;
      pendingAnchor.current =
        clientX === undefined && clientY === undefined ? null : { clientX, clientY };
      const next = pdfScale(scale),
        current = currentZoom.current;
      currentZoom.current = { ...current, scale: next, mode };
      setZoom(currentZoom.current);
      setSelectionUi(null);
      if (scale < PDF_MIN_SCALE || scale > PDF_MAX_SCALE)
        setNotice(t('PDF 缩放范围为 25%–500%。', 'PDF zoom is limited to 25%–500%.'));
    },
    [t],
  );
  useEffect(() => {
    const container = scroll.current;
    if (!container) return;
    const wheel = (event: WheelEvent) => {
      pinnedPage.current = null;
      if (!event.ctrlKey || event.defaultPrevented) return;
      event.preventDefault();
      event.stopPropagation();
      const delta =
        event.deltaY *
        (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? container.clientHeight : 1);
      changeZoom(
        currentZoom.current.scale * Math.exp(-delta * 0.002),
        'custom',
        event.clientX,
        event.clientY,
      );
    };
    container.addEventListener('wheel', wheel, { passive: false });
    return () => container.removeEventListener('wheel', wheel);
  }, [pdf, error, changeZoom]);
  useEffect(() => {
    if (selectionActive !== false) return;
    setSelectionUi(null);
    const selection = window.getSelection();
    if (selection?.anchorNode && scroll.current?.contains(selection.anchorNode))
      selection.removeAllRanges();
  }, [selectionActive]);
  const onReady = useCallback((index: number, value: RenderedPage | null) => {
    if (value) rendered.current.set(index, value);
    else rendered.current.delete(index);
  }, []);
  function goToPage(number: number) {
    if (!pdf || !Number.isInteger(number) || number < 1 || number > pdf.numPages) {
      setPageInput(String(page));
      setNotice(
        t(
          `请输入 1–${pdf?.numPages ?? 1} 之间的页码。`,
          `Enter a page between 1 and ${pdf?.numPages ?? 1}.`,
        ),
      );
      return;
    }
    setSelectionUi(null);
    setJump({ id: ++jumpNumber.current, index: number - 1 });
  }
  function selected(
    locators: SourceLocator[],
    bounds: { left: number; top: number; bottom: number },
  ) {
    const container = scroll.current;
    if (!locators.length || !container) return;
    const rect = container.getBoundingClientRect();
    setSelectionUi({
      locators,
      x: clamp(bounds.left - rect.left, 8, Math.max(8, container.clientWidth - 270)),
      y: clamp(bounds.top - rect.top - 44, 8, Math.max(8, container.clientHeight - 48)),
    });
    callbacks.current.onSelection?.(locators[0], locators);
    if (locators.length > 1)
      setNotice(
        t(
          `跨 ${locators.length} 页选区将按页保存为 ${locators.length} 条关联摘录。`,
          `This selection spans ${locators.length} pages and will be saved as ${locators.length} page highlights.`,
        ),
      );
  }
  function collectSelection() {
    if (regionMode) return;
    const selection = window.getSelection(),
      container = scroll.current;
    if (!selection || !container || selection.isCollapsed || !selection.rangeCount) return;
    const range = selection.getRangeAt(0);
    if (!container.contains(range.startContainer) || !container.contains(range.endContainer))
      return;
    const locators: SourceLocator[] = [];
    try {
      for (const [index, current] of [...rendered.current].sort((a, b) => a[0] - b[0])) {
        const clipped = document.createRange();
        clipped.selectNodeContents(current.layer);
        const start = clipped.comparePoint(range.startContainer, range.startOffset),
          end = clipped.comparePoint(range.endContainer, range.endOffset);
        if (start > 0 || end < 0) continue;
        if (start === 0) clipped.setStart(range.startContainer, range.startOffset);
        if (end === 0) clipped.setEnd(range.endContainer, range.endOffset);
        const quote = clipped.toString();
        if (!quote.trim()) continue;
        const bounds = current.layer.getBoundingClientRect();
        const rects = [...clipped.getClientRects()]
          .filter(
            (rect) =>
              rect.width > 0 &&
              rect.height > 0 &&
              rect.right > bounds.left &&
              rect.left < bounds.right &&
              rect.bottom > bounds.top &&
              rect.top < bounds.bottom,
          )
          .map((rect) =>
            pdfRect(
              [
                rect.left - bounds.left,
                rect.top - bounds.top,
                rect.right - bounds.left,
                rect.bottom - bounds.top,
              ],
              current.page.view as Rect,
              (x, y) => current.viewport.convertToPdfPoint(x, y),
            ),
          );
        if (!rects.length || rects.length > 1000)
          throw new Error(
            t('此选区过大，请分段保存。', 'This selection is too large; save it in smaller parts.'),
          );
        locators.push({
          kind: 'pdf',
          source_version_id: versionId,
          page_index: index,
          page_label: current.label,
          crop_box: current.page.view as Rect,
          rotation: current.page.rotate,
          coordinate_space: 'pdf_user_space',
          rects,
          quote,
          prefix: '',
          suffix: '',
        });
      }
      if (locators.length > 1) {
        const first = locators[0] as PdfLocator,
          last = locators.at(-1) as PdfLocator;
        if (last.page_index - first.page_index + 1 !== locators.length)
          throw new Error(
            t(
              '选区跨过未加载或无文字的页面，请按可见页面分别摘录。',
              'The selection crosses unloaded or textless pages. Highlight visible pages separately.',
            ),
          );
      }
      selected(locators, range.getBoundingClientRect());
    } catch (failure) {
      setNotice(message(failure));
    }
  }
  async function selectionAction(mode: 'highlight' | 'comment') {
    if (!selectionUi || actionBusy || annotationBusy) return;
    if (!identity) {
      onRequireLibrary?.();
      return;
    }
    if (!callbacks.current.onSelectionAction) return;
    setActionBusy(true);
    try {
      if (await callbacks.current.onSelectionAction(selectionUi.locators, mode)) {
        setSelectionUi(null);
        window.getSelection()?.removeAllRanges();
      }
    } finally {
      setActionBusy(false);
    }
  }
  async function findNext() {
    if (!pdf || !find.trim() || finding) return;
    const document = pdf,
      run = ++searchRun.current,
      needle = find.trim().toLocaleLowerCase();
    const start = found.current?.needle === needle ? found.current.index + 1 : page - 1;
    setFinding(true);
    setFindResult(null);
    try {
      for (let offset = 0; offset < document.numPages; offset++) {
        const index = (start + offset) % document.numPages,
          target = await document.getPage(index + 1),
          content = await target.getTextContent();
        if (run !== searchRun.current || loadedPdf.current !== document) return;
        const text = content.items.map((item) => ('str' in item ? item.str : '')).join('');
        if (text.toLocaleLowerCase().includes(needle)) {
          found.current = { needle, index };
          goToPage(index + 1);
          setFindResult(t(`第 ${index + 1} 页包含此文字。`, `Text found on page ${index + 1}.`));
          return;
        }
        if (!rendered.current.has(index)) target.cleanup();
      }
      setFindResult(
        t(
          '没有找到此文字；无文本层的页面仍可阅读或框选区域。',
          'No matching text. Pages without a text layer remain readable and support region selection.',
        ),
      );
    } catch (failure) {
      if (run === searchRun.current) setFindResult(pdfReaderErrorMessage(failure, t));
    } finally {
      if (run === searchRun.current) setFinding(false);
    }
  }
  async function toggleOutline() {
    setShowOutline((value) => !value);
    if (outline || !pdf) return;
    const document = pdf;
    try {
      const nodes = await document.getOutline();
      if (loadedPdf.current !== document) return;
      const flat: Array<{ title: string; dest: unknown; depth: number }> = [];
      function walk(items: NonNullable<typeof nodes>, depth: number) {
        for (const item of items) {
          flat.push({ title: item.title, dest: item.dest, depth });
          if (depth < 10) walk(item.items, depth + 1);
        }
      }
      walk(nodes ?? [], 0);
      setOutline(flat);
    } catch (failure) {
      if (loadedPdf.current === document) setNotice(message(failure));
    }
  }
  async function openOutline(dest: unknown) {
    if (!pdf) return;
    const document = pdf;
    try {
      const value = typeof dest === 'string' ? await document.getDestination(dest) : dest;
      if (!Array.isArray(value) || !value.length)
        throw new Error(t('此目录位置不可用。', 'This outline destination is unavailable.'));
      const index = typeof value[0] === 'number' ? value[0] : await document.getPageIndex(value[0]);
      if (loadedPdf.current !== document) return;
      goToPage(index + 1);
      setShowOutline(false);
    } catch (failure) {
      if (loadedPdf.current === document) setNotice(message(failure));
    }
  }
  const zoomValue = zoom.mode === 'custom' ? String(zoom.scale) : zoom.mode;
  const zoomValues = [...new Set([0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4, 5, zoom.scale])].sort(
    (a, b) => a - b,
  );
  return (
    <div
      className="source-pdf"
      data-pdf-page-count={pdf?.numPages}
      data-pdf-scale={zoom.scale}
      data-pdf-rotation={zoom.rotation}
    >
      <style>{scopedPdfViewerStyles}</style>
      <div
        className="source-pdf-toolbar"
        role="toolbar"
        aria-label={t('PDF 阅读工具', 'PDF reading tools')}
      >
        {startControls && (
          <div className="source-pdf-control-group source-pdf-mode-group">{startControls}</div>
        )}
        <div className="source-pdf-control-group source-pdf-page-group">
          <button
            className="icon-button"
            disabled={!pdf || page <= 1}
            onClick={() => goToPage(page - 1)}
            aria-label={t('上一页', 'Previous page')}
          >
            <ChevronLeft size={16} />
          </button>
          <label className="source-pdf-page-number">
            <span className="sr-only">{t('页码', 'Page')}</span>
            <input
              type="text"
              inputMode="numeric"
              aria-label={t('页码', 'Page')}
              value={pageInput}
              disabled={!pdf}
              onChange={(event) => setPageInput(event.target.value)}
              onBlur={() => {
                if (pageInput !== String(page)) goToPage(Number(pageInput));
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  goToPage(Number(pageInput));
                } else if (event.key === 'Escape') setPageInput(String(page));
              }}
            />
            <span>/ {pdf?.numPages ?? '—'}</span>
          </label>
          <button
            className="icon-button"
            disabled={!pdf || page >= pdf.numPages}
            onClick={() => goToPage(page + 1)}
            aria-label={t('下一页', 'Next page')}
          >
            <ChevronRight size={16} />
          </button>
        </div>
        <div className="source-pdf-control-group source-pdf-zoom-group">
          <button
            className="icon-button"
            disabled={!pdf || zoom.scale <= PDF_MIN_SCALE}
            onClick={() => changeZoom(zoom.scale / 1.15)}
            aria-label={t('缩小 PDF', 'Zoom out PDF')}
          >
            <ZoomOut size={16} />
          </button>
          <Select
            className="source-pdf-zoom-select"
            aria-label={t('PDF 缩放', 'PDF zoom')}
            disabled={!pdf}
            value={zoomValue}
            onValueChange={(value) => {
              if (value === 'width' || value === 'page') changeZoom(zoom.scale, value);
              else changeZoom(Number(value));
            }}
          >
            <option value="width">
              {t('适宽', 'Fit width')}
              {zoom.mode === 'width' ? ` · ${Math.round(zoom.scale * 100)}%` : ''}
            </option>
            <option value="page">
              {t('适页', 'Fit page')}
              {zoom.mode === 'page' ? ` · ${Math.round(zoom.scale * 100)}%` : ''}
            </option>
            {zoomValues.map((value) => (
              <option key={value} value={value}>
                {Math.round(value * 100)}%
              </option>
            ))}
          </Select>
          <button
            className="icon-button"
            disabled={!pdf || zoom.scale >= PDF_MAX_SCALE}
            onClick={() => changeZoom(zoom.scale * 1.15)}
            aria-label={t('放大 PDF', 'Zoom in PDF')}
          >
            <ZoomIn size={16} />
          </button>
        </div>
        <div className="source-pdf-control-group source-pdf-secondary-group">
          <button
            className="icon-button"
            disabled={!pdf}
            onClick={() => {
              pendingAnchor.current = null;
              setZoom((value) => ({ ...value, rotation: pdfRotation(value.rotation + 90) }));
              setSelectionUi(null);
            }}
            aria-label={t('旋转 PDF', 'Rotate PDF')}
            title={`${t('旋转', 'Rotate')} · ${zoom.rotation}°`}
          >
            <RotateCw size={16} />
          </button>
          <button
            className="icon-button"
            disabled={!pdf}
            aria-pressed={regionMode}
            aria-label={t('框选区域摘录', 'Select a region')}
            onClick={() => {
              setRegionMode(!regionMode);
              setSelectionUi(null);
            }}
          >
            <SquareDashed size={16} />
          </button>
          <button disabled={!pdf} onClick={() => void toggleOutline()} aria-expanded={showOutline}>
            {t('目录', 'Contents')}
          </button>
          <button
            className="icon-button"
            disabled={!pdf}
            aria-expanded={showFind}
            aria-label={t('查找 PDF 文字', 'Find PDF text')}
            onClick={() => {
              setShowFind(!showFind);
              if (!showFind) requestAnimationFrame(() => findInput.current?.focus());
            }}
          >
            <Search size={16} />
          </button>
        </div>
        <div className="source-pdf-control-group source-pdf-more">
          <PopupMenu
            label={t('更多 PDF 工具', 'More PDF tools')}
            trigger={<MoreHorizontal size={16} />}
            placement="bottom-start"
            disabled={!pdf}
            items={[
              {
                id: 'rotate',
                label: t('旋转 PDF', 'Rotate PDF'),
                icon: <RotateCw size={16} />,
                onSelect: () => {
                  pendingAnchor.current = null;
                  setZoom((value) => ({
                    ...value,
                    rotation: pdfRotation(value.rotation + 90),
                  }));
                  setSelectionUi(null);
                },
              },
              {
                id: 'region',
                label: t('框选区域摘录', 'Select a region'),
                icon: <SquareDashed size={16} />,
                checked: regionMode,
                onSelect: () => {
                  setRegionMode(!regionMode);
                  setSelectionUi(null);
                },
              },
              {
                id: 'outline',
                label: t('目录', 'Contents'),
                separatorBefore: true,
                onSelect: () => void toggleOutline(),
              },
              {
                id: 'find',
                label: t('查找 PDF 文字', 'Find PDF text'),
                icon: <Search size={16} />,
                onSelect: () => {
                  setShowFind(!showFind);
                  if (!showFind) requestAnimationFrame(() => findInput.current?.focus());
                },
              },
            ]}
          />
        </div>
      </div>
      {showFind && (
        <form
          className="source-find"
          onSubmit={(event) => {
            event.preventDefault();
            void findNext();
          }}
        >
          <input
            ref={findInput}
            aria-label={t('查找 PDF 文字', 'Find PDF text')}
            value={find}
            onChange={(event) => {
              setFind(event.target.value);
              searchRun.current++;
              setFinding(false);
            }}
            placeholder={t('查找文内文字', 'Find in document')}
          />
          <button disabled={finding || !find.trim() || !pdf}>
            {finding ? t('查找中…', 'Searching…') : t('下一个', 'Find next')}
          </button>
          {finding && (
            <button
              type="button"
              onClick={() => {
                searchRun.current++;
                setFinding(false);
              }}
            >
              {t('取消', 'Cancel')}
            </button>
          )}
          {findResult && <small role="status">{findResult}</small>}
        </form>
      )}
      {showOutline && (
        <nav className="source-outline" aria-label={t('PDF 目录', 'PDF contents')}>
          {outline?.length ? (
            outline.map((entry, index) => (
              <button
                key={index}
                style={{ paddingInlineStart: 12 + entry.depth * 12 }}
                onClick={() => void openOutline(entry.dest)}
              >
                {entry.title}
              </button>
            ))
          ) : (
            <p>
              {outline
                ? t('此 PDF 没有目录。', 'This PDF has no outline.')
                : t('加载目录…', 'Loading contents…')}
            </p>
          )}
        </nav>
      )}
      {regionMode && (
        <p className="source-pdf-hint" role="status">
          {t(
            '在单页上拖动框选；区域摘录不假定存在文字。Esc 退出。',
            'Drag on one page to mark a region. A region does not imply available text. Esc exits.',
          )}
        </p>
      )}
      {notice && (
        <div className="source-pdf-hint" role="status">
          <span>{notice}</span>
          <button
            className="icon-button"
            aria-label={t('关闭提示', 'Dismiss')}
            onClick={() => setNotice(null)}
          >
            <X size={14} />
          </button>
        </div>
      )}
      {error ? (
        <PdfFailureNotice
          error={error}
          versionId={versionId}
          location={location}
          onRetry={() => setRetry((n) => n + 1)}
        />
      ) : pdf ? (
        <div className="source-pdf-viewport">
          <div
            className="source-pdf-scroll"
            ref={scroll}
            data-local-zoom="pdf"
            tabIndex={0}
            role="region"
            aria-label={t('PDF 连续阅读区域', 'Continuous PDF reading area')}
            onPointerDown={() => {
              pinnedPage.current = null;
            }}

            onScroll={() => {
              const container = scroll.current;
              if (!container) return;
              setScrollPosition({ top: container.scrollTop, left: container.scrollLeft });
              publish();
              setSelectionUi(null);
            }}
            onPointerUp={() => requestAnimationFrame(collectSelection)}
            onKeyUp={collectSelection}
            onClick={(event) => {
              if (
                regionMode ||
                !window.getSelection()?.isCollapsed ||
                (event.target as HTMLElement).closest('button')
              )
                return;
              const pageElement = (event.target as HTMLElement).closest<HTMLElement>(
                '[data-pdf-page-index]',
              );
              if (!pageElement) return;
              const index = Number(pageElement.dataset.pdfPageIndex),
                current = rendered.current.get(index);
              if (!current) return;
              const bounds = current.layer.getBoundingClientRect(),
                [x, y] = current.viewport.convertToPdfPoint(
                  event.clientX - bounds.left,
                  event.clientY - bounds.top,
                );
              const annotation = annotations.find(
                (item) =>
                  !item.trashed &&
                  item.locator.kind === 'pdf' &&
                  item.source_version_id === versionId &&
                  item.locator.page_index === index &&
                  item.locator.rects.some(
                    (rect) => x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3],
                  ),
              );
              if (annotation) callbacks.current.onAnnotationClick?.(annotation);
            }}
            onKeyDown={(event) => {
              if (
                [
                  'ArrowUp',
                  'ArrowDown',
                  'ArrowLeft',
                  'ArrowRight',
                  'PageUp',
                  'PageDown',
                  'Home',
                  'End',
                  ' ',
                ].includes(event.key)
              )
                pinnedPage.current = null;
              if (event.nativeEvent.isComposing || event.keyCode === 229) return;
              if (event.key === 'Escape') {
                setRegionMode(false);
                setSelectionUi(null);
                return;
              }
              if (!(event.ctrlKey || event.metaKey)) return;
              if (['+', '=', '-', '0'].includes(event.key)) {
                event.preventDefault();
                event.stopPropagation();
                changeZoom(
                  event.key === '0' ? 1 : zoom.scale * (event.key === '-' ? 1 / 1.15 : 1.15),
                );
              } else if (event.key.toLowerCase() === 'f') {
                event.preventDefault();
                event.stopPropagation();
                setShowFind(true);
                requestAnimationFrame(() => findInput.current?.focus());
              }
            }}
          >
            <div
              className="source-pdf-pages"
              style={{ width: layout.width, height: layout.height }}
            >
              {visible.map((item) => (
                <PdfPage
                  key={item.index}
                  pdf={pdf}
                  layout={item}
                  scale={zoom.scale}
                  rotation={zoom.rotation}
                  versionId={versionId}
                  label={labels?.[item.index] ?? null}
                  location={validLocation?.page_index === item.index ? validLocation : null}
                  annotations={annotations}
                  selectedAnnotationId={selectedAnnotationId}
                  regionMode={regionMode}
                  onRegion={selected}
                  onReady={onReady}
                  onAnnotationClick={onAnnotationClick}
                />
              ))}
            </div>
          </div>
          {selectionUi && (
            <div
              className="source-pdf-selection-popover"
              role="toolbar"
              aria-label={t('选区操作', 'Selection actions')}
              style={{ left: selectionUi.x, top: selectionUi.y }}
              onPointerDown={(event) => event.preventDefault()}
            >
              {identity ? (
                <>
                  <button
                    disabled={actionBusy || annotationBusy || !onSelectionAction}
                    onClick={() => void selectionAction('highlight')}
                  >
                    <Highlighter size={15} />
                    {t('高亮', 'Highlight')}
                  </button>
                  <button
                    disabled={actionBusy || annotationBusy || !onSelectionAction}
                    onClick={() => void selectionAction('comment')}
                  >
                    <MessageSquarePlus size={15} />
                    {t('旁注', 'Add note')}
                  </button>
                </>
              ) : onRequireLibrary ? (
                <button onClick={onRequireLibrary}>
                  {t('保存到知识库以批注', 'Save to a workspace to annotate')}
                </button>
              ) : (
                <span>{t('保存到知识库后可添加批注。', 'Save to a workspace to add notes.')}</span>
              )}
              <button
                className="icon-button"
                aria-label={t('取消选区', 'Dismiss selection')}
                onClick={() => {
                  setSelectionUi(null);
                  callbacks.current.onSelection?.(null);
                }}
              >
                <X size={14} />
              </button>
            </div>
          )}
        </div>
      ) : (
        <p className="source-pdf-loading" role="status">
          {t('正在读取原件…', 'Opening original…')}
        </p>
      )}
    </div>
  );
}

function pageGeometry(page: PDFPageProxy): PdfPageGeometry {
  return { crop: [...page.view] as Rect, rotation: page.rotate, userUnit: page.userUnit };
}

function PdfFailureNotice({
  error,
  versionId,
  location,
  onRetry,
}: {
  error: string;
  versionId: string;
  location: SourceLocator | null;
  onRetry: () => void;
}) {
  const { t } = useI18n();
  const excerpt = pdfErrorExcerpt(versionId, location);
  return (
    <div role="alert" className="source-notice source-pdf-failure">
      <div className="source-pdf-failure-summary">
        <p>{error}</p>
        <button onClick={onRetry}>{t('重试', 'Retry')}</button>
      </div>
      <div className="source-pdf-failure-details">
        {excerpt && (
          <div>
            <p>
              {t('已保留的摘录快照', 'Saved excerpt snapshot')} · {t('页', 'Page')} {excerpt.page}
            </p>
            <blockquote>{excerpt.quote}</blockquote>
          </div>
        )}
        <p>
          {t(
            '仍保留当前固定版本，未跳转到其他版本或相似内容。',
            'The selected version is retained. No other version or similar content has been opened.',
          )}
        </p>
      </div>
    </div>
  );
}

const PdfPage = memo(function PdfPage({
  pdf,
  layout,
  scale,
  rotation,
  versionId,
  label,
  location,
  annotations,
  selectedAnnotationId,
  regionMode,
  onRegion,
  onReady,
  onAnnotationClick,
}: {
  pdf: PDFDocumentProxy;
  layout: PdfPageLayout;
  scale: number;
  rotation: number;
  versionId: string;
  label: string | null;
  location: PdfLocator | null;
  annotations: Annotation[];
  selectedAnnotationId?: string | null;
  regionMode: boolean;
  onRegion: (
    locators: SourceLocator[],
    bounds: { left: number; top: number; bottom: number },
  ) => void;
  onReady: (index: number, value: RenderedPage | null) => void;
  onAnnotationClick?: (annotation: Annotation) => void;
}) {
  const { t } = useI18n();
  const canvas = useRef<HTMLCanvasElement>(null),
    layer = useRef<HTMLDivElement>(null);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  // Tracks whether this canvas already holds a real bitmap, and the page it came
  // from, so a re-render at a new scale can keep showing it until the
  // replacement bitmap is ready.
  const painted = useRef(false);
  const lastRendered = useRef<RenderedPage | null>(null);
  const [error, setError] = useState<string | null>(null),
    [textWarning, setTextWarning] = useState<string | null>(null);
  const [retry, setRetry] = useState(0),
    [pixelRatio, setPixelRatio] = useState(() => window.devicePixelRatio || 1);
  const [region, setRegion] = useState<Rect | null>(null);
  const regionStart = useRef<[number, number] | null>(null);
  const [showTarget, setShowTarget] = useState(!!location);
  useEffect(() => {
    setShowTarget(!!location);
    if (!location) return;
    const timer = setTimeout(() => setShowTarget(false), 2600);
    return () => clearTimeout(timer);
  }, [location]);
  useEffect(() => {
    const resized = () => setPixelRatio(window.devicePixelRatio || 1);
    window.addEventListener('resize', resized);
    return () => window.removeEventListener('resize', resized);
  }, []);
  const pageNumber = layout.index + 1;
  useEffect(() => {
    let live = true,
      rendering: ReturnType<PDFPageProxy['render']> | undefined;
    let text: TextLayer | undefined, loaded: PDFPageProxy | undefined;
    const output = canvas.current,
      textContainer = layer.current;
    // A page that already has pixels keeps them on screen while the replacement
    // bitmap is painted offscreen, so a fit-mode switch, sidebar toggle or window
    // resize never flashes a blank paper-white page.
    const keepPainted = painted.current;
    setError(null);
    setTextWarning(null);
    if (!keepPainted) {
      setRendered(null);
      onReady(layout.index, null);
    } else if (lastRendered.current) {
      setRendered(lastRendered.current);
      onReady(layout.index, lastRendered.current);
    }
    void (async () => {
      loaded = await pdf.getPage(pageNumber);
      if (!live || !output || !textContainer) return;
      const viewport = loaded.getViewport({
        scale,
        rotation: pdfRotation(loaded.rotate + rotation),
      });
      const density = Math.min(
        pixelRatio,
        2,
        Math.sqrt(8_000_000 / (viewport.width * viewport.height)),
      );
      const target = document.createElement('canvas');
      target.width = Math.ceil(viewport.width * density);
      target.height = Math.ceil(viewport.height * density);
      textContainer.replaceChildren();
      textContainer.style.setProperty('--scale-factor', String(scale));
      textContainer.style.setProperty('--total-scale-factor', String(scale * loaded.userUnit));
      textContainer.style.setProperty('--scale-round-x', '1px');
      textContainer.style.setProperty('--scale-round-y', '1px');
      rendering = loaded.render({
        canvas: target,
        viewport,
        transform: [density, 0, 0, density, 0, 0],
      });
      text = new TextLayer({
        textContentSource: loaded.streamTextContent(),
        container: textContainer,
        viewport,
      });
      const [paint, words] = await Promise.allSettled([rendering.promise, text.render()]);
      if (!live) return;
      if (paint.status === 'rejected') throw paint.reason;
      // Swap the finished bitmap into the visible canvas in a single frame; the
      // previous page stayed on screen until now, so there is no blank interval.
      output.width = target.width;
      output.height = target.height;
      output.style.width = `${viewport.width}px`;
      output.style.height = `${viewport.height}px`;
      output.getContext('2d')?.drawImage(target, 0, 0);
      painted.current = true;
      if (words.status === 'rejected')
        setTextWarning(
          t(
            '本页文字层不可用；可阅读原页或框选区域。',
            'Text is unavailable on this page. Read the original or select a region.',
          ),
        );
      else if (!textContainer.textContent?.trim())
        setTextWarning(
          t(
            '本页没有可选择的文字，可框选区域。',
            'This page has no selectable text. You can select a region.',
          ),
        );
      const boundary = document.createElement('div');
      boundary.className = 'endOfContent';
      textContainer.append(boundary);
      const result = { page: loaded, viewport, layer: textContainer, label };
      lastRendered.current = result;
      setRendered(result);
      onReady(layout.index, result);
    })().catch((failure) => {
      if (live) setError(pdfReaderErrorMessage(failure, t));
    });
    return () => {
      live = false;
      rendering?.cancel();
      text?.cancel();
      onReady(layout.index, null);
      textContainer?.replaceChildren();
      loaded?.cleanup();
    };
  }, [pdf, pageNumber, scale, rotation, versionId, label, pixelRatio, retry, onReady]);
  useEffect(() => {
    if (!regionMode) {
      setRegion(null);
      regionStart.current = null;
    }
  }, [regionMode]);
  const marks = annotations.filter(
    (item) =>
      !item.trashed &&
      item.source_version_id === versionId &&
      item.locator.kind === 'pdf' &&
      item.locator.page_index === layout.index,
  );
  function markStyle(rect: Rect) {
    if (!rendered) return {};
    const [x1, y1] = rendered.viewport.convertToViewportPoint(rect[0], rect[1]);
    const [x2, y2] = rendered.viewport.convertToViewportPoint(rect[2], rect[3]);
    return {
      left: Math.min(x1, x2),
      top: Math.min(y1, y2),
      width: Math.abs(x2 - x1),
      height: Math.abs(y2 - y1),
    };
  }
  const pinGroups: Array<{ top: number; annotations: Annotation[] }> = [];
  if (rendered) {
    const pins = marks
      .flatMap((annotation) => {
        const rect = (annotation.locator as PdfLocator).rects[0];
        return rect
          ? [
              {
                annotation,
                top: clamp(Number(markStyle(rect).top) - 2, 0, Math.max(0, layout.height - 24)),
              },
            ]
          : [];
      })
      .sort((a, b) => a.top - b.top || a.annotation.id.localeCompare(b.annotation.id));
    for (const pin of pins) {
      const previous = pinGroups.at(-1);
      if (previous && pin.top - previous.top < 28) previous.annotations.push(pin.annotation);
      else pinGroups.push({ top: pin.top, annotations: [pin.annotation] });
    }
  }
  function regionPoint(event: React.PointerEvent<HTMLDivElement>): [number, number] {
    const bounds = event.currentTarget.getBoundingClientRect();
    return [
      clamp(event.clientX - bounds.left, 0, layout.width),
      clamp(event.clientY - bounds.top, 0, layout.height),
    ];
  }
  return (
    <div
      className="source-pdf-page-slot"
      style={{
        position: 'absolute',
        top: layout.top,
        left: layout.left,
        width: layout.width,
        height: layout.height,
      }}
    >
      <small className="source-pdf-page-label" aria-hidden="true">
        {label ?? pageNumber}
      </small>
      <div
        className={`source-pdf-page${showTarget && location && !location.rects.length ? ' is-jump-target' : ''}`}
        data-pdf-page-index={layout.index}
        data-rendered={!!rendered}
        style={{ width: layout.width, height: layout.height }}
      >
        <canvas ref={canvas} aria-label={`PDF ${label ?? pageNumber}`} />
        <div
          className="textLayer"
          ref={layer}
          onPointerDown={() => layer.current?.classList.add('selecting')}
          onPointerUp={() => layer.current?.classList.remove('selecting')}
          onPointerCancel={() => layer.current?.classList.remove('selecting')}
        />
        {!rendered && !error && (
          <span className="source-pdf-page-loading" role="status">
            {t('加载页面…', 'Loading page…')} {label ?? pageNumber}
          </span>
        )}
        {error && (
          <div className="source-pdf-page-error">
            <PdfFailureNotice
              error={error}
              versionId={versionId}
              location={location}
              onRetry={() => setRetry((value) => value + 1)}
            />
          </div>
        )}
        {textWarning && <span className="source-pdf-text-warning">{textWarning}</span>}
        {rendered &&
          marks.map((annotation) => (
            <div
              key={annotation.id}
              className="source-pdf-annotation"
              data-annotation-id={annotation.id}
            >
              {(annotation.locator as PdfLocator).rects.map((rect, index) => (
                <span
                  className={`source-pdf-mark source-pdf-saved-mark${selectedAnnotationId === annotation.id ? ' is-active' : ''}`}
                  key={index}
                  style={markStyle(rect)}
                />
              ))}
            </div>
          ))}
        {pinGroups.map((group) => (
          <div
            className="source-pdf-pin-anchor"
            key={group.annotations[0].id}
            style={{ top: group.top }}
          >
            {group.annotations.length === 1 ? (
              <button
                className={`source-pdf-annotation-pin${selectedAnnotationId === group.annotations[0].id ? ' is-active' : ''}`}
                aria-label={`${t('查看旁注', 'Show note')}: ${group.annotations[0].comment || group.annotations[0].locator.quote}`}
                title={group.annotations[0].comment || group.annotations[0].locator.quote}
                onClick={() => onAnnotationClick?.(group.annotations[0])}
              >
                <MessageSquarePlus size={13} />
              </button>
            ) : (
              <PopupMenu
                className={`source-pdf-annotation-pin${group.annotations.some((item) => item.id === selectedAnnotationId) ? ' is-active' : ''}`}
                label={t(
                  `查看此处 ${group.annotations.length} 条旁注`,
                  `Show ${group.annotations.length} notes here`,
                )}
                trigger={<span>{group.annotations.length}</span>}
                placement="bottom-start"
                items={group.annotations.map((annotation, index) => ({
                  id: annotation.id,
                  label: `${index + 1}. ${annotation.comment ? `${annotation.comment} · ` : ''}${annotation.locator.quote}`,
                  onSelect: () => onAnnotationClick?.(annotation),
                }))}
              />
            )}
          </div>
        ))}
        {rendered &&
          showTarget &&
          location?.rects.map((rect, index) => (
            <span
              className="source-pdf-mark source-pdf-target-mark"
              key={index}
              style={markStyle(rect)}
            />
          ))}
        {regionMode && rendered && (
          <div
            className="source-pdf-region-surface"
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.preventDefault();
              event.stopPropagation();
              event.currentTarget.setPointerCapture(event.pointerId);
              regionStart.current = regionPoint(event);
              setRegion(null);
            }}
            onPointerMove={(event) => {
              const start = regionStart.current;
              if (!start) return;
              const point = regionPoint(event);
              setRegion([
                Math.min(start[0], point[0]),
                Math.min(start[1], point[1]),
                Math.max(start[0], point[0]),
                Math.max(start[1], point[1]),
              ]);
            }}
            onPointerCancel={() => {
              regionStart.current = null;
              setRegion(null);
            }}
            onPointerUp={(event) => {
              const start = regionStart.current;
              regionStart.current = null;
              if (!start) return;
              event.preventDefault();
              event.stopPropagation();
              const point = regionPoint(event),
                box: Rect = [
                  Math.min(start[0], point[0]),
                  Math.min(start[1], point[1]),
                  Math.max(start[0], point[0]),
                  Math.max(start[1], point[1]),
                ];
              if (box[2] - box[0] < 4 || box[3] - box[1] < 4) {
                setRegion(null);
                return;
              }
              const bounds = event.currentTarget.getBoundingClientRect();
              onRegion(
                [
                  {
                    kind: 'pdf',
                    source_version_id: versionId,
                    page_index: layout.index,
                    page_label: label,
                    crop_box: rendered.page.view as Rect,
                    rotation: rendered.page.rotate,
                    coordinate_space: 'pdf_user_space',
                    rects: [
                      pdfRect(box, rendered.page.view as Rect, (x, y) =>
                        rendered.viewport.convertToPdfPoint(x, y),
                      ),
                    ],
                    quote: t(
                      `区域摘录 · 第 ${label ?? pageNumber} 页（无文字引文）`,
                      `Region highlight · page ${label ?? pageNumber} (no text quotation)`,
                    ),
                    prefix: '',
                    suffix: '',
                  },
                ],
                {
                  left: bounds.left + box[0],
                  top: bounds.top + box[1],
                  bottom: bounds.top + box[3],
                },
              );
            }}
          >
            {region && (
              <span
                className="source-pdf-region-selection"
                style={{
                  left: region[0],
                  top: region[1],
                  width: region[2] - region[0],
                  height: region[3] - region[1],
                }}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
});
