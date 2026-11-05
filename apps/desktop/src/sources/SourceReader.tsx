import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from 'react';
import {
  ArrowLeft,
  BookOpen,
  ChevronLeft,
  ChevronRight,
  Highlighter,
  Info,
  List,
  LoaderCircle,
  MoreHorizontal,
  Search,
  X,
} from 'lucide-react';
import type {
  Annotation,
  AnnotationSummary,
  DesktopCommand,
  InboxState,
  ObjectMetadata,
  PdfReadingView,
  ReadingState,
  ResolvedSourceTarget,
  SourceLocator,
  SourceCoverData,
  SourceSummary,
  SourceTextPage,
  SourceTocItem,
  SourceVersion,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, choose, message } from '../api';
import { useI18n } from '../i18n';
import { PageFrame } from '../layout/PageFrame';
import { PopupMenu } from '../ui/PopupMenu';
import { Select } from '../ui/Select';
import { tagToneClass } from '../ui/tag-color';
import { ReferencePanel } from '../editor/reference-panel';
import { AddToReview } from '../review/AddToReview';
import { selectedTextBlock, textLocator } from './locators';
import {
  activeBookChapterIndex,
  adjacentBookChapterIndex,
  bookChapterTarget,
  isBookScrollIntentKey,
} from './book-navigation';
import { SourceBlockImage } from './SourceBlockImage';
import { portableActions } from '../portability/actions';
import { useKnowledgeTaskNavigation } from '../inference/knowledge-task-navigation';
import { useSourceActions } from './useSourceActions';
import { sourceReading } from './reading-registry';
import { SourceNoteImport } from './SourceNoteImport';
import { DocxReadView } from './DocxReadView';
import { MarkdownReadView } from './MarkdownReadView';
import { cleaningSummary } from './cleaning-settings';
import { MAX_ANNOTATIONS_TO_NOTE } from './annotation-to-note';
import { formatSourceSize } from './source-size';
import { filterAnnotations } from './annotation-filter';
import type { PdfReadingPosition } from './pdf-reading-geometry';
import './sources.css';
import './annotation-to-note.css';
import './source-reader-v16.css';

const PdfReader = lazy(() =>
  import('./PdfReader').then((module) => ({ default: module.PdfReader })),
);
export interface SourceReaderProps {
  identity: WorkspaceIdentity;
  sourceId: string;
  versionId?: string;
  initialTarget?: ResolvedSourceTarget;
  initialPdfView?: PdfReadingView | null;
  onClose?: () => void;
  onAddToNote?: (annotations: Annotation[]) => void;
  onOpenSource?: (sourceId: string, versionId: string, annotationId: string) => void;
  onTitleChange?: (sourceId: string, title: string) => void;
  onChanged?: () => void;
  /** The note pane owns the second column; do not create a third pane. */
  hideSidebar?: boolean;
  active?: boolean;
}
export function SourceReader(props: SourceReaderProps) {
  return (
    <ReaderScope
      key={JSON.stringify([
        props.identity,
        props.sourceId,
        props.versionId,
        props.initialTarget,
        props.initialPdfView,
      ])}
      {...props}
    />
  );
}
function ReaderScope({
  identity,
  sourceId,
  versionId,
  initialTarget,
  initialPdfView,
  onClose,
  onAddToNote,
  onOpenSource,
  hideSidebar,
  onTitleChange,
  onChanged,
  active = true,
}: SourceReaderProps) {
  const { t } = useI18n();
  const taskNavigation = useKnowledgeTaskNavigation();
  const actions = useSourceActions(identity, `reader/${sourceId}`);
  const [source, setSource] = useState<SourceSummary | null>(null);
  const [metadata, setMetadata] = useState<ObjectMetadata | null>(null);
  const [reading, setReading] = useState<ReadingState | null>(null);
  const [pdfView, setPdfView] = useState<PdfReadingView | null>(initialPdfView ?? null);
  const [pdfTextView, setPdfTextView] = useState(false);
  const [pdfTextAvailable, setPdfTextAvailable] = useState(false);
  const [pdfOcrApplied, setPdfOcrApplied] = useState(false);
  const [version, setVersion] = useState<SourceVersion | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<string | null>(
    initialTarget?.source_version_id ?? versionId ?? null,
  );
  const [textTarget, setTextTarget] = useState<{ parseId: string | null; offset: number }>({
    parseId: initialTarget?.parse_id ?? null,
    offset: initialTarget?.block_offset ?? 0,
  });
  const [versions, setVersions] = useState<SourceVersion[]>([]);
  const [moreVersions, setMoreVersions] = useState(false);
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [pdfAnnotations, setPdfAnnotations] = useState<Annotation[]>([]);
  const [focusedAnnotation, setFocusedAnnotation] = useState<string | null>(
    initialTarget?.annotation_id ?? null,
  );
  const [referencedAnnotation, setReferencedAnnotation] = useState<Annotation | null>(null);
  const annotationSidebar = useRef<HTMLElement>(null);
  const appliedAnnotationFocus = useRef<HTMLElement | null>(null);
  const [selectedAnnotationIds, setSelectedAnnotationIds] = useState<Set<string>>(() => new Set());
  const [annotationTrash, setAnnotationTrash] = useState(false);
  const [annotationQuery, setAnnotationQuery] = useState('');
  const [annotationSearchScope, setAnnotationSearchScope] = useState<'loaded' | 'library'>(
    'loaded',
  );
  const [libraryAnnotationHits, setLibraryAnnotationHits] = useState<AnnotationSummary[]>([]);
  const [libraryAnnotationCursor, setLibraryAnnotationCursor] = useState<string | null>(null);
  const [libraryAnnotationBusy, setLibraryAnnotationBusy] = useState(false);
  const [libraryAnnotationSearched, setLibraryAnnotationSearched] = useState(false);
  const [libraryAnnotationError, setLibraryAnnotationError] = useState<string | null>(null);
  const [libraryAnnotationRetry, setLibraryAnnotationRetry] = useState(0);
  const [openingLibraryAnnotation, setOpeningLibraryAnnotation] = useState<string | null>(null);
  const libraryAnnotationSequence = useRef(0);
  const annotationSearchComposing = useRef(false);
  const [annotationSearchCompositionVersion, setAnnotationSearchCompositionVersion] = useState(0);
  const listedAnnotations =
    version?.format === 'pdf' && !annotationTrash ? pdfAnnotations : annotations;
  const displayedAnnotations =
    referencedAnnotation &&
    referencedAnnotation.source_version_id === selectedVersion &&
    referencedAnnotation.trashed === annotationTrash &&
    !listedAnnotations.some((item) => item.id === referencedAnnotation.id)
      ? [referencedAnnotation, ...listedAnnotations]
      : listedAnnotations;
  const filteredAnnotations = filterAnnotations(displayedAnnotations, annotationQuery);
  const selectedAnnotations = filteredAnnotations.filter(
    (item) => !item.trashed && selectedAnnotationIds.has(item.id),
  );
  const [moreAnnotations, setMoreAnnotations] = useState(false);
  const [side, setSide] = useState<'annotations' | 'info' | null>(
    initialTarget?.annotation_id ? 'annotations' : null,
  );
  const readerContent = useRef<HTMLDivElement>(null);
  const [location, setLocation] = useState<SourceLocator | null>(
    initialTarget?.locator_status === 'unavailable' ? null : (initialTarget?.locator ?? null),
  );
  const [selection, setSelection] = useState<SourceLocator | null>(null);
  const [selectionParts, setSelectionParts] = useState<SourceLocator[]>([]);
  const [savingHighlights, setSavingHighlights] = useState(false);
  const pendingHighlights = useRef<Array<{ id: string; locator: SourceLocator; comment: string }>>(
    [],
  );
  const commentInput = useRef<HTMLInputElement>(null);
  const [comment, setComment] = useState('');
  const [title, setTitle] = useState('');
  const titleDirty = useRef(false);
  const currentTitle = useRef(title);
  currentTitle.current = title;
  const [tag, setTag] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(
    initialTarget?.locator_status === 'unavailable'
      ? t(
          '原定位不可用；仅打开保存的原版本。',
          'Original location is unavailable; showing the saved version.',
        )
      : initialTarget?.locator_status === 'approximate'
        ? !initialTarget.annotation_id &&
          initialTarget.locator?.kind === 'pdf' &&
          !initialTarget.locator.rects.length
          ? t(
              '已定位资料页，未标记具体文字。',
              'Document page located; no specific text is marked.',
            )
          : t(
              '已打开原版本，位置尚未精确核验。',
              'Original version opened; location is approximate.',
            )
        : null,
  );
  const [needsOcr, setNeedsOcr] = useState(false);
  const [pdfParsedVersion, setPdfParsedVersion] = useState<string | null>(null);
  const latestPdfPosition = useRef<PdfReadingPosition | null>(null);
  const pdfPositionReader = useRef<(() => PdfReadingPosition | null) | null>(null);
  const canRecordPdf = useRef(false);
  canRecordPdf.current = !!source && !source.trashed;
  const parsedVersionRef = useRef(pdfParsedVersion);
  parsedVersionRef.current = pdfParsedVersion;
  const [refresh, setRefresh] = useState(0);
  const [listBusy, setListBusy] = useState(false);
  const [statusBusy, setStatusBusy] = useState(false);
  const preparingStatus = useRef(false);
  const firstLoad = useRef(true);
  const readingOwner = useRef(crypto.randomUUID());
  const readingQueue = useMemo(
    () => sourceReading(identity, sourceId),
    [identity.local_instance_id, identity.workspace_id, sourceId],
  );
  useEffect(() => {
    if (selectedVersion)
      readingQueue.register(readingOwner.current, selectedVersion, () => {
        const current = canRecordPdf.current ? pdfPositionReader.current?.() : null;
        return current
          ? {
              ...current,
              locatorReady: current.locator.source_version_id === parsedVersionRef.current,
            }
          : null;
      });
    return () => readingQueue.release(readingOwner.current);
  }, [readingQueue, selectedVersion]);
  useEffect(() => {
    if (active && selectedVersion) readingQueue.activate(readingOwner.current, selectedVersion);
  }, [active, selectedVersion, readingQueue]);
  const queryScope = JSON.stringify([selectedVersion, annotationTrash]);
  useEffect(() => {
    setSelectedAnnotationIds(new Set());
  }, [queryScope, actions.generation, refresh]);
  useEffect(() => {
    if (
      side !== 'annotations' ||
      annotationSearchScope !== 'library' ||
      annotationSearchComposing.current
    )
      return;
    const query = annotationQuery.trim();
    const sequence = ++libraryAnnotationSequence.current;
    setLibraryAnnotationError(null);
    setLibraryAnnotationCursor(null);
    setLibraryAnnotationSearched(false);
    if (!query) {
      setLibraryAnnotationHits([]);
      setLibraryAnnotationBusy(false);
      return;
    }
    setLibraryAnnotationBusy(true);
    const timer = window.setTimeout(() => {
      void call(
        {
          kind: 'annotations_query',
          payload: { limit: 30, cursor: null, query },
        },
        'annotation_page',
        identity,
      )
        .then((page) => {
          if (sequence !== libraryAnnotationSequence.current) return;
          setLibraryAnnotationHits(page.items);
          setLibraryAnnotationCursor(page.next_cursor);
          setLibraryAnnotationSearched(true);
        })
        .catch((failure: unknown) => {
          if (sequence === libraryAnnotationSequence.current)
            setLibraryAnnotationError(message(failure));
        })
        .finally(() => {
          if (sequence === libraryAnnotationSequence.current) setLibraryAnnotationBusy(false);
        });
    }, 220);
    return () => {
      window.clearTimeout(timer);
      if (sequence === libraryAnnotationSequence.current) libraryAnnotationSequence.current += 1;
    };
  }, [
    annotationQuery,
    annotationSearchScope,
    actions.generation,
    identity,
    annotationSearchCompositionVersion,
    libraryAnnotationRetry,
    refresh,
    side,
  ]);
  const queryScopeRef = useRef(queryScope);
  queryScopeRef.current = queryScope;
  const listScopeRef = useRef('');
  listScopeRef.current = JSON.stringify([queryScope, actions.generation, refresh]);
  const navigation = useRef(0);
  useEffect(
    () => () => {
      navigation.current++;
      queryScopeRef.current = 'unmounted';
    },
    [],
  );
  useEffect(() => {
    let live = true;
    void Promise.all([
      call({ kind: 'sources_get', payload: { source_id: sourceId } }, 'source', identity),
      call(
        { kind: 'objects_metadata_get', payload: { object_id: sourceId } },
        'object_metadata',
        identity,
      ),
      call({ kind: 'reading_get', payload: { source_id: sourceId } }, 'reading', identity),
    ])
      .then(([next, meta, state]) => {
        if (!live) return;
        setSource(next);
        setMetadata(meta);
        setReading(state);
        if (!titleDirty.current) setTitle(next.display_title);
        onTitleChange?.(sourceId, next.display_title);
        if (firstLoad.current) {
          firstLoad.current = false;
          const target = initialTarget?.source_version_id ?? versionId ?? state.source_version_id;
          setSelectedVersion(target);
          if (!initialTarget && !initialPdfView && state.locator?.source_version_id === target) {
            setLocation(state.locator);
            setTextTarget({ parseId: state.parse_id, offset: state.block_offset ?? 0 });
          }
          if (!initialPdfView && state.source_version_id === target)
            setPdfView(state.pdf_view ?? null);
        }
      })
      .catch((e) => {
        if (live) setError(message(e));
      });
    return () => {
      live = false;
    };
  }, [identity, sourceId, actions.generation, refresh]);
  useEffect(() => {
    if (actions.generation > 0) onChanged?.();
  }, [actions.generation]);
  useEffect(() => {
    let live = true;
    void call(
      { kind: 'sources_versions', payload: { source_id: sourceId, after_id: null, limit: 30 } },
      'source_versions',
      identity,
    )
      .then((result) => {
        if (live) {
          setVersions(result);
          setMoreVersions(result.length === 30);
        }
      })
      .catch((e) => {
        if (live) setError(message(e));
      });
    return () => {
      live = false;
    };
  }, [identity, sourceId, actions.generation, refresh]);
  useEffect(() => {
    if (!selectedVersion) return;
    let live = true;
    setVersion(null);
    setSelection(null);
    setSelectionParts([]);
    pendingHighlights.current = [];
    setFocusedAnnotation(
      selectedVersion === initialTarget?.source_version_id
        ? (initialTarget?.annotation_id ?? null)
        : null,
    );
    void call(
      { kind: 'sources_version', payload: { source_version_id: selectedVersion } },
      'source_version',
      identity,
    )
      .then((result) => {
        if (live) setVersion(result);
      })
      .catch((e) => {
        if (live) setError(message(e));
      });
    return () => {
      live = false;
    };
  }, [selectedVersion]);
  useEffect(() => {
    const id = initialTarget?.annotation_id;
    setReferencedAnnotation(null);
    if (!id || selectedVersion !== initialTarget?.source_version_id) return;
    let live = true;
    // A reference may point beyond the first annotation page. Resolve its stable
    // identity directly instead of depending on title matching or list order.
    void call({ kind: 'annotations_resolve', payload: { id } }, 'resolved_annotation', identity)
      .then((resolved) => {
        if (!live || resolved.annotation.source_version_id !== selectedVersion) return;
        setReferencedAnnotation(resolved.annotation);
        setAnnotationTrash(resolved.annotation.trashed);
      })
      .catch((failure) => {
        if (live) setError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [initialTarget?.annotation_id, selectedVersion]);
  useEffect(() => {
    setNeedsOcr(false);
    setPdfTextAvailable(false);
    setPdfOcrApplied(false);
    setPdfTextView(false);
    setPdfParsedVersion(null);
    if (version?.format !== 'pdf') return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () =>
      void call(
        {
          kind: 'sources_text',
          payload: { source_version_id: version.id, parse_id: null, offset: 0, limit: 1 },
        },
        'source_text',
        identity,
      )
        .then((result) => {
          if (live) {
            setNeedsOcr(result.needs_ocr);
            setPdfOcrApplied(Boolean(result.ocr_applied));
            setPdfTextAvailable(Boolean(result.ocr_applied) || result.needs_ocr === false);
            setPdfParsedVersion(version.id);
          }
        })
        .catch((e) => {
          // A saved original may be readable while its asynchronous parse is not ready.
          if (!live) return;
          if ((e as { details?: { code?: string } }).details?.code === 'not_found')
            timer = setTimeout(read, 2000);
          else setError(message(e));
        });
    read();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [version, refresh]);
  useEffect(() => {
    if (
      pdfParsedVersion &&
      latestPdfPosition.current?.locator.source_version_id === pdfParsedVersion &&
      source &&
      !source.trashed
    )
      readingQueue.record(
        readingOwner.current,
        latestPdfPosition.current.locator,
        latestPdfPosition.current.view,
      );
  }, [pdfParsedVersion]);
  useEffect(() => {
    if (!selectedVersion) return;
    let live = true;
    setAnnotations((old) =>
      old.every(
        (item) => item.source_version_id === selectedVersion && item.trashed === annotationTrash,
      )
        ? old
        : [],
    );
    setMoreAnnotations(false);
    void call(
      {
        kind: 'annotations_list',
        payload: {
          source_version_id: selectedVersion,
          after_id: null,
          limit: 30,
          trashed: annotationTrash,
        },
      },
      'annotations',
      identity,
    )
      .then((result) => {
        if (live) {
          setAnnotations(result);
          setMoreAnnotations(result.length === 30);
        }
      })
      .catch((e) => {
        if (live) setError(message(e));
      });
    return () => {
      live = false;
    };
  }, [selectedVersion, actions.generation, annotationTrash, refresh]);
  useEffect(() => {
    if (!selectedVersion || version?.format !== 'pdf') {
      setPdfAnnotations([]);
      return;
    }
    let live = true;
    setPdfAnnotations((old) =>
      old.every((item) => item.source_version_id === selectedVersion) ? old : [],
    );
    // Canvas marks must not disappear merely because the sidebar has only loaded its first page.
    void (async () => {
      let after: string | null = null;
      const marks: Annotation[] = [];
      while (live) {
        const batch: Annotation[] = await call(
          {
            kind: 'annotations_list',
            payload: {
              source_version_id: selectedVersion,
              after_id: after,
              limit: 100,
              trashed: false,
            },
          },
          'annotations',
          identity,
        );
        if (!live) return;
        marks.push(...batch);
        if (batch.length < 100) {
          setPdfAnnotations(marks);
          break;
        }
        after = batch.at(-1)!.id;
      }
    })().catch((failure) => {
      if (live) setError(message(failure));
    });
    return () => {
      live = false;
    };
  }, [selectedVersion, version?.format, actions.generation, refresh]);
  useEffect(() => {
    if (side !== 'annotations' || !focusedAnnotation) {
      appliedAnnotationFocus.current = null;
      return;
    }
    const target = annotationSidebar.current?.querySelector<HTMLElement>(
      `[data-annotation-id="${CSS.escape(focusedAnnotation)}"]`,
    );
    const sidebar = annotationSidebar.current;
    if (target && sidebar && appliedAnnotationFocus.current !== target) {
      appliedAnnotationFocus.current = target;
      const bounds = sidebar.getBoundingClientRect(),
        rect = target.getBoundingClientRect();
      sidebar.scrollTop += rect.top - bounds.top - 56;
      target.focus({ preventScroll: true });
    }
  }, [focusedAnnotation, side, annotations, pdfAnnotations, referencedAnnotation]);
  // Coalesce incidental scroll events; every actual write still uses a durable exact request.
  useEffect(() => {
    const timer = setInterval(() => {
      if (!readingQueue.needsAutoFlush() || actions.locked || source?.trashed) return;
      void readingQueue.flush().catch((e) => setError(message(e)));
    }, 1400);
    return () => clearInterval(timer);
  }, [readingQueue, actions.locked, source?.trashed]);
  function position(locator: SourceLocator, view?: PdfReadingView) {
    if (locator.kind === 'pdf' && view) latestPdfPosition.current = { locator, view };
    if (source && !source.trashed)
      readingQueue.record(
        readingOwner.current,
        locator,
        view ?? null,
        locator.kind !== 'pdf' || pdfParsedVersion === locator.source_version_id,
      );
  }
  async function mutate(command: DesktopCommand, metadata = false) {
    setError(null);
    return actions.execute(command, metadata ? 'metadata_commit' : 'source_mutation');
  }
  async function flag(
    kind: 'objects_set_archived' | 'objects_set_search_enabled' | 'sources_set_trashed',
    value: boolean,
  ) {
    if (!metadata) return;
    try {
      await readingQueue.flush();
    } catch (e) {
      setError(message(e));
      return;
    }
    await mutate(
      {
        kind,
        payload: {
          command_id: crypto.randomUUID(),
          request: { object_id: sourceId, base_row_version: metadata.row_version, value },
        },
      },
      kind !== 'sources_set_trashed',
    );
  }
  /** Exports the immutable original; the task centre tracks the accepted job. */
  async function exportOriginal() {
    if (!source || !selectedVersion) return;
    const grant = await choose('export_file', identity);
    if (!grant) return;
    await portableActions.load();
    const commandId = crypto.randomUUID();
    try {
      await portableActions.submit({
        id: commandId,
        identity,
        createdAt: new Date().toISOString(),
        displayName: grant.display_name,
        command: {
          kind: 'source_export',
          payload: {
            command_id: commandId,
            token: grant.token,
            source_id: source.id,
            version_id: selectedVersion,
          },
        },
      });
    } catch (failure) {
      setError(message(failure));
    }
  }
  function selectExcerpt(locator: SourceLocator | null, all?: SourceLocator[]) {
    if (savingHighlights) return;
    setSelection(locator);
    setSelectionParts(locator ? (all ?? [locator]) : []);
    pendingHighlights.current = [];
  }
  async function highlight(
    parts = selectionParts.length ? selectionParts : selection ? [selection] : [],
    note = comment,
  ) {
    if (!parts.length || savingHighlights || actions.locked || source?.trashed) return false;
    if (!pendingHighlights.current.length)
      pendingHighlights.current = parts.map((locator) => ({
        id: crypto.randomUUID(),
        locator,
        comment: note,
      }));
    setSavingHighlights(true);
    const total = pendingHighlights.current.length;
    try {
      while (pendingHighlights.current.length) {
        const next = pendingHighlights.current[0];
        const ok = await mutate({
          kind: 'annotations_create',
          payload: {
            command_id: next.id,
            request: { locator: next.locator, comment: next.comment },
          },
        });
        if (!ok) {
          setSelectionParts(pendingHighlights.current.map((item) => item.locator));
          setSelection(pendingHighlights.current[0].locator);
          setNotice(
            t(
              `已确认 ${total - pendingHighlights.current.length} 条，剩余 ${pendingHighlights.current.length} 条摘录尚未确认。请先核对原请求后继续保存。`,
              `${total - pendingHighlights.current.length} highlights confirmed; ${pendingHighlights.current.length} remain unconfirmed. Check the original request before continuing.`,
            ),
          );
          return false;
        }
        pendingHighlights.current.shift();
      }
      setSelection(null);
      setSelectionParts([]);
      setComment('');
      setSide('annotations');
      return true;
    } finally {
      setSavingHighlights(false);
    }
  }
  async function selectedAction(parts: SourceLocator[], mode: 'highlight' | 'comment') {
    if (mode === 'highlight') return highlight(parts, '');
    setSelection(parts[0]);
    setSelectionParts(parts);
    requestAnimationFrame(() => commentInput.current?.focus());
    return true;
  }
  function openAnnotation(annotation: Annotation) {
    setAnnotationTrash(false);
    setAnnotations((old) =>
      old.some((item) => item.id === annotation.id) ? old : [annotation, ...old],
    );
    setFocusedAnnotation(annotation.id);
    setSide('annotations');
  }
  function annotationListKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    const target = event.target as HTMLElement;
    if (target.closest('input, textarea, select, button, [contenteditable="true"]')) return;
    const card = target.closest<HTMLElement>('[data-annotation-card]');
    if (!card) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      setFocusedAnnotation(null);
      return;
    }
    if (event.key === 'Enter') {
      const annotation = filteredAnnotations.find((item) => item.id === card.dataset.annotationId);
      if (annotation) {
        event.preventDefault();
        event.stopPropagation();
        void jump(annotation);
      }
      return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const cards = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>('[data-annotation-card]'),
    );
    const index = cards.indexOf(card);
    if (index < 0) return;
    const next = cards[index + (event.key === 'ArrowDown' ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    event.stopPropagation();
    next.focus();
  }
  async function loadMoreLibraryAnnotations() {
    if (libraryAnnotationBusy || !libraryAnnotationCursor) return;
    const sequence = libraryAnnotationSequence.current;
    const cursor = libraryAnnotationCursor;
    setLibraryAnnotationBusy(true);
    setLibraryAnnotationError(null);
    try {
      const page = await call(
        {
          kind: 'annotations_query',
          payload: { limit: 30, cursor, query: annotationQuery.trim() },
        },
        'annotation_page',
        identity,
      );
      if (sequence !== libraryAnnotationSequence.current) return;
      setLibraryAnnotationHits((current) => {
        const seen = new Set(current.map((hit) => hit.id));
        return [...current, ...page.items.filter((hit) => !seen.has(hit.id))];
      });
      setLibraryAnnotationCursor(page.next_cursor);
      setLibraryAnnotationSearched(true);
    } catch (failure) {
      if (sequence === libraryAnnotationSequence.current)
        setLibraryAnnotationError(message(failure));
    } finally {
      if (sequence === libraryAnnotationSequence.current) setLibraryAnnotationBusy(false);
    }
  }
  async function openLibraryAnnotation(hit: AnnotationSummary) {
    setOpeningLibraryAnnotation(hit.id);
    setLibraryAnnotationError(null);
    try {
      if (hit.source_id === sourceId && hit.source_version_id === selectedVersion) {
        const annotation = await call(
          { kind: 'annotations_get', payload: { id: hit.id } },
          'annotation',
          identity,
        );
        setAnnotationTrash(false);
        openAnnotation(annotation);
        await jump(annotation);
      } else if (onOpenSource) {
        onOpenSource(hit.source_id, hit.source_version_id, hit.id);
      } else {
        setNotice(
          t(
            '该摘录位于另一份资料。请从资料库打开后再定位。',
            'This highlight belongs to another document. Open it from the library to locate it.',
          ),
        );
      }
    } catch (failure) {
      setLibraryAnnotationError(message(failure));
    } finally {
      setOpeningLibraryAnnotation(null);
    }
  }
  async function trashAnnotation(annotation: Annotation) {
    const ok = await mutate({
      kind: 'annotations_set_trashed',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          object_id: annotation.id,
          base_row_version: annotation.row_version,
          value: !annotation.trashed,
        },
      },
    });
    if (!ok) return;
    if (referencedAnnotation?.id === annotation.id) setReferencedAnnotation(null);
    if (!annotation.trashed && focusedAnnotation === annotation.id) {
      setFocusedAnnotation(null);
      setLocation(null);
    }
  }
  async function jump(annotation: Annotation) {
    const request = ++navigation.current;
    const scope = queryScopeRef.current;
    try {
      await readingQueue.flush();
      const resolved = await call(
        { kind: 'annotations_resolve', payload: { id: annotation.id } },
        'resolved_annotation',
        identity,
      );
      if (request !== navigation.current || scope !== queryScopeRef.current) return;
      if (resolved.status === 'unavailable') {
        setNotice(
          t(
            '原位置当前不可用，摘录仍已保留。',
            'The original location is unavailable. The quotation is retained.',
          ),
        );
        return;
      }
      setNotice(
        resolved.status === 'approximate'
          ? t(
              '已打开原版本页；选区位置尚未通过解析几何核验。',
              'Original version page opened; selection geometry is not verified.',
            )
          : null,
      );
      setTextTarget({ parseId: resolved.parse_id, offset: resolved.block_offset ?? 0 });
      setSelectedVersion(annotation.source_version_id);
      setLocation({ ...annotation.locator });
      setFocusedAnnotation(annotation.id);
      // Both a narrow reader container and a split pane can replace the original
      // with its sidebar. Follow the rendered state rather than the window width.
      if (readerContent.current && getComputedStyle(readerContent.current).display === 'none') {
        setSide(null);
      }
    } catch (e) {
      if (request === navigation.current && scope === queryScopeRef.current) setError(message(e));
    }
  }
  async function changeVersion(id: string) {
    const request = ++navigation.current;
    try {
      await readingQueue.flush();
      if (request !== navigation.current) return;
      setSelectedVersion(id);
      setTextTarget({ parseId: null, offset: 0 });
      setLocation(null);
      setPdfView(null);
    } catch (e) {
      if (request === navigation.current) setError(message(e));
    }
  }
  async function changeReadingStatus(status: ReadingState['status']) {
    if (!selectedVersion || preparingStatus.current) return;
    preparingStatus.current = true;
    setStatusBusy(true);
    const request = navigation.current;
    try {
      await readingQueue.flush();
      const current = await call(
        { kind: 'reading_get', payload: { source_id: sourceId } },
        'reading',
        identity,
      );
      if (request !== navigation.current) return;
      await mutate({
        kind: 'reading_set',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            source_id: sourceId,
            source_version_id: selectedVersion,
            locator: current.source_version_id === selectedVersion ? current.locator : null,
            ...(current.source_version_id === selectedVersion && current.pdf_view
              ? { pdf_view: current.pdf_view }
              : {}),
            status,
            base_row_version: current.row_version,
          },
        },
      });
    } catch (e) {
      setError(message(e));
    } finally {
      preparingStatus.current = false;
      setStatusBusy(false);
    }
  }
  async function loadMore(kind: 'versions' | 'annotations') {
    if (listBusy || !selectedVersion) return;
    const scope = listScopeRef.current;
    setListBusy(true);
    try {
      if (kind === 'versions') {
        const result = await call(
          {
            kind: 'sources_versions',
            payload: { source_id: sourceId, after_id: versions.at(-1)?.id ?? null, limit: 30 },
          },
          'source_versions',
          identity,
        );
        if (scope === listScopeRef.current) {
          setVersions((old) => [...old, ...result]);
          setMoreVersions(result.length === 30);
        }
      } else {
        const result = await call(
          {
            kind: 'annotations_list',
            payload: {
              source_version_id: selectedVersion,
              after_id: annotations.at(-1)?.id ?? null,
              limit: 30,
              trashed: annotationTrash,
            },
          },
          'annotations',
          identity,
        );
        if (scope === listScopeRef.current) {
          setAnnotations((old) => [...old, ...result]);
          setMoreAnnotations(result.length === 30);
        }
      }
    } catch (e) {
      if (scope === listScopeRef.current) setError(message(e));
    } finally {
      setListBusy(false);
    }
  }
  async function addVersion() {
    if (!source) return;
    try {
      const grant = await choose('import_source', identity);
      if (!grant) return;
      const ext = grant.display_name.split('.').pop()?.toLowerCase();
      const accepted = await actions.execute(
        {
          kind: 'sources_import',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              input_token: grant.token,
              original_name: grant.display_name,
              format:
                ext === 'pdf' ? 'pdf' : ext === 'md' || ext === 'markdown' ? 'markdown' : 'text',
              origin_url: null,
              collection_id: null,
              add_version_to: source.id,
              base_row_version: source.row_version,
            },
            tag_names: [],
          },
        },
        'job_accepted',
      );
      if (accepted)
        setNotice(
          t(
            '导入已受理；请在资料库查看实际处理结果。',
            'Import accepted. Check the library for the processing result.',
          ),
        );
    } catch (e) {
      setError(message(e));
    }
  }
  const pdfModeSwitch = (
    <div
      className="view-switch source-view-switch source-pdf-view-switch"
      role="group"
      aria-label={t('PDF 阅读方式', 'PDF reading mode')}
    >
      <button
        type="button"
        className="secondary-button"
        aria-pressed={!pdfTextView}
        data-short={t('原页', 'Page')}
        onClick={() => setPdfTextView(false)}
      >
        {t('原页', 'Original page')}
      </button>
      <button
        type="button"
        className="secondary-button"
        aria-pressed={pdfTextView}
        data-short={t('文字', 'Text')}
        disabled={!pdfTextAvailable}
        title={
          pdfTextAvailable
            ? undefined
            : t('文字层仍在准备中', 'The text layer is still being prepared')
        }
        onClick={() => setPdfTextView(true)}
      >
        {pdfOcrApplied ? t('OCR 文字', 'OCR text') : t('文字视图', 'Text view')}
      </button>
    </div>
  );
  return (
    <PageFrame
      className={`source-reader${hideSidebar ? ' source-reader-compact' : ''}`}
      title={source?.display_title ?? t('正在打开…', 'Opening…')}
      description={
        version
          ? `${version.format.toUpperCase()} · ${new Date(version.captured_at).toLocaleString()}`
          : t('不可变原件', 'Immutable original')
      }
      actions={
        <div className="source-reader-tools">
          {onClose && (
            <button className="icon-button" onClick={onClose} aria-label={t('返回', 'Back')}>
              <ArrowLeft size={17} />
            </button>
          )}
          <button
            className="icon-button"
            onClick={() => setSide(side === 'annotations' ? null : 'annotations')}
            aria-label={t('摘录与批注', 'Highlights and notes')}
            aria-pressed={side === 'annotations'}
          >
            <Highlighter size={17} />
          </button>
          {selectedVersion && (
            <AddToReview
              identity={identity}
              target={{
                target_id: sourceId,
                target_revision_id: selectedVersion,
                target_block_id: null,
              }}
              disabled={!!source?.trashed || !source}
            />
          )}
          <button
            className="icon-button"
            onClick={() => setSide(side === 'info' ? null : 'info')}
            aria-label={t('来源信息', 'Document information')}
            aria-pressed={side === 'info'}
          >
            <Info size={17} />
          </button>
          <PopupMenu
            label={t('资料操作', 'Document actions')}
            trigger={<MoreHorizontal size={17} />}
            disabled={!metadata || actions.locked}
            placement="bottom-start"
            items={[
              {
                id: 'knowledge-task',
                label: t('基于此资料发起任务', 'Start a task with this source'),
                disabled: !taskNavigation || !!source?.trashed || !selectedVersion,
                onSelect: () =>
                  taskNavigation?.start([
                    { object_id: sourceId, content_revision: selectedVersion ?? null },
                  ]),
              },
              {
                id: 'archive',
                label: metadata?.archived ? t('取消归档', 'Unarchive') : t('归档', 'Archive'),
                disabled: source?.trashed,
                onSelect: () => flag('objects_set_archived', !metadata?.archived),
              },
              {
                id: 'search',
                label: metadata?.search_enabled
                  ? t('停用检索', 'Disable search')
                  : t('启用检索', 'Enable search'),
                disabled: source?.trashed,
                onSelect: () => flag('objects_set_search_enabled', !metadata?.search_enabled),
              },
              {
                id: 'export',
                label: t('导出原件', 'Export original'),
                disabled: source?.trashed || !selectedVersion,
                onSelect: () => void exportOriginal(),
              },
              {
                id: 'version',
                label: t('导入新版本', 'Import new version'),
                disabled: source?.trashed,
                onSelect: () => void addVersion(),
              },
              {
                id: 'trash',
                label: source?.trashed
                  ? t('恢复资料', 'Restore document')
                  : t('移到回收站', 'Move to trash'),
                separatorBefore: true,
                onSelect: () => flag('sources_set_trashed', !source?.trashed),
              },
            ]}
          />
        </div>
      }
      toolbar={version?.format === 'pdf' && pdfTextView ? pdfModeSwitch : undefined}
      onPointerDownCapture={() => {
        if (selectedVersion) readingQueue.activate(readingOwner.current, selectedVersion);
      }}
      onFocusCapture={() => {
        if (selectedVersion) readingQueue.activate(readingOwner.current, selectedVersion);
      }}
      onWheelCapture={() => {
        if (selectedVersion) readingQueue.activate(readingOwner.current, selectedVersion);
      }}
    >
      {(error || actions.error) && (
        <div className="source-notice" role="alert">
          {error || actions.error}
          {!actions.ready && actions.error && (
            <button onClick={() => void actions.retry()}>
              {t('重试本机操作记录', 'Retry local operation storage')}
            </button>
          )}
          <button
            onClick={() => {
              setError(null);
              setRefresh((n) => n + 1);
            }}
          >
            {t('刷新', 'Refresh')}
          </button>
        </div>
      )}
      {actions.unconfirmed && (
        <div className="source-notice">
          {t(
            '保存结果待确认；原请求已保留。',
            'Save result is unconfirmed. Original request retained.',
          )}
          <button disabled={actions.busy} onClick={() => void actions.retry()}>
            {t('核对原请求', 'Check request')}
          </button>
        </div>
      )}
      {notice && (
        <div className="source-notice" role="status">
          {notice}
          <button onClick={() => setNotice(null)} aria-label={t('关闭提示', 'Dismiss')}>
            ×
          </button>
        </div>
      )}
      {source?.trashed && (
        <div className="source-notice">
          {t(
            '此资料在回收站中。恢复后可继续摘录。',
            'This document is in Trash. Restore it to continue highlighting.',
          )}
        </div>
      )}
      {needsOcr && (
        <div className="source-notice">
          {t(
            'PDF 中有页面没有可提取文字；原页仍可阅读或框选摘录。',
            'Some PDF pages have no extractable text. You can still read those pages or select a region.',
          )}
        </div>
      )}
      {version?.format === 'pdf' && pdfParsedVersion !== version.id && (
        <div className="source-pdf-hint" role="status">
          {t(
            '原件可直接阅读，阅读位置照常保存；解析完成后可以保存摘录和旁注。',
            'Read the original now; your position is saved. Highlights and notes become available after processing.',
          )}
        </div>
      )}
      <div className="source-reader-body">
        <div className="source-reader-content" ref={readerContent}>
          {version && (version.format === 'text' || version.format === 'markdown') && (
            <SourceNoteImport
              key={version.id}
              identity={identity}
              version={version}
              disabled={!source || source.trashed}
              onChanged={onChanged}
            />
          )}
          {version &&
            (version.format === 'pdf' ? (
              <div className="source-pdf-mode-shell">
                {pdfTextView ? (
                  <TextReader
                    key={`${version.id}/${textTarget.parseId}/${textTarget.offset}/text-view`}
                    identity={identity}
                    version={version}
                    title={source?.display_title ?? version.original_name}
                    author={source?.author ?? null}
                    initialMode="source"
                    parseId={textTarget.parseId}
                    initialOffset={textTarget.offset}
                    location={location}
                    strictLocation={Boolean(initialTarget)}
                    onSelection={selectExcerpt}
                    onPosition={position}
                  />
                ) : (
                  <Suspense fallback={<p role="status">{t('加载阅读器…', 'Loading reader…')}</p>}>
                    <PdfReader
                      identity={identity}
                      versionId={version.id}
                      location={location}
                      initialView={pdfView}
                      positionReader={pdfPositionReader}
                      onSelection={selectExcerpt}
                      onSelectionAction={selectedAction}
                      selectionActive={!!selection}
                      annotationBusy={
                        actions.locked ||
                        savingHighlights ||
                        !!source?.trashed ||
                        pdfParsedVersion !== version.id
                      }
                      annotations={pdfAnnotations}
                      selectedAnnotationId={focusedAnnotation}
                      onAnnotationClick={openAnnotation}
                      onPosition={position}
                      startControls={pdfModeSwitch}
                    />
                  </Suspense>
                )}
              </div>
            ) : (
              <TextReader
                key={`${version.id}/${textTarget.parseId}/${textTarget.offset}`}
                identity={identity}
                version={version}
                title={source?.display_title ?? version.original_name}
                author={source?.author ?? null}
                initialMode={
                  version.format === 'markdown' ||
                  version.format === 'docx' ||
                  version.format === 'doc'
                    ? !initialTarget
                      ? 'rendered'
                      : 'source'
                    : 'source'
                }
                parseId={textTarget.parseId}
                initialOffset={textTarget.offset}
                location={location}
                strictLocation={Boolean(initialTarget)}
                onSelection={selectExcerpt}
                onPosition={position}
              />
            ))}
          {selection && (
            <div className="source-selection">
              <p title={selection.quote}>{selection.quote}</p>
              <input
                ref={commentInput}
                aria-label={t('批注', 'Note')}
                placeholder={t('写下理解（可选）', 'Add a note (optional)')}
                value={comment}
                disabled={savingHighlights || pendingHighlights.current.length > 0}
                onChange={(e) => setComment(e.target.value)}
              />
              <div className="source-selection-actions">
                <button
                  disabled={
                    actions.locked ||
                    savingHighlights ||
                    source?.trashed ||
                    (selection.kind === 'pdf' && pdfParsedVersion !== selection.source_version_id)
                  }
                  onClick={() => void highlight()}
                >
                  {savingHighlights
                    ? t('保存中…', 'Saving…')
                    : selectionParts.length > 1
                      ? t(
                          `保存 ${selectionParts.length} 页摘录`,
                          `Save ${selectionParts.length} page highlights`,
                        )
                      : t('保存摘录', 'Save highlight')}
                </button>
                <button
                  className="icon-button"
                  disabled={savingHighlights}
                  onClick={() => selectExcerpt(null)}
                  aria-label={t('取消选区', 'Dismiss selection')}
                >
                  <X size={15} />
                </button>
              </div>
            </div>
          )}
        </div>
        {side && (
          <aside className="source-sidebar" ref={annotationSidebar}>
            <div className="source-tools">
              <h2>
                {side === 'info'
                  ? t('来源信息', 'Document information')
                  : t('摘录与批注', 'Highlights and notes')}
              </h2>
              <button
                className="icon-button"
                onClick={() => setSide(null)}
                aria-label={t('回到阅读', 'Return to reading')}
              >
                <X size={15} />
              </button>
            </div>
            {side === 'info' && source && metadata && (
              <>
                <label>
                  {t('标题', 'Title')}
                  <input
                    value={title}
                    onChange={(e) => {
                      titleDirty.current = true;
                      setTitle(e.target.value);
                    }}
                  />
                </label>
                <button
                  disabled={
                    actions.locked ||
                    !title.trim() ||
                    title === source.display_title ||
                    source.trashed
                  }
                  onClick={() => {
                    const submitted = title;
                    void mutate({
                      kind: 'sources_update',
                      payload: {
                        command_id: crypto.randomUUID(),
                        request: {
                          source_id: sourceId,
                          base_row_version: source.row_version,
                          display_title: submitted,
                        },
                      },
                    }).then((ok) => {
                      if (ok && currentTitle.current === submitted) titleDirty.current = false;
                    });
                  }}
                >
                  {t('保存标题', 'Save title')}
                </button>
                <div className="choice-field">
                  {t('阅读状态', 'Reading status')}
                  <Select
                    aria-label={t('阅读状态', 'Reading status')}
                    disabled={actions.locked || statusBusy || !reading || source.trashed}
                    value={reading?.status ?? 'unread'}
                    onValueChange={(value) =>
                      void changeReadingStatus(value as ReadingState['status'])
                    }
                  >
                    <option value="unread">{t('未读', 'Unread')}</option>
                    <option value="reading">{t('阅读中', 'Reading')}</option>
                    <option value="finished">{t('已读', 'Finished')}</option>
                  </Select>
                </div>
                <div className="choice-field">
                  {t('整理状态', 'Location')}
                  <Select
                    aria-label={t('整理状态', 'Location')}
                    disabled={actions.locked || source.trashed}
                    value={metadata.inbox_state ?? 'pending'}
                    onValueChange={(value) =>
                      void mutate(
                        {
                          kind: 'objects_set_inbox_state',
                          payload: {
                            command_id: crypto.randomUUID(),
                            request: {
                              object_id: sourceId,
                              base_row_version: metadata.row_version,
                              target: value as InboxState,
                            },
                          },
                        },
                        true,
                      )
                    }
                  >
                    <option value="pending">{t('收件箱', 'Inbox')}</option>
                    <option value="snoozed">{t('稍后', 'Later')}</option>
                    <option value="processed">{t('已整理', 'Organized')}</option>
                  </Select>
                </div>
                <label>
                  {t('标签', 'Tags')}
                  <input
                    value={tag}
                    onChange={(e) => setTag(e.target.value)}
                    placeholder={t('输入标签名称', 'Tag name')}
                  />
                </label>
                <button
                  disabled={actions.locked || !tag.trim() || source.trashed}
                  onClick={() =>
                    void mutate(
                      {
                        kind: 'tags_assign',
                        payload: {
                          command_id: crypto.randomUUID(),
                          request: {
                            targets: [
                              { object_id: sourceId, base_row_version: metadata.row_version },
                            ],
                            tag: { kind: 'name', name: tag.trim() },
                          },
                        },
                      },
                      true,
                    ).then((ok) => {
                      if (ok) setTag('');
                    })
                  }
                >
                  {t('添加标签', 'Add tag')}
                </button>
                <div className="source-tools">
                  {metadata.tags.map((item) => (
                    <button
                      key={item.id}
                      className={`source-tag-chip tag-inline ${tagToneClass(item.id)}`}
                      disabled={actions.locked || source.trashed}
                      aria-label={`${t('移除标签', 'Remove tag')} ${item.display_name}`}
                      onClick={() =>
                        void mutate(
                          {
                            kind: 'tags_remove',
                            payload: {
                              command_id: crypto.randomUUID(),
                              request: {
                                targets: [
                                  { object_id: sourceId, base_row_version: metadata.row_version },
                                ],
                                tag_id: item.id,
                              },
                            },
                          },
                          true,
                        )
                      }
                    >
                      <span>{item.display_name}</span>
                      <X size={11} aria-hidden="true" />
                    </button>
                  ))}
                </div>
                <div className="choice-field">
                  {t('版本', 'Version')}
                  <Select
                    aria-label={t('版本', 'Version')}
                    value={selectedVersion ?? ''}
                    onValueChange={(value) => void changeVersion(value)}
                  >
                    {version && !versions.some((v) => v.id === version.id) && (
                      <option value={version.id}>{version.original_name}</option>
                    )}
                    {versions.map((v) => (
                      <option key={v.id} value={v.id}>
                        {new Date(v.captured_at).toLocaleString()} · {v.original_name}
                      </option>
                    ))}
                  </Select>
                </div>
                {moreVersions && (
                  <button disabled={listBusy} onClick={() => void loadMore('versions')}>
                    {t('更早版本', 'Earlier versions')}
                  </button>
                )}
                <dl>
                  <dt>{t('原文件', 'Original file')}</dt>
                  <dd>{version?.original_name}</dd>
                  <dt>{t('作者', 'Author')}</dt>
                  <dd>{source.author ?? t('未提供', 'Not provided')}</dd>
                  <dt>{t('大小', 'Size')}</dt>
                  <dd>{version ? formatSourceSize(version.size) : '—'}</dd>
                  {version?.origin_url && (
                    <>
                      <dt>URL</dt>
                      <dd>{version.origin_url}</dd>
                    </>
                  )}
                  <dt>{t('解析', 'Processing')}</dt>
                  <dd>
                    {source.text_ready
                      ? t('正文已解析', 'Text parsed')
                      : t('正文尚未就绪', 'Text not ready')}
                  </dd>
                  <dt>{t('检索', 'Search')}</dt>
                  <dd>
                    {metadata.search_enabled
                      ? source.lexical_ready
                        ? t('关键词索引已就绪', 'Keyword index ready')
                        : t('关键词索引尚未就绪', 'Keyword index not ready')
                      : t('已停用', 'Disabled')}
                  </dd>
                </dl>
                <ReferencePanel
                  noteId={sourceId}
                  identity={identity}
                  generation={actions.generation + refresh}
                  label={t('引用了此资料', 'Links to this document')}
                />
              </>
            )}
            {side === 'annotations' && (
              <>
                {annotationSearchScope === 'loaded' && (
                  <div className="choice-field">
                    {t('显示', 'Show')}
                    <Select
                      aria-label={t('显示摘录', 'Show highlights')}
                      value={annotationTrash ? 'trash' : 'active'}
                      onValueChange={(value) => setAnnotationTrash(value === 'trash')}
                    >
                      <option value="active">{t('摘录', 'Highlights')}</option>
                      <option value="trash">{t('已删除摘录', 'Deleted highlights')}</option>
                    </Select>
                  </div>
                )}
                <div
                  className="annotation-search-scope"
                  role="group"
                  aria-label={t('摘录搜索范围', 'Highlight search scope')}
                >
                  <button
                    type="button"
                    aria-pressed={annotationSearchScope === 'loaded'}
                    onClick={() => setAnnotationSearchScope('loaded')}
                  >
                    {t('当前资料', 'This document')}
                  </button>
                  <button
                    type="button"
                    aria-pressed={annotationSearchScope === 'library'}
                    onClick={() => {
                      setAnnotationTrash(false);
                      setAnnotationSearchScope('library');
                      setSelectedAnnotationIds(new Set());
                    }}
                  >
                    {t('全库', 'Whole library')}
                  </button>
                </div>
                <label className="annotation-search">
                  <span className="sr-only">
                    {annotationSearchScope === 'library'
                      ? t('搜索全库摘录', 'Search highlights across the workspace')
                      : t('搜索已加载摘录', 'Search loaded highlights')}
                  </span>
                  <Search size={14} aria-hidden="true" />
                  <input
                    aria-label={
                      annotationSearchScope === 'library'
                        ? t('搜索全库摘录', 'Search highlights across the workspace')
                        : t('搜索已加载摘录', 'Search loaded highlights')
                    }
                    placeholder={
                      annotationSearchScope === 'library'
                        ? t('搜索资料标题、引文或批注', 'Search document titles, quotes, or notes')
                        : t('搜索引文、批注或页码', 'Search quotes, notes, or pages')
                    }
                    value={annotationQuery}
                    onChange={(event) => {
                      setAnnotationQuery(event.target.value);
                      setSelectedAnnotationIds(new Set());
                    }}
                    onCompositionStart={() => {
                      annotationSearchComposing.current = true;
                    }}
                    onCompositionEnd={() => {
                      annotationSearchComposing.current = false;
                      setAnnotationSearchCompositionVersion((value) => value + 1);
                    }}
                  />
                  {annotationQuery && (
                    <button
                      type="button"
                      aria-label={t('清除摘录搜索', 'Clear highlight search')}
                      onClick={() => {
                        setAnnotationQuery('');
                        setSelectedAnnotationIds(new Set());
                      }}
                    >
                      <X size={13} />
                    </button>
                  )}
                </label>
                {annotationSearchScope === 'loaded' ? (
                  <>
                    {annotationQuery && (
                      <p className="annotation-search-status" role="status">
                        {t(
                          `已加载摘录中匹配 ${filteredAnnotations.length} / ${displayedAnnotations.length} 条`,
                          `${filteredAnnotations.length} of ${displayedAnnotations.length} loaded highlights match`,
                        )}
                      </p>
                    )}
                    {!displayedAnnotations.length && !annotationQuery && (
                      <p>
                        {t(
                          '选中原文，保存值得留下的内容。',
                          'Select original text to keep a highlight.',
                        )}
                      </p>
                    )}
                    {!filteredAnnotations.length && annotationQuery && (
                      <p>
                        {t(
                          '没有已加载摘录匹配。可加载更多摘录后继续搜索。',
                          'No loaded highlights match. Load more highlights and search again.',
                        )}
                      </p>
                    )}
                    {onAddToNote && !annotationTrash && filteredAnnotations.length > 0 && (
                      <div className="annotation-batch-tools">
                        <p role="status">
                          {t('已选摘录', 'Selected excerpts')}: {selectedAnnotations.length} /{' '}
                          {MAX_ANNOTATIONS_TO_NOTE}
                        </p>
                        <div className="source-tools">
                          <button
                            disabled={actions.locked || !!source?.trashed}
                            onClick={() =>
                              setSelectedAnnotationIds(
                                new Set(
                                  displayedAnnotations
                                    .filter((item) => filteredAnnotations.includes(item))
                                    .filter((item) => !item.trashed)
                                    .slice(0, MAX_ANNOTATIONS_TO_NOTE)
                                    .map((item) => item.id),
                                ),
                              )
                            }
                          >
                            {t('选择已加载摘录（最多100条）', 'Select loaded excerpts (up to 100)')}
                          </button>
                          <button
                            disabled={!selectedAnnotations.length}
                            onClick={() => setSelectedAnnotationIds(new Set())}
                          >
                            {t('取消选择', 'Clear selection')}
                          </button>
                          <button
                            disabled={
                              !selectedAnnotations.length || actions.locked || !!source?.trashed
                            }
                            onClick={() => onAddToNote(selectedAnnotations)}
                          >
                            {t('将所选摘录加入同一笔记', 'Add selected excerpts to one note')}
                          </button>
                        </div>
                      </div>
                    )}
                    <div className="annotation-list" role="list" onKeyDown={annotationListKeyDown}>
                      {filteredAnnotations.map((annotation) => (
                        <AnnotationCard
                          key={`${annotation.id}/${annotation.row_version}`}
                          annotation={annotation}
                          focused={focusedAnnotation === annotation.id}
                          identity={identity}
                          locked={actions.locked || !!source?.trashed}
                          onJump={() => void jump(annotation)}
                          onAdd={onAddToNote ? () => onAddToNote([annotation]) : undefined}
                          selected={selectedAnnotationIds.has(annotation.id)}
                          selectDisabled={
                            actions.locked ||
                            !!source?.trashed ||
                            (!selectedAnnotationIds.has(annotation.id) &&
                              selectedAnnotations.length >= MAX_ANNOTATIONS_TO_NOTE)
                          }
                          onSelect={
                            onAddToNote && !annotationTrash
                              ? (checked) =>
                                  setSelectedAnnotationIds((current) => {
                                    const next = new Set(current);
                                    if (checked && next.size < MAX_ANNOTATIONS_TO_NOTE)
                                      next.add(annotation.id);
                                    else if (!checked) next.delete(annotation.id);
                                    return next;
                                  })
                              : undefined
                          }
                          onSave={(value) =>
                            mutate({
                              kind: 'annotations_update',
                              payload: {
                                command_id: crypto.randomUUID(),
                                request: {
                                  id: annotation.id,
                                  base_row_version: annotation.row_version,
                                  comment: value,
                                },
                              },
                            })
                          }
                          onTrash={() => void trashAnnotation(annotation)}
                        />
                      ))}
                    </div>
                    {moreAnnotations && (version?.format !== 'pdf' || annotationTrash) && (
                      <button disabled={listBusy} onClick={() => void loadMore('annotations')}>
                        {t('更多摘录', 'More highlights')}
                      </button>
                    )}
                    {focusedAnnotation && (
                      <ReferencePanel
                        noteId={focusedAnnotation}
                        identity={identity}
                        generation={actions.generation + refresh}
                        label={t('引用了此摘录', 'Links to this highlight')}
                      />
                    )}
                  </>
                ) : (
                  <div
                    className="annotation-library-results"
                    aria-live="polite"
                    aria-busy={libraryAnnotationBusy}
                  >
                    {!annotationQuery.trim() && (
                      <p>
                        {t(
                          '输入关键词，搜索整个知识库的摘录、旁注和资料标题。',
                          'Enter a term to search highlights, notes, and document titles across the workspace.',
                        )}
                      </p>
                    )}
                    {libraryAnnotationBusy && !libraryAnnotationHits.length && (
                      <p className="annotation-library-busy" role="status">
                        <LoaderCircle className="spin" size={15} aria-hidden="true" />
                        {t('正在搜索全库摘录…', 'Searching highlights across the workspace…')}
                      </p>
                    )}
                    {libraryAnnotationError && (
                      <div className="annotation-library-error" role="alert">
                        <p>{libraryAnnotationError}</p>
                        <button
                          type="button"
                          onClick={() => setLibraryAnnotationRetry((value) => value + 1)}
                        >
                          {t('重试搜索', 'Retry search')}
                        </button>
                      </div>
                    )}
                    {annotationQuery.trim() &&
                      !libraryAnnotationBusy &&
                      libraryAnnotationSearched &&
                      !libraryAnnotationError &&
                      !libraryAnnotationHits.length && (
                        <p>
                          {t(
                            '全库没有匹配的摘录。',
                            'No highlights across the workspace match this search.',
                          )}
                        </p>
                      )}
                    {libraryAnnotationHits.length > 0 && (
                      <p className="annotation-search-status" role="status">
                        {t(
                          `全库匹配 ${libraryAnnotationHits.length} 条${libraryAnnotationCursor ? '，可继续加载' : ''}`,
                          `${libraryAnnotationHits.length} workspace highlight${libraryAnnotationHits.length === 1 ? '' : 's'} matched${libraryAnnotationCursor ? ', more available' : ''}`,
                        )}
                      </p>
                    )}
                    {libraryAnnotationHits.map((hit) => (
                      <article className="annotation-library-result" key={hit.id}>
                        <small className="annotation-library-source">{hit.source_title}</small>
                        <blockquote>{hit.quote_preview}</blockquote>
                        {hit.comment_preview && <p>{hit.comment_preview}</p>}
                        <div className="annotation-library-result-footer">
                          {hit.page_index !== null && (
                            <span>
                              {t('页', 'Page')} {hit.page_index + 1}
                            </span>
                          )}
                          <button
                            type="button"
                            disabled={openingLibraryAnnotation !== null}
                            onClick={() => void openLibraryAnnotation(hit)}
                          >
                            {openingLibraryAnnotation === hit.id && (
                              <LoaderCircle className="spin" size={13} aria-hidden="true" />
                            )}
                            {hit.source_id === sourceId && hit.source_version_id === selectedVersion
                              ? t('定位摘录', 'Locate highlight')
                              : t('打开资料并定位', 'Open document and locate')}
                          </button>
                        </div>
                      </article>
                    ))}
                    {libraryAnnotationCursor && (
                      <button
                        type="button"
                        disabled={libraryAnnotationBusy}
                        onClick={() => void loadMoreLibraryAnnotations()}
                      >
                        {libraryAnnotationBusy
                          ? t('正在加载…', 'Loading…')
                          : t('加载更多全库摘录', 'Load more workspace highlights')}
                      </button>
                    )}
                  </div>
                )}
              </>
            )}
          </aside>
        )}
      </div>
    </PageFrame>
  );
}

function AnnotationCard({
  identity,
  annotation,
  focused,
  locked,
  onJump,
  onAdd,
  selected,
  selectDisabled,
  onSelect,
  onSave,
  onTrash,
}: {
  identity: WorkspaceIdentity;
  annotation: Annotation;
  focused?: boolean;
  locked: boolean;
  onJump: () => void;
  onAdd?: () => void;
  selected?: boolean;
  selectDisabled?: boolean;
  onSelect?: (checked: boolean) => void;
  onSave: (comment: string) => Promise<boolean>;
  onTrash: () => void;
}) {
  const { t } = useI18n();
  const [edit, setEdit] = useState(false);
  const [comment, setComment] = useState(annotation.comment);
  const editField = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (edit) editField.current?.focus();
  }, [edit]);
  return (
    <article
      className={`source-annotation${focused ? ' is-focused' : ''}`}
      data-annotation-card="true"
      data-annotation-id={annotation.id}
      role="listitem"
      tabIndex={0}
      aria-current={focused ? 'true' : undefined}
      aria-label={`${t('摘录与旁注', 'Highlight and note')}: ${annotation.locator.quote.slice(0, 120)}`}
    >
      {onSelect && !annotation.trashed && (
        <label className="source-annotation-selection">
          <input
            type="checkbox"
            checked={selected ?? false}
            disabled={selectDisabled}
            onChange={(event) => onSelect(event.target.checked)}
            aria-label={`${t('选择摘录', 'Select excerpt')}: ${annotation.locator.quote.slice(0, 80)}`}
          />
          {t('选择此摘录', 'Select this excerpt')}
        </label>
      )}
      <blockquote>{annotation.locator.quote}</blockquote>
      {annotation.locator.kind === 'pdf' && (
        <small>
          {t('页', 'Page')} {annotation.locator.page_label ?? annotation.locator.page_index + 1}
        </small>
      )}
      {edit ? (
        <>
          <textarea
            ref={editField}
            aria-label={t('编辑批注', 'Edit note')}
            rows={3}
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
          <button
            disabled={locked}
            onClick={() =>
              void onSave(comment).then((ok) => {
                if (ok) setEdit(false);
              })
            }
          >
            {t('保存', 'Save')}
          </button>
          <button
            onClick={() => {
              setComment(annotation.comment);
              setEdit(false);
            }}
          >
            {t('取消', 'Cancel')}
          </button>
        </>
      ) : (
        annotation.comment && <p>{annotation.comment}</p>
      )}
      <div className="source-tools">
        <AddToReview
          identity={identity}
          target={{
            target_id: annotation.id,
            target_revision_id: annotation.source_version_id,
            target_block_id: null,
          }}
          disabled={locked || annotation.trashed}
        />
        <button onClick={onJump} disabled={annotation.trashed}>
          {t('回到原文', 'Show original')}
        </button>
        {onAdd && (
          <button onClick={onAdd} disabled={annotation.trashed}>
            {t('加入笔记', 'Add to note')}
          </button>
        )}
        <PopupMenu
          label={t('摘录操作', 'Highlight actions')}
          trigger={<MoreHorizontal size={16} />}
          disabled={locked}
          items={[
            {
              id: 'edit',
              label: t('编辑批注', 'Edit note'),
              disabled: annotation.trashed,
              onSelect: () => setEdit(true),
            },
            {
              id: 'trash',
              label: annotation.trashed
                ? t('恢复摘录', 'Restore highlight')
                : t('删除摘录', 'Delete highlight'),
              onSelect: onTrash,
            },
          ]}
        />
      </div>
    </article>
  );
}

function BookCover({
  identity,
  versionId,
  parseId,
  title,
}: {
  identity: WorkspaceIdentity;
  versionId: string;
  parseId: string | null;
  title: string;
}) {
  const { t } = useI18n();
  const [cover, setCover] = useState<SourceCoverData | null>(null);
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let live = true;
    setCover(null);
    setFailed(false);
    void call(
      {
        kind: 'sources_cover',
        payload: { source_version_id: versionId, parse_id: parseId },
      },
      'source_cover',
      identity,
    )
      .then((result) => {
        if (!live) return;
        if (
          result.source_version_id !== versionId ||
          !/^image\/(?:png|jpeg|webp)$/i.test(result.media_type) ||
          !result.data_base64 ||
          result.data_base64.length > 6 * 1024 * 1024
        ) {
          setFailed(true);
          return;
        }
        setCover(result);
      })
      .catch(() => {
        if (live) setFailed(true);
      });
    return () => {
      live = false;
    };
  }, [identity, versionId, parseId, retry]);
  return (
    <figure className="source-book-cover" aria-busy={!cover && !failed}>
      {cover ? (
        <img
          src={`data:${cover.media_type};base64,${cover.data_base64}`}
          alt={t(`《${title}》封面`, `Cover of ${title}`)}
          decoding="async"
          onError={() => {
            setCover(null);
            setFailed(true);
          }}
        />
      ) : (
        <div className="source-book-cover-placeholder">
          <BookOpen size={24} aria-hidden="true" />
          {failed ? (
            <button type="button" className="text-button" onClick={() => setRetry((n) => n + 1)}>
              {t('重试封面', 'Retry cover')}
            </button>
          ) : (
            <span role="status">{t('正在读取封面…', 'Loading cover…')}</span>
          )}
        </div>
      )}
    </figure>
  );
}
function TextReader({
  identity,
  version,
  title,
  author,
  initialMode,
  parseId,
  initialOffset,
  location,
  strictLocation,
  onSelection,
  onPosition,
}: {
  identity: WorkspaceIdentity;
  version: SourceVersion;
  title: string;
  author: string | null;
  initialMode: 'rendered' | 'source';
  parseId: string | null;
  initialOffset: number;
  location: SourceLocator | null;
  strictLocation: boolean;
  onSelection: (locator: SourceLocator | null) => void;
  onPosition: (locator: SourceLocator) => void;
}) {
  const { t } = useI18n();
  const versionId = version.id;
  const isMarkdown = version.format === 'markdown';
  const isDocx = version.format === 'docx';
  const isWord = isDocx || version.format === 'doc';
  const hasRenderedView = isMarkdown || isWord;
  const [mode, setMode] = useState<'rendered' | 'source'>(initialMode);
  const [text, setText] = useState<SourceTextPage | null>(null);
  const [startOffset, setStartOffset] = useState(Math.max(0, initialOffset - 50));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [find, setFind] = useState('');
  const [findNotice, setFindNotice] = useState<string | null>(null);
  const [tableNotice, setTableNotice] = useState<string | null>(null);
  const [found, setFound] = useState<SourceLocator | null>(null);
  const [currentBlockId, setCurrentBlockId] = useState<string | null>(null);
  const [tocOpen, setTocOpen] = useState(() => window.innerHeight > 700);
  const [bookBarCompact, setBookBarCompact] = useState(false);
  const targetLocation = found ?? location;
  const tocItems = text?.toc ?? [];
  const currentTocIndex = useMemo(() => {
    if (!text?.toc.length) return -1;
    const loadedIndex = currentBlockId
      ? text.blocks.findIndex((block) => block.id === currentBlockId)
      : -1;
    return activeBookChapterIndex(
      text.toc,
      loadedIndex >= 0 ? startOffset + loadedIndex : startOffset,
    );
  }, [text, startOffset, currentBlockId]);
  const currentTocItem = currentTocIndex >= 0 ? tocItems[currentTocIndex] : null;
  useEffect(() => {
    if (bookBarCompact) setTocOpen(false);
  }, [bookBarCompact]);
  const previousChapterIndex = adjacentBookChapterIndex(tocItems, currentTocIndex, -1);
  const nextChapterIndex = adjacentBookChapterIndex(tocItems, currentTocIndex, 1);

  const appliedLocation = useRef<SourceLocator | null>(null);
  const alignTargetTop = useRef(false);
  const programmaticBlock = useRef<string | null>(null);
  const programmaticUntil = useRef(0);
  const epoch = useRef(0);
  const root = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const loading = useRef(false);
  useEffect(() => {
    let live = true;
    setError(null);
    setBusy(true);
    void call(
      {
        kind: 'sources_text',
        payload: {
          source_version_id: versionId,
          parse_id: parseId,
          offset: Math.max(0, initialOffset - 50),
          limit: 100,
        },
      },
      'source_text',
      identity,
    )
      .then((result) => {
        if (live) {
          setText(result);
          setStartOffset(Math.max(0, initialOffset - 50));
        }
      })
      .catch((e) => {
        if (live)
          setError(
            (e as { details?: { code?: string } }).details?.code === 'not_found'
              ? t(
                  '此版本的正文尚不可用。请在任务中心查看解析结果，处理完成后重新加载。',
                  'Text for this version is not available. Check processing in Tasks, then reload once it completes.',
                )
              : message(e),
          );
      })
      .finally(() => {
        if (live) setBusy(false);
      });
    return () => {
      live = false;
    };
  }, [versionId, retry]);
  useEffect(() => {
    epoch.current++;
    return () => {
      epoch.current++;
    };
  }, [versionId, retry]);
  useEffect(() => {
    setFound(null);
    alignTargetTop.current = false;
  }, [location]);
  async function more() {
    if (text?.next_offset == null || loading.current) return;
    loading.current = true;
    const run = epoch.current;
    setBusy(true);
    try {
      const result = await call(
        {
          kind: 'sources_text',
          payload: {
            source_version_id: versionId,
            parse_id: text.parse_id,
            offset: text.next_offset,
            limit: 100,
          },
        },
        'source_text',
        identity,
      );
      if (run === epoch.current)
        setText((old) => (old ? { ...result, blocks: [...old.blocks, ...result.blocks] } : result));
    } catch (e) {
      if (run === epoch.current) setError(message(e));
    } finally {
      loading.current = false;
      if (run === epoch.current) setBusy(false);
    }
  }
  async function previous() {
    if (!text || startOffset === 0 || loading.current || busy) return;
    loading.current = true;
    const run = epoch.current;
    const offset = Math.max(0, startOffset - 100);
    const anchor = root.current?.querySelector<HTMLElement>('[data-block-id]');
    const top = anchor?.getBoundingClientRect().top;
    setBusy(true);
    try {
      const result = await call(
        {
          kind: 'sources_text',
          payload: {
            source_version_id: versionId,
            parse_id: text.parse_id,
            offset,
            limit: startOffset - offset,
          },
        },
        'source_text',
        identity,
      );
      if (run !== epoch.current) return;
      setText((old) => (old ? { ...old, blocks: [...result.blocks, ...old.blocks] } : result));
      setStartOffset(offset);
      requestAnimationFrame(() => {
        if (run === epoch.current && anchor?.isConnected && top !== undefined && scroller.current)
          scroller.current.scrollTop += anchor.getBoundingClientRect().top - top;
      });
    } catch (e) {
      if (run === epoch.current) setError(message(e));
    } finally {
      loading.current = false;
      if (run === epoch.current) setBusy(false);
    }
  }
  function locatorFor(block: SourceTextPage['blocks'][number]): SourceLocator {
    const end = [...block.text].slice(0, 32).join('').length;
    return textLocator(versionId, block.id, block.text, 0, end);
  }
  function revealBlock(block: SourceTextPage['blocks'][number]) {
    setCurrentBlockId(block.id);
    appliedLocation.current = null;
    alignTargetTop.current = true;
    setFound(locatorFor(block));
  }
  async function jumpToPosition(position: number, blockId: string | null, offsetOverride?: number) {
    if (!text || loading.current) return;
    const direct = blockId
      ? text.blocks.findIndex((block) => block.id === blockId)
      : position >= startOffset && position < startOffset + text.blocks.length
        ? position - startOffset
        : -1;
    if (direct >= 0 && text.blocks[direct]) {
      setTocOpen(false);
      revealBlock(text.blocks[direct]);
      return;
    }
    loading.current = true;
    const run = epoch.current;
    const offset = offsetOverride ?? Math.max(0, position - 8);
    setBusy(true);
    setError(null);
    try {
      const page = await call(
        {
          kind: 'sources_text',
          payload: { source_version_id: versionId, parse_id: text.parse_id, offset, limit: 100 },
        },
        'source_text',
        identity,
      );
      if (run !== epoch.current) return;
      const target = blockId
        ? page.blocks.find((block) => block.id === blockId)
        : page.blocks[Math.min(Math.max(0, position - offset), page.blocks.length - 1)];
      if (!target)
        throw new Error(
          t(
            '目录位置不在当前解析版本中，请重新导入此电子书。',
            'The chapter is not in this parse; import the book again.',
          ),
        );
      setText(page);
      setStartOffset(offset);
      setTocOpen(false);
      revealBlock(target);
    } catch (e) {
      if (run === epoch.current) setError(message(e));
    } finally {
      loading.current = false;
      if (run === epoch.current) setBusy(false);
    }
  }
  function jumpToToc(index: number) {
    if (!text) return;
    const target = bookChapterTarget(text.toc, index);
    if (target) void jumpToPosition(target.position, target.blockId, target.offset);
  }
  function goChapter(direction: -1 | 1) {
    if (!text) return;
    const next = adjacentBookChapterIndex(text.toc, currentTocIndex, direction);
    if (next !== null) jumpToToc(next);
  }
  async function findText() {
    if (!text || !find || busy || loading.current) return;
    const run = ++epoch.current;
    setBusy(true);
    setFindNotice(null);
    try {
      let offset = 0;
      for (;;) {
        const page = await call(
          {
            kind: 'sources_text',
            payload: { source_version_id: versionId, parse_id: text.parse_id, offset, limit: 100 },
          },
          'source_text',
          identity,
        );
        if (run !== epoch.current) return;
        const block = page.blocks.find((block) => block.text.includes(find));
        if (block) {
          const start = block.text.indexOf(find);
          setText(page);
          setStartOffset(offset);
          setFound(textLocator(versionId, block.id, block.text, start, start + find.length));
          setFindNotice(t('已定位到原文。', 'Matching text located.'));
          return;
        }
        if (page.next_offset === null) break;
        if (page.next_offset <= offset) throw new Error('Invalid text page cursor');
        offset = page.next_offset;
      }
      setFindNotice(t('没有找到此文字（区分大小写）。', 'No matching text (case sensitive).'));
    } catch (e) {
      if (run === epoch.current) setError(message(e));
    } finally {
      if (run === epoch.current) setBusy(false);
    }
  }
  useEffect(() => {
    const location = targetLocation;
    if (
      !location ||
      location === appliedLocation.current ||
      location.kind !== 'text' ||
      location.source_version_id !== versionId ||
      !text
    )
      return;
    const element = [
      ...(root.current?.querySelectorAll<HTMLElement>('[data-block-id]') ?? []),
    ].find((e) => e.dataset.blockId === location.block_id);
    if (element) {
      if (alignTargetTop.current && scroller.current) {
        const viewport = scroller.current.getBoundingClientRect();
        const chrome = scroller.current
          .querySelector('.source-book-navigation')
          ?.getBoundingClientRect();
        const visibleTop = Math.max(viewport.top, chrome?.bottom ?? viewport.top) + 4;
        scroller.current.scrollTop += element.getBoundingClientRect().top - visibleTop;
      } else {
        element.scrollIntoView({ block: 'center' });
      }
      alignTargetTop.current = false;
      programmaticBlock.current = location.block_id;
      programmaticUntil.current = performance.now() + 700;
      element.focus({ preventScroll: true });
      setCurrentBlockId(location.block_id);
      appliedLocation.current = location;
    } else if (text.next_offset !== null && !busy && !error) void more();
    else if (text.next_offset === null) {
      appliedLocation.current = location;
      if (strictLocation) {
        setError(
          t(
            '原文块在此解析版本中不可用；未跳转到相似文字。',
            'Original block is unavailable in this parse version.',
          ),
        );
      }
    }
  }, [targetLocation, text, busy, strictLocation]);
  function selection() {
    const selected = window.getSelection();
    if (!selected || selected.isCollapsed || !selected.rangeCount || !root.current || !text) return;
    const selection = selectedTextBlock(root.current, selected.getRangeAt(0));
    if (!selection) return;
    if (selection === 'multiple') {
      onSelection(null);
      setError(
        t(
          '请在同一段内选择摘录，分别保留每段的准确位置。',
          'Select within one paragraph to preserve its exact location.',
        ),
      );
      return;
    }
    const block = text.blocks.find((b) => b.id === selection.block.dataset.blockId);
    if (!block) return;
    try {
      onSelection(textLocator(versionId, block.id, block.text, selection.start, selection.end));
      setError(null);
    } catch (e) {
      setError(message(e));
    }
  }
  function reportPosition() {
    if (!scroller.current || !root.current || !text) return;
    const compact = scroller.current.scrollTop > 220;
    setBookBarCompact((current) => (current === compact ? current : compact));
    if (programmaticBlock.current && performance.now() < programmaticUntil.current) {
      const pinned = text.blocks.find((block) => block.id === programmaticBlock.current);
      if (pinned) {
        setCurrentBlockId(pinned.id);
        onPosition(locatorFor(pinned));
        return;
      }
    }
    programmaticBlock.current = null;
    const viewport = scroller.current.getBoundingClientRect();
    const chrome = scroller.current
      .querySelector('.source-book-navigation')
      ?.getBoundingClientRect();
    const visibleTop = Math.max(viewport.top, chrome?.bottom ?? viewport.top) + 8;
    const element = [...root.current.querySelectorAll<HTMLElement>('[data-block-id]')].find(
      (el) => el.getBoundingClientRect().bottom > visibleTop,
    );
    const block = text.blocks.find((b) => b.id === element?.dataset.blockId);
    if (block?.text) {
      setCurrentBlockId(block.id);
      onPosition(locatorFor(block));
    }
  }
  function releaseProgrammaticBookTarget() {
    programmaticBlock.current = null;
    programmaticUntil.current = 0;
  }
  return (
    <div
      className="source-prose-scroll"
      ref={scroller}
      onScroll={reportPosition}
      onWheel={releaseProgrammaticBookTarget}
      onPointerDown={releaseProgrammaticBookTarget}
      onKeyDown={(event) => {
        if (isBookScrollIntentKey(event.key)) releaseProgrammaticBookTarget();
      }}
    >
      {hasRenderedView && (
        <div
          className="view-switch source-view-switch"
          role="group"
          aria-label={t('资料阅读视图', 'Document reading view')}
        >
          <button
            type="button"
            aria-pressed={mode === 'rendered'}
            onClick={() => setMode('rendered')}
          >
            {isWord ? t('版式阅读', 'Word layout') : t('阅读', 'Reading')}
          </button>
          <button type="button" aria-pressed={mode === 'source'} onClick={() => setMode('source')}>
            {t('原文（可摘录）', 'Source (excerpts)')}
          </button>
        </div>
      )}
      {isMarkdown && mode === 'rendered' && (
        <MarkdownReadView identity={identity} version={version} />
      )}
      {isWord && mode === 'rendered' && (
        <DocxReadView identity={identity} version={version} onFallback={() => setMode('source')} />
      )}
      {(!hasRenderedView || mode === 'source') && (
        <>
          {text && (text.has_cover || tocItems.length > 0) && (
            <section
              className={`source-book-navigation${bookBarCompact ? ' is-compact' : ''}`}
              aria-label={t('电子书导航', 'Book navigation')}
            >
              <div className="source-book-summary">
                {text.has_cover && (
                  <BookCover
                    identity={identity}
                    versionId={versionId}
                    parseId={text.parse_id}
                    title={title}
                  />
                )}
                <div className="source-book-identity">
                  <span className="source-book-kicker">
                    <BookOpen size={14} aria-hidden="true" />
                    {version.format.toUpperCase()}
                  </span>
                  <strong>{title}</strong>
                  {author && <small>{author}</small>}
                  <p className="source-book-current" aria-live="polite">
                    {currentTocItem
                      ? t(
                          `当前章节：${currentTocItem.title}`,
                          `Current chapter: ${currentTocItem.title}`,
                        )
                      : t('从当前位置开始阅读', 'Reading from your saved position')}
                  </p>
                  <div
                    className="source-book-controls"
                    role="group"
                    aria-label={t('章节导航', 'Chapter navigation')}
                  >
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={busy || previousChapterIndex === null}
                      onClick={() => goChapter(-1)}
                    >
                      <ChevronLeft size={15} aria-hidden="true" />
                      {t('上一章', 'Previous')}
                    </button>
                    <button
                      type="button"
                      className="secondary-button"
                      aria-expanded={tocOpen}
                      aria-controls={`book-toc-${versionId}`}
                      disabled={tocItems.length === 0}
                      onClick={() => setTocOpen((open) => !open)}
                    >
                      <List size={15} aria-hidden="true" />
                      {t('目录', 'Contents')}
                      <span className="source-book-count">{tocItems.length}</span>
                    </button>
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={busy || nextChapterIndex === null}
                      onClick={() => goChapter(1)}
                    >
                      {t('下一章', 'Next')}
                      <ChevronRight size={15} aria-hidden="true" />
                    </button>
                  </div>
                </div>
              </div>
              {tocOpen && tocItems.length > 0 && (
                <nav
                  id={`book-toc-${versionId}`}
                  className="source-book-toc"
                  aria-label={t('目录', 'Contents')}
                >
                  <ol>
                    {tocItems.map((item, index) => (
                      <li key={`${item.block_id ?? item.block_position}-${index}`}>
                        <button
                          type="button"
                          aria-current={index === currentTocIndex ? 'location' : undefined}
                          disabled={
                            busy || (item.block_position === null && item.block_id === null)
                          }
                          style={{ paddingInlineStart: `${12 + (item.level - 1) * 14}px` }}
                          onClick={() => jumpToToc(index)}
                        >
                          <span>{item.title}</span>
                        </button>
                      </li>
                    ))}
                  </ol>
                </nav>
              )}
            </section>
          )}
          <form
            className="source-find"
            onSubmit={(e) => {
              e.preventDefault();
              void findText();
            }}
          >
            <input
              aria-label={t('查找正文（区分大小写）', 'Find text (case sensitive)')}
              value={find}
              onChange={(e) => setFind(e.target.value)}
              placeholder={t('查找文内文字', 'Find in document')}
            />
            <button disabled={!text || busy || !find}>{t('查找', 'Find')}</button>
            {findNotice && <small role="status">{findNotice}</small>}
          </form>
          {error && (
            <div role="alert" className="source-notice">
              {error}
              <button onClick={() => setRetry((n) => n + 1)}>{t('重新加载', 'Reload')}</button>
            </div>
          )}
          {text?.ocr_applied && (
            <p className="source-notice" role="status">
              {t(
                '已对本资料中没有文字层的页面应用本地 OCR（Tesseract 5）。识别文本可能有误，词级坐标只用于近似定位。',
                'Local OCR (Tesseract 5) ran on pages without a text layer. Recognition can be wrong; word-level boxes are approximate.',
              )}
            </p>
          )}
          {text?.needs_ocr && (
            <p className="source-notice">
              {t(
                '此资料的部分内容没有可提取文字；本地 OCR 未配置、不可用或未能识别。',
                'Some content has no extractable text; local OCR is unconfigured, unavailable, or could not recognize it.',
              )}
            </p>
          )}
          {text?.cleaning && (
            <p className="source-notice source-cleaning-notice" role="status">
              {t('清洗：', 'Cleaning: ')}
              {cleaningSummary(text.cleaning, t)}
            </p>
          )}
          {text?.tables && text.tables.length > 0 && (
            <details className="source-table-extract" open>
              <summary>
                {t(`表格抽取（${text.tables.length}）`, `Extracted tables (${text.tables.length})`)}
              </summary>
              {text.tables.map((table, tableIndex) => (
                <div key={`${table.page_index}-${tableIndex}`} className="source-table-block">
                  <div className="source-table-caption">
                    <span>
                      {t(
                        `第 ${table.page_index + 1} 页 · ${table.rows.length} 行`,
                        `Page ${table.page_index + 1} · ${table.rows.length} rows`,
                      )}
                    </span>
                    <button
                      className="secondary-button"
                      onClick={() => {
                        const rows = table.rows;
                        const escape = (cell: string) => cell.replaceAll('|', '\\|');
                        const header = rows[0] ?? [];
                        const line = (row: string[]) => `| ${row.map(escape).join(' | ')} |`;
                        const markdown = [
                          line(header),
                          `| ${header.map(() => '---').join(' | ')} |`,
                          ...rows.slice(1).map(line),
                        ].join('\n');
                        void navigator.clipboard
                          .writeText(markdown)
                          .then(() =>
                            setTableNotice(
                              t('表格已复制为 Markdown。', 'Table copied as Markdown.'),
                            ),
                          )
                          .catch(() =>
                            setTableNotice(
                              t(
                                '复制失败，请手动选择表格。',
                                'Copy failed; select the table manually.',
                              ),
                            ),
                          );
                      }}
                    >
                      {t('复制 Markdown', 'Copy Markdown')}
                    </button>
                  </div>
                  <table className="source-extracted-table">
                    <thead>
                      <tr>
                        {(table.rows[0] ?? []).map((cell, cellIndex) => (
                          <th key={cellIndex}>{cell}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {table.rows.slice(1).map((row, rowIndex) => (
                        <tr key={rowIndex}>
                          {row.map((cell, cellIndex) => (
                            <td key={cellIndex}>{cell}</td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ))}
              <p className="muted">
                {t(
                  '按字符几何还原，适合普通网格；合并单元格、跨页表头、竖排与未识别扫描格不保证。',
                  'Reconstructed from character geometry; merged cells, cross-page headers, vertical text and unrecognized scan cells are not guaranteed.',
                )}
              </p>
              {tableNotice && <p role="status">{tableNotice}</p>}
            </details>
          )}
          <div className="source-prose" ref={root} onPointerUp={selection} onKeyUp={selection}>
            {text && startOffset > 0 && (
              <button disabled={busy} onClick={() => void previous()}>
                {t('阅读前文', 'Read earlier text')}
              </button>
            )}
            {text?.blocks.map((block) => {
              const selected =
                targetLocation?.kind === 'text' && targetLocation.block_id === block.id
                  ? targetLocation
                  : null;
              const points = selected ? [...block.text] : null;
              const image = text.images.find((candidate) => candidate.block_id === block.id);
              const moveFocus = (event: ReactKeyboardEvent<HTMLElement>) => {
                if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
                if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                  const sibling =
                    event.key === 'ArrowDown'
                      ? event.currentTarget.nextElementSibling
                      : event.currentTarget.previousElementSibling;
                  if (sibling instanceof HTMLElement && sibling.dataset.blockId) {
                    event.preventDefault();
                    sibling.focus();
                  }
                } else if (event.key.toLowerCase() === 'h' && block.text) {
                  event.preventDefault();
                  window.getSelection()?.removeAllRanges();
                  setError(null);
                  onSelection(textLocator(versionId, block.id, block.text, 0, block.text.length));
                }
              };
              if (image) {
                return (
                  <figure
                    key={block.id}
                    className="source-prose-image"
                    data-block-id={block.id}
                    tabIndex={0}
                    aria-label={image.alt || block.text || t('图片', 'Image')}
                    onKeyDown={moveFocus}
                    onFocus={() => {
                      setCurrentBlockId(block.id);
                      if (block.text) onPosition(locatorFor(block));
                    }}
                  >
                    <SourceBlockImage
                      identity={identity}
                      versionId={versionId}
                      parseId={text.parse_id}
                      image={image}
                    />
                  </figure>
                );
              }
              return (
                <p
                  key={block.id}
                  data-block-id={block.id}
                  tabIndex={0}
                  onKeyDown={moveFocus}
                  onFocus={() => {
                    if (block.text) {
                      setCurrentBlockId(block.id);
                      onPosition(locatorFor(block));
                    }
                  }}
                >
                  {selected && points ? (
                    <>
                      {points.slice(0, selected.start_codepoint).join('')}
                      <mark>
                        {points.slice(selected.start_codepoint, selected.end_codepoint).join('')}
                      </mark>
                      {points.slice(selected.end_codepoint).join('')}
                    </>
                  ) : (
                    block.text
                  )}
                </p>
              );
            })}
            {busy && <p role="status">{t('正在加载正文…', 'Loading text…')}</p>}
            {text?.next_offset != null && (
              <button disabled={busy} onClick={() => void more()}>
                {t('继续阅读', 'Continue reading')}
              </button>
            )}
            {text && !text.blocks.length && !busy && (
              <p>{t('此版本没有可用正文。', 'No readable text is available for this version.')}</p>
            )}
          </div>
        </>
      )}
    </div>
  );
}
