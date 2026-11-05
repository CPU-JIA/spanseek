import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { focusActiveNoteTitle } from './editor/focus-note-title';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { isTauri } from '@tauri-apps/api/core';
import {
  AlertCircle,
  BookOpen,
  Columns2,
  LibraryBig,
  LoaderCircle,
  Plus,
  RefreshCw,
  Inbox,
  Clock3,
  LayoutGrid,
  List,
  FileText,
  Maximize2,
  Minimize2,
  Mic,
  Network,
  Activity,
  Archive,
  FolderClosed,
  FolderOpen,
  ListFilter,
  LayoutTemplate,
  X,
  Trash2,
  Search,
  Sparkles,
  Zap,
  ArrowLeft,
  ArrowRight,
  ChevronsLeft,
  ChevronDown,
  Ellipsis,
  Home,
  Settings2,
  Save,
} from 'lucide-react';
import type {
  AppOperation,
  DesktopAppState,
  DesktopEvent,
  NoteSummary,
  Navigation,
  SessionState,
  WorkspaceIdentity,
  InboxState,
  Tag,
  NoteSort,
  AppPreferences,
  ExportFormat,
  TabKind,
  Annotation,
  ResolvedSourceTarget,
  EvidenceTarget,
  SearchEvidence,
  SearchScope,
  OpenTab,
  LocalDocument,
  LocalDocuments,
  ResourceGrant,
  PdfReadingView,
  TrashItem,
  TrashPreview,
} from '@pkms/contracts';
import {
  call,
  choose,
  message,
  ServiceError,
  chooseOpenDocuments,
  takeOpenDocuments,
  grantOpenDocuments,
  releaseOpenDocument,
  retryStartup,
} from './api';
import { ReferenceProvider } from './editor/reference-context';
import { DraftController } from './editor/draft-controller';
import { EDITOR_DOCUMENT_SCHEMA } from './editor/document-schema';
import { DraftRegistry } from './editor/draft-registry';
import { indexedDrafts } from './editor/drafts';
import { duplicateTitle } from './editor/duplicate-title';
import { TagFilter } from './organization/TagFilter';
import { RecentObjects } from './organization/RecentObjects';
import { getMetadataController } from './organization/NoteOrganization';
import { LibraryBrowser } from './LibraryBrowser';
import { KnowledgeIndexPage } from './organization/KnowledgeIndexPage';
import { SettingsPanel, type SettingsSection } from './SettingsPanel';
import { ProductNavigation } from './layout/ProductNavigation';
import {
  KnowledgeTaskNavigation,
  type TaskMaterialRequest,
} from './inference/knowledge-task-navigation';
import { productSection, type ProductSection } from './layout/product-navigation';
import { TagsPage } from './organization/TagsPage';
import { startupRetryDelay } from './startup-retry';
import {
  modalGuard,
  modalGuardOutsideDrawer,
  shortcutBarrier,
  shortcutBarrierOutsideDrawer,
} from './layout/modal-guard';
import { CollectionsPage } from './organization/CollectionsPage';
import { SavedViewsPage } from './organization/SavedViewsPage';
import { CanvasPage } from './organization/CanvasPage';
import { GraphPage } from './organization/GraphPage';
import { BackupPage } from './portability/BackupPage';
import { SyncStatusButton } from './portability/SyncStatusButton';
import { TaskCenter } from './portability/TaskCenter';
import { acceptedTaskDestination, type TaskDestination } from './portability/task-navigation';
import { portableActions } from './portability/actions';
import {
  announceSyncStatusChanged,
  SYNC_PULLED_EVENT,
  type SyncPulledDetail,
} from './portability/sync-status-events';
import { useTagAutoSync } from './portability/useTagAutoSync';
import {
  announceTagChange,
  sameTagSyncScope,
  TAG_METADATA_CHANGED_EVENT,
} from './portability/tag-sync-events';
import { LanguageContext, setActiveLanguage, useI18n, type Language } from './i18n';
import {
  webView2Compatibility,
  webView2Notice,
  webView2VersionFromUserAgent,
} from './diagnostics/webview-compat';
import {
  DEFAULT_NEW_NOTE_SHORTCUT,
  DEFAULT_SEARCH_SHORTCUT,
  matchShortcut,
  normalizedShortcut,
} from './layout/shortcuts';
import { navigationForView, viewForNavigation, type ViewName } from './layout/navigation-view';
import { resolvePaneIndex } from './layout/pane-target';
import { WORKSPACE_OPERATION_POLL_MS, operationWaitDeadline } from './app/operation-wait';
import { operationErrorMessage, syncFailureReason } from './errors';
import { displayPath } from './format';
import { ColumnResizer } from './layout/ColumnResizer';
import { LayoutDrawer } from './layout/LayoutDrawer';
import { SidebarIcon } from './layout/SidebarIcon';
import { CommandPalette } from './layout/CommandPalette';
import { WorkspaceMenu } from './layout/WorkspaceMenu';
import { WorkspaceTabs } from './layout/WorkspaceTabs';
import {
  closeTabsAfterBarriers,
  handleTabCloseKey,
  isTabCloseKey,
  planTabClose,
  restoreTabCloseFocus,
  type TabCloseMode,
  type TabClosePlan,
} from './layout/tab-closing';
import { EDITOR_MIN, fitColumns, paneChrome, type ColumnWidths } from './layout/columns';
import { SessionQueue } from './layout/session-queue';
import { SessionPositions } from './layout/session-positions';
import { useWindowSession } from './layout/useWindowSession';
import { EditorScrollSession } from './layout/EditorScrollSession';
import { navigateAfterBarriers, navigationIsCurrent } from './layout/navigation-guard';
import {
  VisitHistory,
  isPaneLayoutChange,
  mouseNavigation,
  navigationKey,
} from './layout/visit-history';
import { PageNavigationContext, type PageNavigationState } from './layout/page-navigation-state';
import { useWindowZoom } from './layout/useWindowZoom';
import { usePagedList } from './layout/Pagination.tsx';
import { Tooltip } from './ui/Tooltip';
import { ToastHost } from './ui/ToastHost';
import { PopupMenu } from './ui/PopupMenu';
import { WelcomePage } from './welcome/WelcomePage';
import { BrandMark } from './ui/BrandMark';
import { TrashPage } from './organization/TrashPage';
import { VoiceCapturePage } from './voice/VoiceCapturePage';
import { useTrashActions, useTrashLifecycle } from './organization/useTrashActions';
import { TrashActionNotice } from './organization/TrashActionNotice';
import type { TrashActionOutcome } from './organization/trash-actions';
import { useCreateNote } from './organization/useCreateNote';
import { objectTypeMenuItems, ObjectTypeIcon } from './organization/ObjectTypeControls';
import {
  createObjectTypeDocument,
  objectTypeCatalog,
  objectTypeDefaultTitle,
  objectTypeDefinition,
  objectTypeQueryTokens,
  readObjectTypeRegistry,
  type ObjectTypeDefinition,
} from './organization/object-types';
import { readNoteTemplateRegistry } from './organization/template-registry';
import type { OpenReference, ReferenceLocation } from './editor/reference-model';
import { withoutNote, withoutObjects } from './organization/note-lifecycle';
import { ActionNotice, PageLoadFailure } from './organization/OrganizationControls';
import { flushSourceReadings as flushSavedReadings } from './sources/reading-registry';
import { parseComponentNotice } from './sources/source-import-failure';
import {
  LocalDocumentPage,
  LocalDocumentLibraryDialog,
  type LocalReadingPosition,
} from './sources/LocalDocumentPage';
const SourcesPage = lazy(() =>
  import('./sources/SourcesPage').then((module) => ({ default: module.SourcesPage })),
);
const SourceReader = lazy(() =>
  import('./sources/SourceReader').then((module) => ({ default: module.SourceReader })),
);
const AnnotationToNoteDialog = lazy(() =>
  import('./sources/AnnotationToNoteDialog').then((module) => ({
    default: module.AnnotationToNoteDialog,
  })),
);
const SearchPage = lazy(() =>
  import('./inference/SearchPage').then((module) => ({ default: module.SearchPage })),
);
const AnswersPage = lazy(() =>
  import('./inference/AiPage').then((module) => ({ default: module.AiPage })),
);
const ProposalPage = lazy(() =>
  import('./inference/ProposalPage').then((module) => ({ default: module.ProposalPage })),
);
const ReviewPage = lazy(() =>
  import('./review/ReviewPage').then((module) => ({ default: module.ReviewPage })),
);
const AutomationPage = lazy(() =>
  import('./automation/AutomationPage').then((module) => ({ default: module.AutomationPage })),
);
import './styles.css';
import './ui/field-focus.css';
import './layout/command-palette.css';

const EditorPane = lazy(() =>
  import('./editor/EditorPane').then((module) => ({ default: module.EditorPane })),
);
function keyOf(identity: WorkspaceIdentity | null) {
  return identity ? `${identity.local_instance_id}/${identity.workspace_id}` : '';
}

export default function App() {
  const [app, setApp] = useState<DesktopAppState | null>(null);
  const language: Language = app?.preferences.preferences.language ?? 'zh-CN';
  const t = useCallback((zh: string, en: string) => (language === 'en' ? en : zh), [language]);
  /**
   * The runtime this shell is drawing in. An old WebView2 starts the app and
   * silently drops style declarations; the diagnostics page names that, but only
   * for readers who go looking, so the shell says it once on the way in.
   */
  const runtimeNotice = useMemo(
    () =>
      webView2Notice(
        webView2Compatibility(webView2VersionFromUserAgent(navigator.userAgent)),
        language,
      ),
    [language],
  );
  const prefs = app?.preferences.preferences;
  const [loading, setLoading] = useState(true);
  const [startupReveal, setStartupReveal] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tagSyncError, setTagSyncError] = useState<{ scope: string; text: string } | null>(null);
  // The kernel tells the user to retry some failures (a busy workspace, a full
  // disk). The welcome screen therefore has to offer a real retry action rather
  // than only the close button.
  const [startupFailed, setStartupFailed] = useState(false);
  useEffect(() => {
    const timer = window.setTimeout(() => setStartupReveal(true), 450);
    return () => window.clearTimeout(timer);
  }, []);
  // A missing parse component every import fails on; closing the notice keeps the
  // workbench usable, and the diagnostics page keeps showing the same report.
  const [componentNoticeClosed, setComponentNoticeClosed] = useState(false);
  const [runtimeNoticeClosed, setRuntimeNoticeClosed] = useState(false);
  useWindowZoom({
    storedPercent: prefs?.zoom_percent ?? null,
    onError: (error) =>
      setError(t('无法调整界面缩放：', 'Could not change zoom: ') + message(error)),
    // Ctrl+wheel and Ctrl+± change the same value the settings screen shows.
    onScaleChange: (percent) => void updatePreferences({ zoom_percent: percent }),
  });
  const [session, setSession] = useState<SessionState | null>(null);
  const [view, setViewState] = useState<ViewName>('notes');
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [preferenceError, setPreferenceError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsMounted, setSettingsMounted] = useState(false);
  const [settingsRevision, setSettingsRevision] = useState(0);
  function openSettings() {
    showSettings();
  }
  function showSettings(section?: SettingsSection) {
    setSidebarDrawerOpen(false);
    setListDrawerOpen(false);
    setError(null);
    setPreferenceError(null);
    if (section) updatePageNavigation('settings.section', () => section);
    setSettingsMounted(true);
    setSettingsOpen(true);
  }
  function dismissSettings() {
    setSettingsOpen(false);
    setSettingsRevision((revision) => revision + 1);
  }
  const voiceSaveRef = useRef<(() => Promise<void>) | null>(null);
  const canvasSaveRef = useRef<(() => Promise<void>) | null>(null);
  const registerCanvasSave = useCallback((save: (() => Promise<void>) | null) => {
    canvasSaveRef.current = save;
  }, []);
  const flushSourceReadings = useCallback(async (scope: WorkspaceIdentity) => {
    await voiceSaveRef.current?.();
    await canvasSaveRef.current?.();
    await flushSavedReadings(scope);
  }, []);
  const viewEpoch = useRef(0);
  function setView(next: typeof view, beforeCommit?: () => void, navigationOverride?: Navigation) {
    if (next === 'settings' || next === 'models' || next === 'diagnostics') {
      beforeCommit?.();
      showSettings(next === 'settings' ? undefined : next);
      return;
    }
    const epoch = ++viewEpoch.current;
    const scope = activeRef.current;
    void (scope ? flushSourceReadings(scope) : Promise.resolve())
      .then(() => {
        if (
          epoch === viewEpoch.current &&
          keyOf(activeRef.current) === keyOf(scope) &&
          !transitionRef.current
        ) {
          setSidebarDrawerOpen(false);
          setListDrawerOpen(false);
          beforeCommit?.();
          setSettingsOpen(false);
          setLocalDocumentToken(null);
          setViewState(next);
          // Keep the stored sidebar entry in step with the page. The session has
          // to exist first: writing before it loads would only report a false
          // "workspace is changing".
          const navigation = navigationOverride ?? navigationForView(next);
          if (sessionRef.current && sessionRef.current.layout.navigation !== navigation) {
            void updateLayout((layout) => ({ ...layout, navigation }));
          }
        }
      })
      .catch(failed);
  }
  const setViewRef = useRef(setView);
  function openTaskCenter(destination?: TaskDestination) {
    setView('operations', () => {
      if (destination) updatePageNavigation('tasks.section', () => destination.section);
      updatePageNavigation('tasks.target', () => destination ?? null);
    });
  }
  setViewRef.current = setView;
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    const listener = listen('voice-capture', () => {
      if (disposed || !activeRef.current) return;
      setViewRef.current('voice');
    });
    return () => {
      disposed = true;
      void listener.then((unlisten) => unlisten());
    };
  }, []);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const [sessionAttempt, setSessionAttempt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [sidebar, setSidebar] = useState(true);
  const [windowWidth, setWindowWidth] = useState(() => window.innerWidth);
  const [sidebarDrawerOpen, setSidebarDrawerOpen] = useState(false);
  const [listDrawerOpen, setListDrawerOpen] = useState(false);
  const [columnPreview, setColumnPreview] = useState<Partial<ColumnWidths>>({});
  const columnGeneration = useRef(0);
  const [gallery, setGallery] = useState(false);
  const [noteListOpen, setNoteListOpen] = useState(false);
  const [contentOrigins, setContentOrigins] = useState<Record<string, ProductSection>>({});
  const [focusNoteFilter, setFocusNoteFilter] = useState(false);
  const [navigationAvailability, setNavigationAvailability] = useState({
    back: false,
    forward: false,
  });
  const [focusMode, setFocusMode] = useState(false);
  const [sort, setSort] = useState<NoteSort>('updated_desc');
  const [preferenceBusy, setPreferenceBusy] = useState(false);
  const [titles, setTitles] = useState<Record<string, string>>({});
  const [sourceLocations, setSourceLocations] = useState<
    Record<
      number,
      {
        sourceId: string;
        versionId?: string;
        target?: ResolvedSourceTarget;
        initialPdfView?: PdfReadingView;
      }
    >
  >({});
  const [localDocuments, setLocalDocuments] = useState<LocalDocument[]>([]);
  const localDocumentsRef = useRef(localDocuments);
  localDocumentsRef.current = localDocuments;
  const [localDocumentToken, setLocalDocumentToken] = useState<string | null>(null);
  const localPositions = useRef<Record<string, LocalReadingPosition>>({});
  const [localLibraryDialog, setLocalLibraryDialog] = useState<string | null>(null);
  const [localSaveWhenReady, setLocalSaveWhenReady] = useState<string | null>(null);
  const [openingDocuments, setOpeningDocuments] = useState(false);
  const [draggingDocuments, setDraggingDocuments] = useState(false);
  const [sourceImportRequest, setSourceImportRequest] = useState<
    { id: string; grants: ResourceGrant[] } | undefined
  >();
  const promotedDocuments = useRef(
    new Map<
      string,
      { document: LocalDocument; position?: LocalReadingPosition; identity: WorkspaceIdentity }
    >(),
  );
  const activeLocalDocument = localDocuments.find(
    (document) => document.token === localDocumentToken,
  );
  const [addingAnnotation, setAddingAnnotation] = useState<{
    annotations: Annotation[];
    pane: number;
  } | null>(null);
  const [answerId, setAnswerId] = useState<string | null>(null);
  const [taskMaterialRequest, setTaskMaterialRequest] = useState<TaskMaterialRequest | null>(null);
  const [inferenceScope, setInferenceScope] = useState<SearchScope | null>(null);
  const [proposalId, setProposalId] = useState<string | null>(null);
  const [draftTitles, setDraftTitles] = useState<Record<string, string>>({});
  const controllerSubscriptions = useRef(new Map<DraftController, () => void>());
  const refreshNotesRef = useRef<() => void>(() => undefined);
  const [filter, setFilter] = useState('');
  const [titleQuery, setTitleQuery] = useState('');
  useEffect(() => {
    const timer = setTimeout(() => setTitleQuery(filter.trim()), 180);
    return () => clearTimeout(timer);
  }, [filter]);
  const [inbox, setInbox] = useState<InboxState | null>(null);
  const [typeFilter, setTypeFilter] = useState('');
  const [tags, setTags] = useState<Tag[]>([]);
  const selectedTagsRef = useRef(tags);
  selectedTagsRef.current = tags;
  const tagRefreshSequence = useRef(0);
  const [organizationVersion, setOrganizationVersion] = useState(0);
  const registry = useRef(new DraftRegistry());
  const [transition, setTransition] = useState<string | null>(null);
  const [transitionOperation, setTransitionOperation] = useState<string | null>(null);
  const [transitionError, setTransitionError] = useState<string | null>(null);
  const [transitionCancelState, setTransitionCancelState] = useState<
    'idle' | 'requesting' | 'requested'
  >('idle');
  const retryTransition = useRef<(() => void) | null>(null);
  const transitionRef = useRef(false);
  const application = useRef<HTMLDivElement>(null);
  // Window width drives the column budget. The startup screen renders before
  // the application element exists, so the listener must not depend on it, and
  // the observer is attached once the shell is actually mounted.
  useEffect(() => {
    const measure = () => {
      const width = window.innerWidth;
      setWindowWidth((previous) => (previous === width ? previous : width));
    };
    measure();
    window.addEventListener('resize', measure);
    window.visualViewport?.addEventListener('resize', measure);
    return () => {
      window.removeEventListener('resize', measure);
      window.visualViewport?.removeEventListener('resize', measure);
    };
  }, []);
  useEffect(() => {
    const node = application.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => setWindowWidth(entry.contentRect.width));
    observer.observe(node);
    return () => observer.disconnect();
  }, [app]);
  const refreshSequence = useRef(0);
  const sessionRef = useRef<SessionState | null>(null);
  const layoutQueue = useRef<SessionQueue | null>(null);
  const layoutPositions = useRef<SessionPositions | null>(null);
  const captureWindow = useWindowSession(session ? layoutPositions.current : null, (error) =>
    setError(
      t('无法保存或恢复窗口位置：', 'Could not save or restore window position: ') + message(error),
    ),
  );
  const activeRef = useRef<WorkspaceIdentity | null>(null);
  const workspaceKey = keyOf(app?.active?.identity ?? null);
  const identity = useMemo(() => app?.active?.identity ?? null, [workspaceKey]);
  useEffect(() => setTypeFilter(''), [workspaceKey]);
  const objectTypeRegistryJson =
    app?.workspace_metadata?.object_type_registry ?? prefs?.object_type_registry ?? null;
  const customObjectTypes = useMemo(
    () => readObjectTypeRegistry(objectTypeRegistryJson, identity?.workspace_id).types,
    [identity?.workspace_id, objectTypeRegistryJson],
  );
  const objectTypeOptions = useMemo(
    () => objectTypeCatalog(customObjectTypes),
    [customObjectTypes],
  );
  const templateRegistryJson =
    app?.workspace_metadata?.note_template_registry ?? prefs?.note_template_registry ?? null;
  const noteTemplates = useMemo(
    () => readNoteTemplateRegistry(templateRegistryJson, identity?.workspace_id).templates,
    [identity?.workspace_id, templateRegistryJson],
  );
  const typeFilterTokens = useMemo(() => {
    if (!typeFilter) return [];
    const definition = objectTypeDefinition(typeFilter, customObjectTypes);
    return definition.id.startsWith('custom:')
      ? [typeFilter.trim().toLocaleLowerCase('en-US')]
      : objectTypeQueryTokens(definition);
  }, [customObjectTypes, typeFilter]);
  const bulkPropertyFields = useMemo(() => {
    if (!typeFilter) return [];
    return objectTypeDefinition(typeFilter, customObjectTypes).fields;
  }, [customObjectTypes, typeFilter]);
  const metadataMigration = useRef('');
  useEffect(() => {
    const activeIdentity = app?.active?.identity ?? null;
    const metadata = app?.workspace_metadata;
    const preferences = app?.preferences.preferences;
    if (!activeIdentity || !metadata || !preferences) return;
    const scope = keyOf(activeIdentity);
    if (metadataMigration.current === scope) return;
    const workspaceId = activeIdentity.workspace_id;
    const preferencesTypes = readObjectTypeRegistry(
      preferences.object_type_registry,
      workspaceId,
    ).types;
    const preferencesTemplates = readNoteTemplateRegistry(
      preferences.note_template_registry,
      workspaceId,
    ).templates;
    const metadataTypes = readObjectTypeRegistry(metadata.object_type_registry, workspaceId).types;
    const metadataTemplates = readNoteTemplateRegistry(
      metadata.note_template_registry,
      workspaceId,
    ).templates;
    const migrateTypes = preferencesTypes.length > 0 && metadataTypes.length === 0;
    const migrateTemplates = preferencesTemplates.length > 0 && metadataTemplates.length === 0;
    if (!migrateTypes && !migrateTemplates) return;
    metadataMigration.current = scope;
    void call(
      {
        kind: 'workspace_metadata_update',
        payload: {
          request: {
            base_row_version: metadata.row_version,
            object_type_registry: migrateTypes
              ? preferences.object_type_registry
              : metadata.object_type_registry,
            note_template_registry: migrateTemplates
              ? preferences.note_template_registry
              : metadata.note_template_registry,
          },
        },
      },
      'workspace_metadata',
      activeIdentity,
    )
      .then((updated) => {
        setApp((current) => (current ? { ...current, workspace_metadata: updated } : current));
        void call({ kind: 'sync_auto_push_workspace_metadata' }, 'sync_auto_push', activeIdentity)
          .then((result) => {
            if (result.conflict)
              setPreferenceError(
                t(
                  '模板与对象类型已迁移到本机，但远端版本也发生了变化；请在“备份与同步”中核对后重新保存。',
                  'Templates and object types migrated locally, but the remote version also changed. Review them in Backup and sync, then save again.',
                ),
              );
          })
          .catch((syncError) => {
            setPreferenceError(
              t(
                '模板与对象类型已迁移到本机，但自动同步失败：',
                'Templates and object types migrated locally, but automatic sync failed: ',
              ) + message(syncError),
            );
          });
      })
      .catch((migrationError) => {
        metadataMigration.current = '';
        setPreferenceError(message(migrationError));
      });
  }, [app?.active?.identity, app?.preferences.preferences, app?.workspace_metadata]);
  const commandItems = [
    {
      id: 'new-note',
      label: t('新建笔记', 'New note'),
      description: t('创建一篇空白笔记', 'Create a blank note'),
      keywords: ['create', 'note', '新建', '笔记'],
      icon: <Plus size={16} />,
      shortcut: normalizedShortcut(prefs?.new_note_shortcut, DEFAULT_NEW_NOTE_SHORTCUT),
      disabled: !identity,
      run: () => createNote(),
    },
    ...objectTypeCatalog(customObjectTypes).map((type) => ({
      id: `new-object-${type.id}`,
      label: t(`新建${type.labelZh}`, `New ${type.labelEn.toLocaleLowerCase()}`),
      description: t(type.descriptionZh, type.descriptionEn),
      keywords: ['create', 'object', type.id, ...type.aliases, type.labelZh, type.labelEn],
      icon: <ObjectTypeIcon type={type.icon} />,
      disabled: !identity,
      run: () => createNote(undefined, undefined, type.id),
    })),
    ...noteTemplates.map((template) => ({
      id: `new-template-${template.id}`,
      label: t(`从模板新建：${template.nameZh}`, `New from template: ${template.nameEn}`),
      description: t(template.descriptionZh, template.descriptionEn),
      keywords: ['create', 'template', template.id, template.nameZh, template.nameEn],
      icon: <LayoutTemplate size={16} />,
      disabled: !identity,
      run: () => createNote(undefined, undefined, undefined, template.id),
    })),
    {
      id: 'open-document',
      label: t('打开资料', 'Open document'),
      description: t(
        '打开 PDF、Markdown、TXT、DOCX、DOC、EPUB、MOBI、CSV、XLSX、XLS 或 PPTX',
        'Open a PDF, Markdown, TXT, DOCX, DOC, EPUB, MOBI, CSV, XLSX, XLS or PPTX file',
      ),
      keywords: ['open', 'document', 'file', '资料', '文件'],
      icon: <FolderOpen size={16} />,
      shortcut: 'Ctrl+O',
      run: () => openDocuments(),
    },
    {
      id: 'open-pdf',
      label: t('打开 PDF', 'Open PDF'),
      description: t('直接打开 PDF 阅读', 'Open a PDF for reading'),
      keywords: ['pdf', 'open', '阅读'],
      icon: <FileText size={16} />,
      run: () => openDocuments(true),
    },
    {
      id: 'search-notes',
      label: t('搜索笔记', 'Search notes'),
      description: t('聚焦笔记列表的搜索框', 'Focus the notes list search'),
      keywords: ['search', 'find', '搜索', '查找'],
      icon: <Search size={16} />,
      shortcut: normalizedShortcut(prefs?.search_shortcut, DEFAULT_SEARCH_SHORTCUT),
      disabled: !identity,
      run: () => revealNoteList(false, true),
    },
    {
      id: 'save-current-note',
      label: t('保存当前笔记', 'Save current note'),
      description: t('立即提交当前草稿', 'Commit the current draft now'),
      keywords: ['save', 'commit', '保存', '提交'],
      icon: <Save size={16} />,
      shortcut: 'Ctrl+S',
      disabled: !identity,
      run: () => flushAll().catch((error) => setError(message(error))),
    },
    {
      id: 'navigate-back',
      label: t('后退', 'Back'),
      description: t('返回上一个工作位置', 'Return to the previous location'),
      keywords: ['back', 'previous', '后退', '返回'],
      icon: <ArrowLeft size={16} />,
      shortcut: 'Alt+←',
      disabled: busy || !navigationAvailability.back,
      run: () => visit(-1),
    },
    {
      id: 'navigate-forward',
      label: t('前进', 'Forward'),
      description: t('前往下一个工作位置', 'Go to the next location'),
      keywords: ['forward', 'next', '前进', '下一个'],
      icon: <ArrowRight size={16} />,
      shortcut: 'Alt+→',
      disabled: busy || !navigationAvailability.forward,
      run: () => visit(1),
    },
    {
      id: 'toggle-sidebar',
      label: t('切换侧栏', 'Toggle sidebar'),
      description: t('显示或隐藏左侧导航', 'Show or hide the navigation sidebar'),
      keywords: ['sidebar', 'navigation', '侧栏', '导航'],
      icon: <ChevronsLeft size={16} />,
      shortcut: 'Ctrl+\\',
      run: toggleSidebar,
    },
    {
      id: 'page-knowledge',
      label: t('知识索引', 'Knowledge index'),
      description: t('按分类浏览笔记与资料', 'Browse notes and sources by category'),
      keywords: ['knowledge', 'index', '分类', '知识索引'],
      icon: <LibraryBig size={16} />,
      disabled: !identity,
      run: () => showKnowledgePage('all'),
    },

    {
      id: 'page-inbox',
      label: t('收件箱', 'Inbox'),
      description: t('查看待整理笔记', 'Review notes waiting to be organised'),
      keywords: ['inbox', 'pending', '收件箱'],
      icon: <Inbox size={16} />,
      disabled: !identity,
      run: () => showKnowledgePage('inbox'),
    },
    {
      id: 'page-sources',
      label: t('资料库', 'Library'),
      description: t('浏览导入的资料与原件', 'Browse imported sources and files'),
      keywords: ['sources', 'library', '资料', '原件'],
      icon: <BookOpen size={16} />,
      disabled: !identity,
      run: () => setView('sources'),
    },
    {
      id: 'page-graph',
      label: t('知识图谱', 'Knowledge graph'),
      description: t('查看知识关系与连接路径', 'Explore knowledge relations and connection paths'),
      keywords: ['graph', 'relations', 'connections', '图谱', '关系', '连接'],
      icon: <Network size={16} />,
      disabled: !identity,
      run: () => setView('graph'),
    },
    {
      id: 'page-answers',
      label: t('AI 助手', 'AI assistant'),
      description: t('打开有据问答与 AI 对话', 'Open evidence-backed answers and AI chat'),
      keywords: ['answers', 'chat', 'ai', '问答', '对话'],
      icon: <Sparkles size={16} />,
      disabled: !identity,
      run: () => setView('answers', () => setInferenceScope(null)),
    },
    {
      id: 'page-voice',
      label: t('语音速记', 'Voice capture'),
      description: t('录音、转写并存入收件箱', 'Record, transcribe and file into the inbox'),
      keywords: ['voice', 'audio', 'record', 'transcribe', '语音', '录音', '速记'],
      icon: <Mic size={16} />,
      disabled: !identity,
      run: () => setView('voice'),
    },
    {
      id: 'page-review',
      label: t('回顾', 'Review'),
      description: t('打开回顾队列', 'Open the review queue'),
      keywords: ['review', '回顾'],
      icon: <RefreshCw size={16} />,
      disabled: !identity,
      run: () => setView('review'),
    },
    {
      id: 'page-automation',
      label: t('自动化', 'Automation'),
      description: t('打开整理提案与研究任务', 'Open tidy proposals and research tasks'),
      keywords: ['automation', 'proposal', 'research', '自动化', '提案'],
      icon: <Zap size={16} />,
      disabled: !identity,
      run: () => setView('automation'),
    },
    {
      id: 'page-collections',
      label: t('集合', 'Collections'),
      description: t('打开集合与主题', 'Open collections and topics'),
      keywords: ['collections', 'topics', '集合', '主题'],
      icon: <FolderClosed size={16} />,
      disabled: !identity,
      run: () => setView('collections'),
    },
    {
      id: 'page-views',
      label: t('保存的视图', 'Saved views'),
      description: t(
        '打开表格、看板、日历与时间线',
        'Open tables, boards, calendars and timelines',
      ),
      keywords: ['views', 'table', 'board', 'calendar', '视图', '看板'],
      icon: <LayoutGrid size={16} />,
      disabled: !identity,
      run: () => setView('views'),
    },
    {
      id: 'page-canvas',
      label: t('画布', 'Canvas'),
      description: t('打开主题画布', 'Open the topic canvas'),
      keywords: ['canvas', 'board', '画布'],
      icon: <LayoutGrid size={16} />,
      disabled: !identity,
      run: () => setView('canvas'),
    },
    {
      id: 'page-backup',
      label: t('备份与恢复', 'Backup and restore'),
      description: t('打开完整备份与恢复', 'Open full backup and restore'),
      keywords: ['backup', 'restore', '备份', '恢复'],
      icon: <Archive size={16} />,
      disabled: !identity,
      run: () => setView('backup'),
    },
    {
      id: 'page-trash',
      label: t('回收站', 'Trash'),
      description: t('查看已删除内容', 'View deleted items'),
      keywords: ['trash', 'delete', '回收站', '删除'],
      icon: <Trash2 size={16} />,
      disabled: !identity,
      run: () => setView('trash'),
    },
    {
      id: 'page-operations',
      label: t('任务中心', 'Task center'),
      description: t('查看后台任务与操作', 'View background tasks and operations'),
      keywords: ['tasks', 'operations', '任务', '操作'],
      icon: <List size={16} />,
      disabled: !identity,
      run: () => openTaskCenter(),
    },
    {
      id: 'page-diagnostics',
      label: t('统计与诊断', 'Diagnostics'),
      description: t('查看运行统计与诊断导出', 'View runtime statistics and diagnostics'),
      keywords: ['diagnostics', 'stats', '诊断', '统计'],
      icon: <Activity size={16} />,
      disabled: !identity,
      run: () => setView('diagnostics'),
    },
    {
      id: 'settings',
      label: t('设置', 'Settings'),
      description: t(
        '打开外观、知识库与快捷键设置',
        'Open appearance, workspace and shortcut settings',
      ),
      keywords: ['settings', 'preferences', '设置', '偏好'],
      icon: <Settings2 size={16} />,
      shortcut: 'Ctrl+,',
      run: openSettings,
    },
  ];
  // The notes list is read one page at a time. The scope carries the workspace
  // plus every filter and sort input, so changing any of them starts again at
  // page 1 instead of replaying a cursor from the previous query.
  const notes = usePagedList<NoteSummary>(
    identity
      ? JSON.stringify([
          workspaceKey,
          inbox,
          sort,
          titleQuery,
          tags.map((tag) => tag.id),
          typeFilterTokens,
        ])
      : null,
    (cursor, size) => {
      const scope = identity;
      if (!scope) return Promise.resolve({ items: [], next_cursor: null });
      return call(
        {
          kind: 'notes_list',
          payload: {
            limit: size,
            cursor,
            query: titleQuery || null,
            trashed: false,
            sort,
            filter: {
              inbox_state: inbox,
              tag_ids: tags.map((tag) => tag.id),
              object_types: typeFilterTokens,
            },
          },
        },
        'notes',
        scope,
        12_000,
      );
    },
  );
  const notesItems = notes.page?.items;
  useEffect(() => {
    if (!notesItems) return;
    setTitles((old) => ({
      ...old,
      ...Object.fromEntries(notesItems.map((note) => [note.id, note.title])),
    }));
  }, [notesItems]);
  const trashActions = useTrashActions(identity);
  const trashLifecycle = useTrashLifecycle(identity);
  const creation = useCreateNote(identity);
  const navigationSequence = useRef<Record<number, number>>({});
  const [referenceLocations, setReferenceLocations] = useState<Record<number, ReferenceLocation>>(
    {},
  );
  const [pageNavigation, setPageNavigation] = useState<PageNavigationState>({});
  type NavigationLocation = {
    view: typeof view;
    panes: Array<OpenTab | null>;
    activePane: number;
    sourceLocations: typeof sourceLocations;
    referenceLocations: typeof referenceLocations;
    inbox: typeof inbox;
    tags: typeof tags;
    filter: string;
    sort: typeof sort;
    gallery: boolean;
    answerId: string | null;
    inferenceScope: SearchScope | null;
    proposalId: string | null;
    pageState: PageNavigationState;
  };
  const visits = useRef(new VisitHistory<NavigationLocation>());
  const visiting = useRef(false);
  const updatePageNavigation = useCallback(
    (key: string, change: (old: unknown) => unknown) => {
      if (keyOf(activeRef.current) !== workspaceKey) return;
      setPageNavigation((old) => {
        const next = change(old[key]);
        return Object.is(next, old[key]) ? old : { ...old, [key]: next };
      });
    },
    [workspaceKey],
  );
  const pageNavigationContext = useMemo(
    () => ({
      state: pageNavigation,
      update: updatePageNavigation,
    }),
    [pageNavigation, updatePageNavigation],
  );
  const lifecycleReady =
    !!trashActions.controller &&
    !trashActions.state?.pending &&
    !trashActions.state?.busy &&
    !!trashLifecycle.controller &&
    !trashLifecycle.state?.pending &&
    !trashLifecycle.state?.busy;
  const [editorEpoch, setEditorEpoch] = useState(0);
  const startupAttempt = useRef(0);
  const startupTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const refresh = useCallback(async () => {
    const sequence = ++refreshSequence.current;
    try {
      const next = await call({ kind: 'app_state' }, 'app_state');
      if (sequence !== refreshSequence.current) return;
      clearTimeout(startupTimer.current);
      startupTimer.current = undefined;
      startupAttempt.current = 0;
      activeRef.current = next.active?.identity ?? null;
      setApp(next);
      setError(null);
      setStartupFailed(false);
    } catch (error) {
      if (sequence !== refreshSequence.current) return;
      if (error instanceof ServiceError && error.details.code === 'backend_starting') {
        // The kernel is still opening its state. Wait longer each time instead of
        // asking four times a second, and stop hiding a startup that never ends.
        const delay = startupRetryDelay(
          startupAttempt.current,
          document.visibilityState === 'hidden',
        );
        if (delay === null) {
          setStartupFailed(true);
          setError(
            t(
              '本地服务没有在预期时间内就绪。请重试；若反复出现，请在诊断页查看组件状态。',
              'The local service did not become ready in time. Retry, and check the diagnostics page if it keeps happening.',
            ),
          );
        } else {
          startupAttempt.current += 1;
          setLoading(true);
          clearTimeout(startupTimer.current);
          startupTimer.current = setTimeout(() => void refresh(), delay);
        }
        return;
      }
      startupAttempt.current = 0;
      setStartupFailed(true);
      setError(message(error));
    }
    setLoading(false);
  }, [t]);
  /**
   * Startup failures stay remembered in the kernel until something clears them,
   * so both retry buttons must ask for a real restart before re-reading state.
   */
  const reconnect = useCallback(async () => {
    // A manual retry deserves a fresh budget, whatever the automatic loop spent.
    startupAttempt.current = 0;
    if (isTauri()) {
      try {
        await retryStartup();
      } catch {
        // refresh() below reports whatever still blocks startup.
      }
    }
    await refresh();
  }, [refresh]);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  // A retry scheduled while the window was hidden waits much longer; returning to
  // the window asks right away instead of making the user wait it out.
  useEffect(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'hidden' || startupTimer.current === undefined) return;
      clearTimeout(startupTimer.current);
      startupTimer.current = undefined;
      void refresh();
    };
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      document.removeEventListener('visibilitychange', onVisibility);
      clearTimeout(startupTimer.current);
      startupTimer.current = undefined;
    };
  }, [refresh]);
  useEffect(() => {
    if (!isTauri()) return;
    const listener = listen<DesktopEvent>('pkms:event', (event) => {
      if (event.payload.kind === 'workspace_changed') void refresh();
      if (event.payload.kind === 'operation_changed') {
        void call(
          { kind: 'operation_get', payload: { operation_id: event.payload.payload.operation_id } },
          'operation',
        )
          .then((operation) => {
            if (operation.kind === 'restore_backup' && operation.status === 'succeeded')
              void refresh();
          })
          .catch((error) => setError(message(error)));
      }
      if (event.payload.kind === 'operation_failed') setError(event.payload.payload.error.message);
    });
    return () => {
      void listener.then((unlisten) => unlisten());
    };
  }, [refresh]);
  const refreshTagMetadata = useCallback(async () => {
    if (!identity) return;
    const sequence = ++tagRefreshSequence.current;
    setOrganizationVersion((version) => version + 1);
    refreshNotesRef.current();
    const currentTags = selectedTagsRef.current;
    const results = await Promise.allSettled(
      currentTags.map((tag) =>
        call({ kind: 'tags_get', payload: { tag_id: tag.id } }, 'tag_details', identity),
      ),
    );
    if (sequence !== tagRefreshSequence.current || keyOf(activeRef.current) !== keyOf(identity))
      return;
    const updates = new Map<string, Tag | null>();
    results.forEach((result, index) => {
      if (result.status === 'fulfilled') updates.set(currentTags[index].id, result.value.tag);
      else if (result.reason instanceof ServiceError && result.reason.details.code === 'not_found')
        updates.set(currentTags[index].id, null);
      else setError(message(result.reason));
    });
    setTags((current) =>
      current.flatMap((tag) => {
        const updated = updates.get(tag.id);
        return updated === null ? [] : [updated ?? tag];
      }),
    );
  }, [identity]);
  useEffect(() => {
    const changed = (event: Event) => {
      if (sameTagSyncScope((event as CustomEvent<WorkspaceIdentity>).detail, identity))
        void refreshTagMetadata();
    };
    window.addEventListener(TAG_METADATA_CHANGED_EVENT, changed);
    return () => window.removeEventListener(TAG_METADATA_CHANGED_EVENT, changed);
  }, [identity, refreshTagMetadata]);
  useTagAutoSync(
    identity,
    (result) => {
      if (!result.configured || result.complete) setTagSyncError(null);
      else
        setTagSyncError({
          scope: keyOf(identity),
          text: t(
            `标签同步尚未完成：${result.pending} 项等待处理，${result.dirty} 项本机修改，${result.unavailable} 项缺少本机内容。请在“备份与恢复”查看状态。`,
            `Tag sync is incomplete: ${result.pending} pending, ${result.dirty} with local changes, ${result.unavailable} missing local content. Review the status in Backup and restore.`,
          ),
        });
    },
    (failure) => {
      setTagSyncError({
        scope: keyOf(identity),
        text: t(
          `标签自动同步未完成：${message(failure)}。本机内容已保留，可在“备份与恢复”重试。`,
          `Automatic tag sync did not finish: ${message(failure)}. Local content is retained; retry in Backup and restore.`,
        ),
      });
    },
  );
  const applySyncedNotes = useCallback(
    async (noteIds: string[]) => {
      if (!identity) return;
      const conflicts: string[] = [];
      for (const noteId of new Set(noteIds)) {
        const controller = await registry.current.existing(identity, noteId);
        if (!controller) continue;
        try {
          const note = await call(
            { kind: 'notes_get', payload: { note_id: noteId } },
            'note',
            identity,
          );
          if ((await controller.applyRemote(note)) === 'conflict') conflicts.push(noteId);
        } catch (error) {
          if (keyOf(activeRef.current) === keyOf(identity)) setError(message(error));
        }
      }
      if (conflicts.length > 0 && keyOf(activeRef.current) === keyOf(identity))
        setError(
          t(
            `同步已收到 ${conflicts.length} 篇笔记的远端更新；本地未保存内容已保留，请在编辑器中比较后处理。`,
            `Sync received remote updates for ${conflicts.length} open note(s); local unsaved content was kept for comparison in the editor.`,
          ),
        );
    },
    [identity, t],
  );
  useEffect(() => {
    const onPulled = (event: Event) => {
      const detail = (event as CustomEvent<SyncPulledDetail>).detail;
      if (!detail || !sameTagSyncScope(detail.identity, identity) || !Array.isArray(detail.applied))
        return;
      const applied = detail.applied;
      void applySyncedNotes(applied);
      void refreshTagMetadata();
      void refresh();
    };
    window.addEventListener(SYNC_PULLED_EVENT, onPulled);
    return () => window.removeEventListener(SYNC_PULLED_EVENT, onPulled);
  }, [applySyncedNotes, identity, refresh, refreshTagMetadata]);
  useEffect(() => {
    if (!isTauri() || !identity) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const loop = async () => {
      if (cancelled) return;
      if (document.visibilityState === 'hidden') {
        timer = setTimeout(loop, 15_000);
        return;
      }
      try {
        const watch = await call(
          { kind: 'sync_watch', payload: { timeout_ms: 20_000 } },
          'sync_watch',
          identity,
          25_000,
        );
        if (cancelled) return;
        if (!watch.configured) {
          timer = setTimeout(loop, 30_000);
          return;
        }
        if (watch.changed) {
          const pulled = await call({ kind: 'sync_pull' }, 'sync_pull', identity, 30_000);
          if (cancelled) return;
          await applySyncedNotes(pulled.applied);
          if (cancelled) return;
          if (pulled.tag_results.length) announceTagChange(TAG_METADATA_CHANGED_EVENT, identity);
          const pendingConflicts =
            pulled.config_conflicts.length +
            pulled.conflicts.length +
            pulled.server_conflicts.length +
            pulled.tag_pending;
          refreshNotesRef.current();
          await refresh();
          if (cancelled) return;
          if (pendingConflicts > 0) {
            announceSyncStatusChanged();
            setError(
              t(
                `同步发现 ${pendingConflicts} 项需要核对的冲突；本机内容已保留，请在“备份与恢复”中逐项比较。`,
                `Sync found ${pendingConflicts} conflict(s) that need review. Local content was preserved; compare them in Backup and restore.`,
              ),
            );
          }
        }
        timer = setTimeout(loop, 250);
      } catch (failure) {
        if (cancelled) return;
        announceSyncStatusChanged();
        if (syncFailureReason(failure) === 'device_revoked') {
          setError(message(failure));
          return;
        }
        timer = setTimeout(loop, 15_000);
      }
    };
    timer = setTimeout(loop, 1_200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [applySyncedNotes, identity, refresh]);
  const flushLayout = useCallback(
    async (allowPendingTrash = false) => {
      if (
        activeRef.current &&
        !allowPendingTrash &&
        (!trashActions.controller ||
          !trashLifecycle.controller ||
          trashActions.controller.getSnapshot().pending ||
          trashLifecycle.controller.getSnapshot().pending)
      )
        throw new Error(
          t(
            '回收站操作仍需确认，请先完成或重试后继续。',
            'Confirm or retry the pending Trash operation before continuing.',
          ),
        );
      if (activeRef.current) await flushSourceReadings(activeRef.current);
      await captureWindow();
      await layoutPositions.current?.flush();
      await layoutQueue.current?.flush();
    },
    [captureWindow, trashActions.controller, trashLifecycle.controller, t],
  );
  const flushAll = useCallback(async () => {
    await registry.current.flush();
    await flushLayout();
  }, [flushLayout]);
  const beginTransition = useCallback(
    (label: string) => {
      if (transitionRef.current)
        throw new Error(
          t('请等待当前工作区操作完成。', 'Please wait for the workspace operation to finish.'),
        );
      // A transition owns the window; older deferred navigation must not resume inside it.
      ++viewEpoch.current;
      transitionRef.current = true;
      // Synchronous DOM barrier: no input can sneak in before React commits state.
      if (application.current) {
        application.current.inert = true;
        // Modal dialogs escape ancestor inertness. Dismiss navigation before the
        // transition's retry/cancel controls take ownership of the window.
        for (const drawer of application.current.querySelectorAll<HTMLDialogElement>(
          '.layout-drawer[open]',
        )) {
          drawer.inert = true;
          drawer.close();
        }
      }
      setSidebarDrawerOpen(false);
      setListDrawerOpen(false);
      setTransition(label);
      return () => {
        transitionRef.current = false;
        if (application.current) application.current.inert = false;
        setTransition(null);
        setTransitionOperation(null);
        setTransitionError(null);
        setTransitionCancelState('idle');
        retryTransition.current = null;
      };
    },
    [t],
  );
  useEffect(() => {
    if (!isTauri()) return;
    let closing = false;
    const listener = getCurrentWindow().onCloseRequested(async (event) => {
      if (closing) return;
      event.preventDefault();
      let finish: (() => void) | undefined;
      let drain: ((release: boolean) => void) | undefined;
      try {
        finish = beginTransition(t('正在保存并关闭窗口…', 'Saving and closing the window…'));
        if (activeRef.current) await flushSourceReadings(activeRef.current);
        drain = await registry.current.freezeAndFlush();
        await flushLayout();
        closing = true;
        await getCurrentWindow().destroy();
      } catch (error) {
        closing = false;
        setError(t('窗口已保留：', 'Window kept open: ') + message(error));
      } finally {
        drain?.(false);
        finish?.();
      }
    });
    return () => {
      void listener.then((unlisten) => unlisten());
    };
  }, [beginTransition, t, flushLayout]);
  useEffect(() => {
    visits.current = new VisitHistory();
    setNavigationAvailability({ back: false, forward: false });
    setFocusNoteFilter(false);
    visiting.current = false;
    setPageNavigation({});
    setContentOrigins({});
    setNoteListOpen(false);
    sessionRef.current = null;
    layoutPositions.current?.dispose();
    layoutPositions.current = null;
    layoutQueue.current = null;
    setSessionError(null);
    setReferenceLocations({});
    navigationSequence.current = {};
    setSession(null);
    setFilter('');
    setTitleQuery('');
    setInbox(null);
    setTags([]);
    setTitles({});
    setSourceLocations({});
    setSourceImportRequest(undefined);
    setAddingAnnotation(null);
    setAnswerId(null);
    setInferenceScope(null);
    setProposalId(null);
    setDraftTitles({});
    setGallery(false);
    setFocusMode(false);
    ++columnGeneration.current;
    setColumnPreview({});
    setView('notes');
    if (!identity) return;
    let live = true;
    void call({ kind: 'session_get' }, 'session', identity)
      .then(async (stored) => {
        if (live && keyOf(activeRef.current) === keyOf(identity)) {
          const accept = (next: SessionState) => {
            if (!live || keyOf(activeRef.current) !== keyOf(identity)) return;
            setSession(next);
            sessionRef.current = next;
            setSidebar(next.layout.sidebar_visible);
          };
          const queue = new SessionQueue(stored, {
            read: () => call({ kind: 'session_get' }, 'session', identity),
            save: (base_state_version, layout) =>
              call(
                { kind: 'session_save', payload: { base_state_version, layout } },
                'session',
                identity,
              ),
            onConfirmed: accept,
          });
          layoutQueue.current = queue;
          layoutPositions.current = new SessionPositions(queue, (error) => {
            if (live && keyOf(activeRef.current) === keyOf(identity)) setError(message(error));
          });
          // A recycle commit may precede a failed layout write. Reconcile stale tabs
          // from authoritative metadata before mounting editors after a restart.
          const tabIds = [
            ...new Set(
              stored.layout.panes.flatMap((pane) => pane.tabs.map((tab) => tab.object_id)),
            ),
          ];
          const metadata = await Promise.all(
            tabIds.map((object_id) =>
              call(
                { kind: 'objects_metadata_get', payload: { object_id } },
                'object_metadata',
                identity,
              ),
            ),
          );
          if (!live || keyOf(activeRef.current) !== keyOf(identity)) return;
          const trashed = metadata.filter((item) => item.trashed).map((item) => item.id);
          if (trashed.length) await queue.enqueue((layout) => trashed.reduce(withoutNote, layout));
          else accept(stored);
          if (!live || keyOf(activeRef.current) !== keyOf(identity)) return;
          // Reopen the page the user left behind. A single reset to the note list
          // happens above, so an epoch above that means they already navigated and
          // their choice wins over the stored one.
          if (viewEpoch.current <= 1) {
            const restored = viewForNavigation(stored.layout.navigation);
            if (restored.pageState) {
              setPageNavigation((old) => ({ ...old, ...restored.pageState }));
            }
            if (restored.inbox) showLibrary(restored.inbox);
            else setView(restored.view, undefined, stored.layout.navigation);
          }
          if (
            live &&
            keyOf(activeRef.current) === keyOf(identity) &&
            !sessionRef.current?.layout.panes.some((pane) => pane.active_object_id)
          ) {
            setGallery(true);
          }
        }
      })
      .catch((error) => {
        if (live && keyOf(activeRef.current) === keyOf(identity)) {
          layoutQueue.current = null;
          layoutPositions.current?.dispose();
          layoutPositions.current = null;
          setSessionError(message(error));
        }
      });
    return () => {
      live = false;
    };
  }, [identity, sessionAttempt]);
  useEffect(
    () => () => {
      for (const unsubscribe of controllerSubscriptions.current.values()) unsubscribe();
      controllerSubscriptions.current.clear();
    },
    [identity],
  );
  refreshNotesRef.current = () => void notes.refresh();
  function updateLayout(
    transform: (layout: SessionState['layout']) => SessionState['layout'],
    duringTransition = false,
  ) {
    const queue = layoutQueue.current;
    // A temporary document can be read without a workspace. There is then no
    // session to persist into, so a layout change is simply nothing to write;
    // reporting "workspace is switching" also blocked the close barrier.
    if (!identity) return Promise.resolve();
    const write =
      !queue ||
      keyOf(activeRef.current) !== keyOf(identity) ||
      (!sessionRef.current && !duringTransition) ||
      (transitionRef.current && !duringTransition) ||
      keyOf(queue.getSnapshot().session.identity) !== keyOf(identity)
        ? Promise.reject(
            new Error(
              t(
                '知识库正在切换或尚未准备完成，操作已保留。',
                'The workspace is changing or not ready. Your operation is kept.',
              ),
            ),
          )
        : queue.enqueue(transform).then(() => undefined);
    void write.catch((error) => {
      if (keyOf(activeRef.current) === keyOf(identity)) setError(message(error));
    });
    return write;
  }
  useEffect(() => {
    if (view !== 'knowledge' || !session) return;
    const navigation: Navigation =
      pageNavigation['knowledge.mode'] === 'inbox' ? 'inbox' : 'knowledge';
    if (session.layout.navigation === navigation) return;
    void updateLayout((layout) => ({ ...layout, navigation }));
  }, [identity, pageNavigation, session, view]);

  function toggleSidebar() {
    setListDrawerOpen(false);
    if (!columns.sidebarAllowed) {
      setFocusMode(false);
      setSidebarDrawerOpen((open) => !open);
      return;
    }
    const visible = !(sidebar && !focusMode);
    setFocusMode(false);
    setSidebar(visible);
    void updateLayout((layout) => ({ ...layout, sidebar_visible: visible }));
  }
  function revealNoteList(toggle = false, focus = false) {
    const fits = fitColumns(windowWidth, sidebar, true, {
      sidebar_width: null,
      note_list_width: null,
    }).listAllowed;
    setView('notes', () => {
      setFocusMode(false);
      setGallery(false);
      setNoteListOpen(fits && toggle && view === 'notes' ? !noteListOpen : true);
      setListDrawerOpen(!fits);
      if (focus) setFocusNoteFilter(true);
    });
  }
  async function resizeColumn(column: keyof ColumnWidths, value: number | null) {
    const generation = ++columnGeneration.current;
    setColumnPreview((old) => ({ ...old, [column]: value }));
    try {
      await updateLayout((layout) => ({ ...layout, [column]: value }));
    } finally {
      if (generation === columnGeneration.current) setColumnPreview({});
    }
  }
  function cancelColumnResize() {
    ++columnGeneration.current;
    setColumnPreview({});
  }
  function openNote(
    noteId: string,
    requestedPane = sessionRef.current?.layout.active_pane ?? 0,
    referenceSequence?: number,
    expectedViewEpoch = viewEpoch.current,
    kind: 'note' | 'source' = 'note',
  ) {
    const scope = identity;
    const layout = sessionRef.current?.layout;
    const activeId = layout?.panes[layout.active_pane]?.active_object_id;
    const origin =
      (view === 'notes' || view === 'reader') && !gallery
        ? (contentOrigins[noteId] ??
          productSection(view, false, activeId ? contentOrigins[activeId] : undefined))
        : productSection(view, pageNavigation['knowledge.mode'] === 'inbox');
    const paneIndex = resolvePaneIndex(
      layout?.panes.length ?? 0,
      requestedPane,
      layout?.active_pane,
    );
    if (paneIndex === null) {
      const opening = Promise.reject<boolean>(
        new Error(
          t(
            '当前没有可用的编辑面板，请重新打开知识库。',
            'No editor pane is available. Reopen the workspace.',
          ),
        ),
      );
      void opening.catch(failed);
      return opening;
    }
    const sequence = referenceSequence ?? (navigationSequence.current[paneIndex] ?? 0) + 1;
    navigationSequence.current[paneIndex] = sequence;
    const opening = (scope ? flushSourceReadings(scope) : Promise.resolve())
      .then(async () => {
        if (
          keyOf(activeRef.current) !== keyOf(scope) ||
          transitionRef.current ||
          !sessionRef.current?.layout.panes[paneIndex] ||
          navigationSequence.current[paneIndex] !== sequence ||
          viewEpoch.current !== expectedViewEpoch
        )
          return false;
        await updateLayout((layout) =>
          !paneNavigationCurrent(scope, paneIndex, sequence, expectedViewEpoch)
            ? layout
            : {
                ...layout,
                active_pane: paneIndex,
                panes: layout.panes.map((pane, index) =>
                  index === paneIndex
                    ? {
                        ...pane,
                        active_object_id: noteId,
                        tabs: pane.tabs.some((tab) => tab.object_id === noteId)
                          ? pane.tabs
                          : [...pane.tabs, { object_id: noteId, kind }],
                      }
                    : pane,
                ),
              },
        );
        return true;
      })
      .then((accepted) => {
        if (!accepted) return false;
        if (keyOf(activeRef.current) !== keyOf(scope))
          throw new Error(
            t(
              '知识库已切换，原操作已保留。',
              'The workspace changed. The original operation is kept.',
            ),
          );
        if (
          transitionRef.current ||
          !sessionRef.current?.layout.panes[paneIndex] ||
          navigationSequence.current[paneIndex] !== sequence ||
          viewEpoch.current !== expectedViewEpoch
        )
          return false;
        setSidebarDrawerOpen(false);
        setListDrawerOpen(false);
        setLocalDocumentToken(null);
        setContentOrigins((current) => ({ ...current, [noteId]: origin }));
        setNoteListOpen(false);
        setViewState(kind === 'source' ? 'reader' : 'notes');
        setGallery(false);
        setReferenceLocations((old) => {
          const next = { ...old };
          delete next[paneIndex];
          return next;
        });
        // Recency is best-effort bookkeeping for the inbox list; the opened object
        // stays the source of truth, so a failed touch never blocks opening it.
        if (scope)
          void call(
            { kind: 'recent_objects_touch', payload: { object_id: noteId } },
            'done',
            scope,
          ).catch(() => undefined);
        return true;
      });
    void opening.catch((error) => {
      if (keyOf(activeRef.current) === keyOf(scope)) failed(error);
    });
    return opening;
  }
  async function openSource(
    sourceId: string,
    versionId?: string,
    requestedPane = sessionRef.current?.layout.active_pane ?? 0,
    target?: ResolvedSourceTarget,
    sequence?: number,
    epoch = viewEpoch.current,
    initialPdfView?: PdfReadingView,
  ) {
    const scope = identity;
    const layout = sessionRef.current?.layout;
    const paneIndex = resolvePaneIndex(
      layout?.panes.length ?? 0,
      requestedPane,
      layout?.active_pane,
    );
    if (paneIndex === null)
      throw new Error(
        t(
          '当前没有可用的编辑面板，请重新打开知识库。',
          'No editor pane is available. Reopen the workspace.',
        ),
      );
    const expectedSequence = sequence ?? (navigationSequence.current[paneIndex] ?? 0) + 1;
    if (
      (await openNote(sourceId, paneIndex, expectedSequence, epoch, 'source')) &&
      paneNavigationCurrent(scope, paneIndex, expectedSequence, epoch) &&
      sessionRef.current?.layout.panes[paneIndex]?.active_object_id === sourceId
    )
      setSourceLocations((old) => ({
        ...old,
        [paneIndex]: { sourceId, versionId, target, initialPdfView },
      }));
  }
  function paneNavigationCurrent(
    scope: WorkspaceIdentity | null,
    paneIndex: number,
    sequence: number,
    epoch: number,
  ) {
    return navigationIsCurrent(
      { workspaceKey: keyOf(scope), sequence, epoch },
      {
        workspaceKey: keyOf(activeRef.current),
        sessionWorkspaceKey: keyOf(sessionRef.current?.identity ?? null),
        sequence: navigationSequence.current[paneIndex] ?? 0,
        epoch: viewEpoch.current,
        panePresent: !!sessionRef.current?.layout.panes[paneIndex],
        transitioning: transitionRef.current,
      },
    );
  }
  const [tabCloseFocus, setTabCloseFocus] = useState<{
    plan: TabClosePlan;
    previous: Element | null;
    current: () => boolean;
  } | null>(null);
  useLayoutEffect(() => {
    if (!tabCloseFocus) return;
    if (!tabCloseFocus.current()) {
      setTabCloseFocus(null);
      return;
    }
    const pane = session?.layout.panes[tabCloseFocus.plan.paneIndex];
    if (pane?.tabs.some((tab) => tabCloseFocus.plan.objectIds.includes(tab.object_id))) return;
    restoreTabCloseFocus(tabCloseFocus.plan, tabCloseFocus.previous);
    setTabCloseFocus(null);
  }, [session, tabCloseFocus]);
  async function closeTab(paneIndex: number, objectId: string, mode: TabCloseMode = 'this') {
    const scope = identity;
    const layout = sessionRef.current?.layout;
    if (!scope || transitionRef.current || !layout) return;
    const plan = planTabClose(layout, paneIndex, objectId, mode);
    if (!plan) return;
    const previousFocus = document.activeElement;
    const epoch = viewEpoch.current;
    const sequence = (navigationSequence.current[paneIndex] ?? 0) + 1;
    navigationSequence.current[paneIndex] = sequence;
    const current = () =>
      paneNavigationCurrent(scope, paneIndex, sequence, epoch) &&
      (!plan.keepId ||
        !!sessionRef.current?.layout.panes[paneIndex]?.tabs.some(
          (tab) => tab.object_id === plan.keepId,
        ));
    try {
      const closed = await closeTabsAfterBarriers(
        plan,
        [() => registry.current.flush(), () => flushSourceReadings(scope)],
        current,
        updateLayout,
      );
      if (closed) setTabCloseFocus({ plan, previous: previousFocus, current });
    } catch (error) {
      if (current()) failed(error);
    }
  }
  async function toggleSplit(paneIndex: number) {
    const current = sessionRef.current;
    if (!current || transitionRef.current || !current.layout.panes[paneIndex]) return;
    const closing = current.layout.panes.length === 2;
    const pane = current.layout.panes[paneIndex];
    const location = sourceLocations[paneIndex];
    const scope = identity;
    const epoch = viewEpoch.current;
    const sequence = (navigationSequence.current[paneIndex] ?? 0) + 1;
    navigationSequence.current[paneIndex] = sequence;
    if (closing && paneIndex !== 1)
      navigationSequence.current[1] = (navigationSequence.current[1] ?? 0) + 1;
    const valid = () =>
      keyOf(activeRef.current) === keyOf(scope) &&
      epoch === viewEpoch.current &&
      !transitionRef.current &&
      navigationSequence.current[paneIndex] === sequence;
    if (identity) await flushSourceReadings(identity);
    if (!valid()) return;
    await registry.current.flush();
    if (
      keyOf(activeRef.current) !== keyOf(scope) ||
      epoch !== viewEpoch.current ||
      transitionRef.current ||
      navigationSequence.current[paneIndex] !== sequence ||
      !sessionRef.current?.layout.panes[paneIndex] ||
      sessionRef.current.layout.panes.length !== current.layout.panes.length
    )
      return;
    if (!closing) {
      // Prepare the new pane before its layout is observed by navigation history.
      // A closed pane's previous reference must not become a new content visit.
      setReferenceLocations((old) => {
        const next = { ...old };
        const reference = old[paneIndex];
        if (reference?.target.target_id === pane.active_object_id)
          next[1] = { ...reference, requestId: crypto.randomUUID() };
        else delete next[1];
        return next;
      });
    }
    await updateLayout((layout) =>
      !valid() || layout.panes.length !== current.layout.panes.length
        ? layout
        : {
            ...layout,
            panes: closing
              ? [layout.panes[0]]
              : layout.panes.length === 2
                ? layout.panes
                : [
                    ...layout.panes,
                    {
                      tabs: pane.tabs.filter((tab) => tab.object_id === pane.active_object_id),
                      active_object_id: pane.active_object_id,
                      scroll_offsets: [],
                    },
                  ],
            active_pane: 0,
          },
    );
    if (!valid()) return;
    if (closing)
      setReferenceLocations((old) => {
        const next = { ...old };
        delete next[1];
        return next;
      });
    setSourceLocations((old) => {
      const next = { ...old };
      if (!closing && location) next[1] = location;
      else delete next[1];
      return next;
    });
  }
  const openReference = async (
    requestedPane: number,
    target: Parameters<OpenReference>[0],
    occurrenceId?: string,
    evidence?: EvidenceTarget,
  ) => {
    const scope = identity;
    const layout = sessionRef.current?.layout;
    const paneIndex = resolvePaneIndex(
      layout?.panes.length ?? 0,
      requestedPane,
      layout?.active_pane,
    );
    if (paneIndex === null)
      throw new Error(
        t(
          '当前没有可用的编辑面板，请重新打开知识库。',
          'No editor pane is available. Reopen the workspace.',
        ),
      );
    const expectedViewEpoch = viewEpoch.current;
    const sequence = (navigationSequence.current[paneIndex] ?? 0) + 1;
    navigationSequence.current[paneIndex] = sequence;
    if (!scope || transitionRef.current || !sessionRef.current)
      throw new Error(t('请等待知识库准备完成。', 'Please wait for the workspace to be ready.'));
    const resolved = await call(
      { kind: 'links_resolve', payload: { input: target } },
      'resolved_link',
      scope,
    );
    if (
      keyOf(activeRef.current) !== keyOf(scope) ||
      transitionRef.current ||
      !sessionRef.current?.layout.panes[paneIndex] ||
      navigationSequence.current[paneIndex] !== sequence ||
      viewEpoch.current !== expectedViewEpoch
    )
      return;
    if (resolved.state !== 'available') {
      throw new Error(
        resolved.state === 'target_trashed'
          ? t('引用目标在回收站中，请先恢复。', 'This reference is in Trash. Restore it first.')
          : t(
              '无法定位这处引用，目标可能已移除或更新。',
              'This reference cannot be located. Its target may have been removed or changed.',
            ),
      );
    }
    if (resolved.source_target) {
      const location = resolved.source_target;
      if (location.locator_status === 'unavailable')
        throw new Error(
          t(
            '引用的原文位置已不可用，请检查来源版本或重新选择目标。',
            'The original reference position is unavailable. Check the source version or select the target again.',
          ),
        );
      if (evidence?.parse_id && location.parse_id !== evidence.parse_id)
        throw new Error(
          t(
            '证据所属的原始解析版本无法定位。',
            'The original parse for this evidence cannot be located.',
          ),
        );
      await openSource(
        location.source_id,
        location.source_version_id,
        paneIndex,
        location,
        sequence,
        expectedViewEpoch,
      );
      return;
    }
    if (!(await openNote(target.target_id, paneIndex, sequence, expectedViewEpoch))) return;
    if (
      keyOf(activeRef.current) !== keyOf(scope) ||
      navigationSequence.current[paneIndex] !== sequence ||
      viewEpoch.current !== expectedViewEpoch
    )
      return;
    setReferenceLocations((old) => ({
      ...old,
      [paneIndex]: { requestId: crypto.randomUUID(), target, occurrenceId },
    }));
  };
  function beginKnowledgeTask(targets: TaskMaterialRequest['targets']) {
    if (!identity || !targets.length) return;
    const request: TaskMaterialRequest = {
      id: crypto.randomUUID(),
      identity: { ...identity },
      targets: structuredClone(targets),
    };
    setView('applications', () => {
      setAnswerId(null);
      setInferenceScope(null);
      setTaskMaterialRequest(request);
      updatePageNavigation('knowledge-task.location', () => ({ mode: 'compose', id: null }));
    });
  }
  function openKnowledgeTask(taskId: string) {
    setView('applications', () => {
      setAnswerId(null);
      setInferenceScope(null);
      setTaskMaterialRequest(null);
      updatePageNavigation('knowledge-task.location', () => ({ mode: 'task', id: taskId }));
    });
  }
  const openEvidence = (evidence: SearchEvidence) =>
    openReference(
      sessionRef.current?.layout.active_pane ?? 0,
      evidence.annotation
        ? {
            target_id: evidence.annotation.annotation_id,
            target_revision_id: evidence.target.content_revision,
            target_block_id: null,
          }
        : {
            target_id: evidence.target.object_id,
            target_revision_id: evidence.target.content_revision,
            target_block_id: evidence.target.block_id,
          },
      undefined,
      evidence.target,
    );
  function openAnswer(id: string) {
    setView('answers', () => {
      setAnswerId(id);
      setPageNavigation((previous) => ({ ...previous, [`ai.strict/${id}`]: true }));
    });
  }
  function openKnowledgeSearch(collectionId: string | null) {
    setView('knowledge', () => {
      setInferenceScope(null);
      setPageNavigation((previous) => ({
        ...previous,
        'knowledge.mode': 'all',
        'knowledge.category': collectionId,
        'knowledge.query': '',
        'knowledge.search-mode': 'exact',
      }));
    });
  }
  function openCollectionQuery(collectionId: string, mode: 'search' | 'answers') {
    if (mode === 'search') {
      openKnowledgeSearch(collectionId);
      return;
    }
    setView('answers', () => {
      setInferenceScope({
        object_ids: [],
        kinds: [],
        tag_ids: [],
        collection_id: collectionId,
        inbox_state: null,
        reading_status: null,
        source_format: null,
        archived: null,
      });
      setAnswerId(null);
      setPageNavigation((previous) => ({ ...previous, 'ai.strict/new': true }));
    });
  }
  function openAdvancedSearch(collectionId: string | null) {
    setView('search', () => {
      setInferenceScope({
        object_ids: [],
        kinds: [],
        tag_ids: [],
        collection_id: collectionId,
        inbox_state: null,
        reading_status: null,
        source_format: null,
        archived: null,
      });
    });
  }
  function openProposal(id: string) {
    setView('proposal', () => setProposalId(id));
  }
  async function withSavedNote(
    noteId: string | null,
    action: () => Promise<void>,
    purpose: 'proposal' | 'relation' = 'proposal',
  ) {
    const scope = identity;
    let epoch = viewEpoch.current;
    const queue = layoutQueue.current;
    if (!scope) throw new Error(t('请先打开知识库。', 'Open a workspace first.'));
    const current = () =>
      keyOf(activeRef.current) === keyOf(scope) &&
      viewEpoch.current === epoch &&
      layoutQueue.current === queue &&
      keyOf(sessionRef.current?.identity ?? null) === keyOf(scope);
    if (!queue || !current() || transitionRef.current)
      throw new Error(
        t(
          purpose === 'relation'
            ? '页面或知识库已变化，请重新打开画布。'
            : '页面或知识库已变化，请重新打开提案。',
          purpose === 'relation'
            ? 'The page or workspace changed. Reopen the canvas.'
            : 'The page or workspace changed. Reopen the proposal.',
        ),
      );
    if (noteId) await controllerFor(noteId);
    if (!current() || transitionRef.current)
      throw new Error(
        t(
          purpose === 'relation'
            ? '页面或知识库已变化，请重新打开画布。'
            : '页面或知识库已变化，请重新打开提案。',
          purpose === 'relation'
            ? 'The page or workspace changed. Reopen the canvas.'
            : 'The page or workspace changed. Reopen the proposal.',
        ),
      );
    const finish = beginTransition(
      purpose === 'relation'
        ? t('正在保存笔记并核对关系…', 'Saving notes and checking the relation…')
        : t('正在保存并应用提案…', 'Saving and applying the proposal…'),
    );
    epoch = viewEpoch.current;
    let drain: ((release: boolean) => void) | undefined;
    try {
      drain = await registry.current.freezeAndFlush();
      if (!current()) throw new Error(t('页面或知识库已变化。', 'The page or workspace changed.'));
      await flushSourceReadings(scope);
      if (!current()) throw new Error(t('页面或知识库已变化。', 'The page or workspace changed.'));
      await queue.flush();
      if (!current()) throw new Error(t('页面或知识库已变化。', 'The page or workspace changed.'));
      await action();
    } finally {
      try {
        drain?.(true);
        if (drain && keyOf(activeRef.current) === keyOf(scope)) {
          for (const unsubscribe of controllerSubscriptions.current.values()) unsubscribe();
          controllerSubscriptions.current.clear();
          setDraftTitles({});
          setEditorEpoch((epoch) => epoch + 1);
          saved();
        }
      } finally {
        finish();
      }
    }
  }
  const controllerFor = useCallback(
    (noteId: string) => {
      if (!identity)
        return Promise.reject(new Error(t('请先打开知识库。', 'Open a workspace first.')));
      return registry.current
        .get(identity, noteId, () =>
          call({ kind: 'notes_get', payload: { note_id: noteId } }, 'note', identity).then(
            (note) => {
              if (note.trashed)
                throw new Error(
                  t(
                    '这篇笔记已移入回收站，请恢复后继续编辑。',
                    'This note is in Trash. Restore it before editing.',
                  ),
                );
              return DraftController.open(identity, note, indexedDrafts, (commandId, request) =>
                call(
                  { kind: 'notes_save', payload: { command_id: commandId, note: request } },
                  'note_commit',
                  identity,
                ),
              );
            },
          ),
        )
        .then((controller) => {
          if (
            keyOf(activeRef.current) !== keyOf(identity) ||
            controllerSubscriptions.current.has(controller)
          )
            return controller;
          let lastRevision = controller.getSnapshot().record.base.revision_id;
          let lastTitle: string | undefined;
          const observe = () => {
            if (keyOf(activeRef.current) !== keyOf(identity)) return;
            const { status, record } = controller.getSnapshot();
            if (record.title !== lastTitle) {
              lastTitle = record.title;
              setDraftTitles((old) => ({ ...old, [record.base.id]: record.title }));
            }
            if (status === 'saved' && record.base.revision_id !== lastRevision) {
              lastRevision = record.base.revision_id;
              setOrganizationVersion((version) => version + 1);
              refreshNotesRef.current();
              void call(
                {
                  kind: 'sync_auto_push_note',
                  payload: { note_id: record.base.id },
                },
                'sync_auto_push',
                identity,
                30_000,
              )
                .then((result) => {
                  if (result.conflict) {
                    setError(
                      t(
                        '笔记已保存到本机，但同步发现冲突；请在“备份与同步”页比较版本。',
                        'The note is saved locally, but sync reported a conflict. Compare versions in Backup and sync.',
                      ),
                    );
                  }
                })
                .catch((error) => {
                  if (keyOf(activeRef.current) !== keyOf(identity)) return;
                  setError(
                    t(
                      '笔记已保存到本机，但自动同步失败：',
                      'Saved locally, but automatic sync failed: ',
                    ) + message(error),
                  );
                });
            }
          };
          controllerSubscriptions.current.set(controller, controller.subscribe(observe));
          observe();
          return controller;
        });
    },
    [identity],
  );
  const metadataFor = useCallback(
    async (noteId: string) => {
      if (!identity) throw new Error(t('请先打开知识库。', 'Open a workspace first.'));
      return getMetadataController(await controllerFor(noteId), identity);
    },
    [controllerFor, identity, t],
  );
  const activeTabsKey = session?.layout.panes.map((pane) => pane.active_object_id ?? '').join('/');
  useEffect(() => {
    application.current?.querySelectorAll('.tab.active').forEach((tab) => {
      tab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    });
  }, [activeTabsKey, gallery, view]);
  async function createNote(
    collectionId?: string,
    collectionRowVersion?: number,
    objectType?: string,
    templateId?: string,
  ) {
    const actions = creation.controller;
    if (
      !identity ||
      !actions ||
      !sessionRef.current ||
      busy ||
      transitionRef.current ||
      !lifecycleReady
    )
      return;
    const scope = identity;
    const paneIndex = sessionRef.current.layout.active_pane;
    const expectedViewEpoch = viewEpoch.current;
    const sequence = (navigationSequence.current[paneIndex] ?? 0) + 1;
    navigationSequence.current[paneIndex] = sequence;
    const current = () =>
      keyOf(activeRef.current) === keyOf(scope) &&
      !transitionRef.current &&
      viewEpoch.current === expectedViewEpoch &&
      navigationSequence.current[paneIndex] === sequence;
    setBusy(true);
    setError(null);
    try {
      const template = templateId
        ? noteTemplates.find((candidate) => candidate.id === templateId)
        : null;
      if (templateId && !template)
        throw new Error(t('所选模板已不存在，请重新选择。', 'That template no longer exists.'));
      const prepared = template
        ? (await import('./organization/template-document')).createTemplatedNoteDocument(
            template,
            t,
            language,
            customObjectTypes,
          )
        : {
            title: objectTypeDefaultTitle(objectType, t, customObjectTypes),
            document: createObjectTypeDocument(objectType, t, customObjectTypes),
          };
      const created = await actions.create({
        kind: 'notes_create',
        payload: {
          command_id: crypto.randomUUID(),
          note: {
            title: prepared.title,
            ...(collectionId !== undefined
              ? {
                  collection_id: collectionId,
                  base_collection_row_version: collectionRowVersion,
                }
              : {}),
            document_schema: EDITOR_DOCUMENT_SCHEMA,
            document: prepared.document,
          },
        },
      });
      if (!created) return;
      if (!current()) return;
      await notes.loadFirst();
      if (!current()) return;
      setInbox(null);
      setTags([]);
      setFilter('');
      if (!(await openNote(created.note.id, paneIndex, sequence, expectedViewEpoch))) return;
      if (!current()) return;
      await actions.acknowledgeOpened(created.note.id);
      focusActiveNoteTitle({ select: true });
    } catch (error) {
      if (current()) setError(message(error));
    } finally {
      setBusy(false);
    }
  }
  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return;
      if (isTabCloseKey(event)) {
        if (activeLocalDocument) {
          if (
            !busy &&
            !openingDocuments &&
            !transitionRef.current &&
            !document.querySelector('dialog[open], [role="menu"], [role="listbox"]')
          ) {
            event.preventDefault();
            void closeLocalDocument(activeLocalDocument.token).catch(failed);
          }
          return;
        }
        handleTabCloseKey(event, {
          layout: sessionRef.current?.layout ?? null,
          contentActive: (view === 'notes' || view === 'reader') && !gallery,
          blocked:
            !!transitionRef.current ||
            busy ||
            !lifecycleReady ||
            !!document.querySelector(shortcutBarrier),
          close: (paneIndex, objectId) => void closeTab(paneIndex, objectId),
        });
        return;
      }
      if (
        transitionRef.current ||
        event.isComposing ||
        !(event.ctrlKey || event.metaKey || event.altKey) ||
        document.querySelector(shortcutBarrierOutsideDrawer)
      )
        return;
      if (matchShortcut('Ctrl+Shift+P', event)) {
        event.preventDefault();
        setCommandPaletteOpen(true);
        return;
      }
      if (matchShortcut('Ctrl+,', event)) {
        event.preventDefault();
        openSettings();
      }
      if (matchShortcut('Ctrl+\\', event)) {
        event.preventDefault();
        toggleSidebar();
      }
      // Both are user-choosable in Settings; the stored value is the single
      // source for what fires here.
      if (
        matchShortcut(
          normalizedShortcut(prefs?.new_note_shortcut, DEFAULT_NEW_NOTE_SHORTCUT),
          event,
        )
      ) {
        event.preventDefault();
        void createNote();
      }
      if (matchShortcut('Ctrl+O', event)) {
        event.preventDefault();
        void openDocuments();
      }
      if (matchShortcut('Ctrl+S', event)) {
        event.preventDefault();
        void flushAll().catch((error) => setError(message(error)));
      }
      if (
        matchShortcut(normalizedShortcut(prefs?.search_shortcut, DEFAULT_SEARCH_SHORTCUT), event)
      ) {
        event.preventDefault();
        if (gallery) {
          setView('notes', () => {
            setFocusMode(false);
            setFocusNoteFilter(true);
          });
        } else revealNoteList(false, true);
      }
    }
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  });
  useEffect(() => {
    if (!focusNoteFilter || view !== 'notes' || loading || focusMode) return;
    const field = document.getElementById('note-filter');
    if (field) {
      field.focus();
      setFocusNoteFilter(false);
    }
  }, [focusNoteFilter, view, loading, focusMode, gallery, noteListOpen, listDrawerOpen]);
  async function bootstrap(create: boolean, recent?: WorkspaceIdentity) {
    let finish: (() => void) | undefined;
    let drain: ((release: boolean) => void) | undefined;
    const previous = keyOf(activeRef.current);
    setBusy(true);
    setError(null);
    async function confirm<T>(action: () => Promise<T>, accepting = false): Promise<T> {
      for (;;) {
        try {
          const result = await action();
          setTransitionError(null);
          return result;
        } catch (error) {
          // These failures precede acceptance, including idempotent receipt lookup.
          if (
            accepting &&
            error instanceof ServiceError &&
            [
              'invalid_input',
              'input_unavailable',
              'unsupported_api',
              'idempotency_mismatch',
            ].includes(error.details.code)
          )
            throw error;
          setTransitionError(
            t('结果尚未确认：', 'Result not confirmed: ') +
              message(error) +
              t(
                ' 已保存的草稿仍保留，请重查后继续。',
                ' Saved drafts are kept; re-check and continue.',
              ),
          );
          await new Promise<void>((resolve) => {
            retryTransition.current = resolve;
          });
          retryTransition.current = null;
          setTransitionError(null);
        }
      }
    }
    try {
      finish = beginTransition(t('正在保存并准备工作区…', 'Saving and preparing the workspace…'));
      drain = await registry.current.freezeAndFlush();
      await flushLayout();
      const grant = recent
        ? await call(
            { kind: 'workspace_select', payload: { target: recent } },
            'resource_grant',
            identity,
          )
        : await choose(create ? 'create_workspace' : 'open_workspace', identity);
      if (!grant) return;
      const commandId = crypto.randomUUID();
      const accepted = await confirm(
        () =>
          call(
            {
              kind: create ? 'workspace_create' : 'workspace_open',
              payload: { command_id: commandId, token: grant.token },
            },
            'bootstrap',
          ),
        true,
      );
      setTransitionOperation(accepted.operation_id);
      setTransition(t('正在处理工作区…', 'Preparing workspace…'));
      // Events may be missed; query this operation until its actual terminal state.
      let terminal: AppOperation;
      const startedAt = Date.now();
      let cancelObservedAt: number | null = null;
      for (;;) {
        const operation = await confirm(() =>
          call(
            { kind: 'operation_get', payload: { operation_id: accepted.operation_id } },
            'operation',
          ),
        );
        if (operation.cancel_requested) {
          cancelObservedAt ??= Date.now();
          setTransitionCancelState('requested');
        }
        if (!['queued', 'running'].includes(operation.status)) {
          terminal = operation;
          break;
        }
        if (Date.now() >= operationWaitDeadline(startedAt, cancelObservedAt)) {
          throw new Error(
            cancelObservedAt === null
              ? t(
                  '工作区操作等待超时，已停止本轮轮询。请在任务中心查看操作结果。',
                  'Workspace operation timed out; polling stopped. Check the result in Task center.',
                )
              : t(
                  '取消请求已发送，但内核未在 15 秒内确认终态。已停止等待；请在任务中心查看结果，草稿仍会保留。',
                  'Cancellation was requested, but the kernel did not confirm a final state within 15 seconds. Waiting stopped; check Task center. Drafts are still kept.',
                ),
          );
        }
        await new Promise((resolve) => setTimeout(resolve, WORKSPACE_OPERATION_POLL_MS));
      }
      const confirmed = await confirm(() => call({ kind: 'app_state' }, 'app_state'));
      ++refreshSequence.current;
      activeRef.current = confirmed.active?.identity ?? null;
      setApp(confirmed);
      if (terminal.status === 'failed')
        throw new Error(
          operationErrorMessage(terminal.error_code, language, terminal.kind) +
            t('原有草稿仍保留。', ' Existing drafts are kept.'),
        );
    } catch (error) {
      setError(message(error));
    } finally {
      drain?.(previous !== keyOf(activeRef.current));
      finish?.();
      setBusy(false);
    }
  }
  async function cancelTransitionOperation() {
    if (!transitionOperation || transitionCancelState !== 'idle') return;
    setTransitionCancelState('requesting');
    setTransitionError(null);
    try {
      const operation = await call(
        { kind: 'operation_cancel', payload: { operation_id: transitionOperation } },
        'operation',
        null,
        4_000,
      );
      setTransitionCancelState('requested');
      setTransition(
        operation.status === 'canceled'
          ? t('取消已确认，正在关闭提示…', 'Cancellation confirmed. Closing…')
          : operation.status === 'succeeded'
            ? t('操作已完成，正在读取结果…', 'The operation finished. Reading the result…')
            : operation.status === 'failed'
              ? t('操作未完成，正在读取失败原因…', 'The operation failed. Reading the reason…')
              : ['queued', 'running'].includes(operation.status)
                ? t(
                    '取消请求已发送，正在等待内核确认…',
                    'Cancellation requested. Waiting for confirmation…',
                  )
                : t('取消请求已发送。', 'Cancellation requested.'),
      );
    } catch (error) {
      setTransitionCancelState('idle');
      setTransitionError(t('取消请求未确认：', 'Cancellation not confirmed: ') + message(error));
    }
  }
  async function withSavedWorkspace(
    scope: WorkspaceIdentity,
    action: () => Promise<void>,
    message?: string,
  ) {
    if (keyOf(activeRef.current) !== keyOf(scope))
      throw new Error(
        t(
          '知识库已切换，请在原知识库中重试。',
          'The workspace changed. Retry in the original workspace.',
        ),
      );
    const finish = beginTransition(
      message ?? t('正在保存并准备任务…', 'Saving and preparing the task…'),
    );
    let drain: ((release: boolean) => void) | undefined;
    try {
      drain = await registry.current.freezeAndFlush();
      await flushLayout();
      if (keyOf(activeRef.current) !== keyOf(scope))
        throw new Error(t('知识库已切换。', 'The workspace changed.'));
      await action();
    } finally {
      drain?.(false);
      finish();
    }
  }
  async function duplicateNote(controller: DraftController) {
    const scope = identity;
    if (!scope) return;
    let copyId: string | null = null;
    await withSavedWorkspace(
      scope,
      async () => {
        const note = controller.getSnapshot().record.base;
        const created = await call(
          {
            kind: 'notes_copy',
            payload: {
              command_id: crypto.randomUUID(),
              note: {
                note_id: note.id,
                source_revision_id: note.revision_id,
                title: duplicateTitle(note.title, t('副本', 'copy')),
              },
            },
          },
          'note_commit',
          scope,
        );
        copyId = created.note.id;
      },
      t('正在创建副本…', 'Creating a copy…'),
    );
    if (!copyId) return;
    await notes.loadFirst();
    await openNote(copyId);
  }
  async function exportNote(
    controller: DraftController,
    format: ExportFormat,
    citationStyleId: string | null,
  ) {
    const scope = identity;
    if (!scope) return;
    await portableActions.load();
    const pendingExport = () =>
      portableActions
        .getSnapshot()
        .pending.some(
          (action) =>
            keyOf(action.identity) === keyOf(scope) &&
            action.command.kind === 'note_export' &&
            action.command.payload.note_id === controller.getSnapshot().record.base.id,
        );
    if (pendingExport()) {
      openTaskCenter({ section: 'exports' });
      return;
    }
    const grant = await choose('export_file', scope);
    if (!grant) return;
    await withSavedWorkspace(scope, async () => {
      if (pendingExport()) {
        openTaskCenter({ section: 'exports' });
        return;
      }
      const note = controller.getSnapshot().record.base;
      const id = crypto.randomUUID();
      const accepted = await portableActions.submit({
        id,
        identity: scope,
        createdAt: new Date().toISOString(),
        displayName: grant.display_name,
        command: {
          kind: 'note_export',
          payload: {
            command_id: id,
            token: grant.token,
            note_id: note.id,
            revision_id: note.revision_id,
            format,
            citation_style_id: citationStyleId,
          },
        },
      });
      openTaskCenter(acceptedTaskDestination(accepted));
    });
  }
  async function forgetWorkspace(target: WorkspaceIdentity) {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      // Hiding a workspace from the recent list keeps its identity, drafts and
      // files untouched; opening it again restores the entry.
      await call({ kind: 'workspace_forget' }, 'done', target);
      await refresh();
    } catch (error) {
      setError(message(error));
    } finally {
      setBusy(false);
    }
  }
  async function closeWorkspace() {
    if (!identity) return;
    let finish: (() => void) | undefined;
    let drain: ((release: boolean) => void) | undefined;
    let closed = false;
    try {
      finish = beginTransition(t('正在保存并关闭知识库…', 'Saving and closing the workspace…'));
      drain = await registry.current.freezeAndFlush();
      await flushLayout();
      await call({ kind: 'workspace_close' }, 'done', identity);
      closed = true;
      await refresh();
    } catch (error) {
      setError(message(error));
    } finally {
      drain?.(closed);
      finish?.();
    }
  }
  async function changeNoteTrash(noteId?: string, value = true) {
    const actions = trashActions.controller;
    const scope = identity;
    if (!scope || !actions || transitionRef.current) return;
    const pending = actions.getSnapshot().pending?.command;
    if (noteId && pending) return;
    if (!noteId && pending?.kind !== 'objects_set_trashed') return;
    const targetId =
      noteId ?? (pending?.kind === 'objects_set_trashed' ? pending.payload.request.object_id : '');
    let finish: (() => void) | undefined;
    let drain: ((release: boolean) => void) | undefined;
    let attempted = false;
    try {
      finish = beginTransition(t('正在保存并更新回收站…', 'Saving and updating Trash…'));
      // A card may have a durable draft from before restart without a mounted editor.
      if (noteId && value) await controllerFor(noteId);
      drain = await registry.current.freezeAndFlush();
      await flushLayout(true);
      const metadata = noteId
        ? await call(
            { kind: 'objects_metadata_get', payload: { object_id: noteId } },
            'object_metadata',
            scope,
          )
        : null;
      attempted = true;
      const result = await actions.run(
        metadata
          ? {
              kind: 'objects_set_trashed',
              payload: {
                command_id: crypto.randomUUID(),
                request: { object_id: metadata.id, base_row_version: metadata.row_version, value },
              },
            }
          : undefined,
      );
      if (result) {
        await updateLayout((layout) => withoutNote(layout, targetId), true);
      }
    } catch (error) {
      failed(error);
    } finally {
      // A response may be lost after acceptance. Reopen from current persisted state;
      // pending intents gate all editors until the same command is confirmed.
      if (attempted && drain) {
        for (const unsubscribe of controllerSubscriptions.current.values()) unsubscribe();
        controllerSubscriptions.current.clear();
        setEditorEpoch((value) => value + 1);
        setDraftTitles({});
      }
      try {
        drain?.(attempted);
      } catch (error) {
        failed(error);
      } finally {
        finish?.();
        if (attempted) saved();
      }
    }
  }
  const rememberTitle = useCallback((noteId: string, title: string) => {
    setTitles((old) => (old[noteId] === title ? old : { ...old, [noteId]: title }));
  }, []);
  async function updatePreferences(change: Partial<AppPreferences>): Promise<boolean> {
    if (!app || preferenceBusy) return false;
    const ordinary: Partial<AppPreferences> = { ...change };
    const hasObjectTypes = Object.prototype.hasOwnProperty.call(ordinary, 'object_type_registry');
    const hasTemplates = Object.prototype.hasOwnProperty.call(ordinary, 'note_template_registry');
    if (hasObjectTypes) delete ordinary.object_type_registry;
    if (hasTemplates) delete ordinary.note_template_registry;
    const hasOrdinary = Object.keys(ordinary).length > 0;
    if (!hasObjectTypes && !hasTemplates && !hasOrdinary) return true;
    setPreferenceBusy(true);
    try {
      if (hasObjectTypes || hasTemplates) {
        const metadata = app.workspace_metadata;
        if (!identity || !metadata)
          throw new Error(t('请先打开知识库再编辑模板或对象类型。', 'Open a workspace first.'));
        const updated = await call(
          {
            kind: 'workspace_metadata_update',
            payload: {
              request: {
                base_row_version: metadata.row_version,
                object_type_registry: change.object_type_registry ?? metadata.object_type_registry,
                note_template_registry:
                  change.note_template_registry ?? metadata.note_template_registry,
              },
            },
          },
          'workspace_metadata',
          identity,
        );
        setApp((current) => (current ? { ...current, workspace_metadata: updated } : current));
        void call({ kind: 'sync_auto_push_workspace_metadata' }, 'sync_auto_push', identity)
          .then((result) => {
            if (result.conflict)
              setPreferenceError(
                t(
                  '模板与对象类型已保存到本机，但远端版本也发生了变化；请在“备份与同步”中核对后重新保存。',
                  'Templates and object types were saved locally, but the remote version also changed. Review them in Backup and sync, then save again.',
                ),
              );
          })
          .catch((syncFailure) =>
            setPreferenceError(
              t('模板与对象类型已保存，但同步失败：', 'Saved locally, but sync failed: ') +
                message(syncFailure),
            ),
          );
      }
      if (hasOrdinary) {
        const preferences = await call(
          {
            kind: 'preferences_update',
            payload: {
              base_state_version: app.preferences.state_version,
              preferences: { ...app.preferences.preferences, ...ordinary },
            },
          },
          'preferences',
        );
        setApp((current) => (current ? { ...current, preferences } : current));
      }
      setPreferenceError(null);
      return true;
    } catch (error) {
      setPreferenceError(message(error));
      return false;
    } finally {
      setPreferenceBusy(false);
    }
  }
  useEffect(() => {
    if (!prefs) return;
    document.documentElement.dataset.theme = prefs.theme;
    document.documentElement.lang = language;
    document.title =
      language === 'en' ? '跨索 spanseek · Personal knowledge' : '跨索 spanseek · 个人知识库';
    document.documentElement.style.setProperty('--ui-size', `${prefs.ui_font_size}px`);
    document.documentElement.style.setProperty('--editor-size', `${prefs.editor_font_size}px`);
    // Kernel error sentences are Chinese only; helpers outside React resolve the
    // message from the code, so they need to know the active language too.
    setActiveLanguage(language);
  }, [prefs, language]);
  async function importLocalDocuments(documents: LocalDocument[]) {
    const scope = activeRef.current;
    if (!scope || documents.length === 0) return;
    if (sourceImportRequest || openingDocuments || transitionRef.current)
      throw new Error(
        t(
          '请先处理当前导入，再打开下一批资料。已选文件仍保留在“已打开资料”中。',
          'Finish the current import before opening another batch. Selected files remain in Opened documents.',
        ),
      );
    setOpeningDocuments(true);
    try {
      let grants: ResourceGrant[] = [];
      await withSavedWorkspace(scope, async () => {
        grants = await grantOpenDocuments(
          scope,
          documents.map((document) => document.token),
        );
      });
      if (keyOf(activeRef.current) !== keyOf(scope)) return;
      grants.forEach((grant, index) =>
        promotedDocuments.current.set(grant.token, {
          document: documents[index],
          position: localPositions.current[documents[index].token],
          identity: scope,
        }),
      );
      setView('sources', () => setSourceImportRequest({ id: crypto.randomUUID(), grants }));
    } finally {
      setOpeningDocuments(false);
    }
  }
  async function acceptLocalDocuments(result: LocalDocuments) {
    const documents = [...new Map(result.documents.map((item) => [item.token, item])).values()];
    if (result.failures.length)
      setError(
        result.failures
          .map(
            (failure) =>
              failure.display_name +
              ': ' +
              (failure.error.code === 'invalid_input'
                ? t(
                    '请选择 PDF、Markdown、TXT、DOCX、DOC、EPUB、MOBI、CSV、XLSX、XLS 或 PPTX 文件，每批最多 100 个。',
                    'Choose PDF, Markdown, TXT, DOCX, DOC, EPUB, MOBI, CSV, XLSX, XLS or PPTX files, up to 100 per batch.',
                  )
                : t(
                    '无法读取文件，请检查文件是否存在、有效且不超过 256 MB。',
                    'Cannot read this file. Check that it exists, is valid, and is no larger than 256 MB.',
                  )),
          )
          .join('\n'),
      );
    if (!documents.length) return;
    setLocalDocuments((old) => [
      ...old,
      ...documents.filter((document) => !old.some((item) => item.token === document.token)),
    ]);
    if (transitionRef.current || document.querySelector(modalGuard)) {
      setError(
        t(
          '资料已接收，请完成当前操作后从“已打开资料”继续。',
          'Documents received. Finish the current action, then continue from Opened documents.',
        ),
      );
      return;
    }
    if (!activeRef.current) {
      ++viewEpoch.current;
      setLocalDocumentToken(documents[0].token);
      return;
    }
    await importLocalDocuments(documents);
  }
  /**
   * `pdfOnly` is the welcome screen's explicitly labelled "打开 PDF" shortcut.
   * Every generic entry point (sidebar, Ctrl+O, drop target) offers every
   * importable format, because the temporary reader turns a non-PDF into the
   * same "save it to a workspace" step whether it arrived by drop or by dialog.
   */
  async function openDocuments(pdfOnly = false) {
    if (openingDocuments || transitionRef.current) return;
    setOpeningDocuments(true);
    let result: LocalDocuments | undefined;
    try {
      result = await chooseOpenDocuments(pdfOnly);
    } catch (error) {
      failed(error);
    } finally {
      setOpeningDocuments(false);
    }
    if (result) await acceptLocalDocuments(result).catch(failed);
  }
  async function openLocalDocument(token: string | null) {
    if (transitionRef.current) return;
    const epoch = ++viewEpoch.current;
    const scope = activeRef.current;
    const sequences = { ...navigationSequence.current };
    if (token) await flushAll();
    if (
      epoch !== viewEpoch.current ||
      keyOf(activeRef.current) !== keyOf(scope) ||
      transitionRef.current ||
      Object.keys(navigationSequence.current).length !== Object.keys(sequences).length ||
      Object.entries(sequences).some(
        ([pane, sequence]) => navigationSequence.current[Number(pane)] !== sequence,
      ) ||
      (token && !localDocumentsRef.current.some((item) => item.token === token))
    )
      return;
    setSidebarDrawerOpen(false);
    setListDrawerOpen(false);
    setLocalDocumentToken(token);
  }
  async function closeLocalDocument(token: string) {
    ++viewEpoch.current;
    await releaseOpenDocument(token);
    setLocalDocuments((old) => old.filter((document) => document.token !== token));
    setLocalDocumentToken((current) => (current === token ? null : current));
    delete localPositions.current[token];
  }
  function saveLocalDocument(token: string) {
    const document = localDocuments.find((item) => item.token === token);
    if (!document) return;
    if (activeRef.current) void importLocalDocuments([document]).catch(failed);
    else setLocalLibraryDialog(token);
  }
  async function chooseLocalLibrary(create: boolean, recent?: WorkspaceIdentity) {
    const token = localLibraryDialog;
    setLocalLibraryDialog(null);
    await bootstrap(create, recent);
    if (token && activeRef.current) setLocalSaveWhenReady(token);
    else if (token) await openLocalDocument(token);
  }
  useEffect(() => {
    if (
      !localSaveWhenReady ||
      !identity ||
      !session ||
      !lifecycleReady ||
      keyOf(session.identity) !== keyOf(identity)
    )
      return;
    const document = localDocuments.find((item) => item.token === localSaveWhenReady);
    setLocalSaveWhenReady(null);
    if (document) void importLocalDocuments([document]).catch(failed);
  }, [localSaveWhenReady, identity, session, lifecycleReady]);
  async function openImportedSource(
    sourceId: string,
    versionId: string | undefined,
    inputToken: string,
  ) {
    const promoted = promotedDocuments.current.get(inputToken);
    const scope = activeRef.current;
    if (!scope || (promoted && keyOf(promoted.identity) !== keyOf(scope))) return;
    const pane = sessionRef.current?.layout.active_pane ?? 0;
    await openSource(
      sourceId,
      versionId,
      pane,
      undefined,
      undefined,
      viewEpoch.current,
      promoted?.position?.view,
    );
    if (
      keyOf(activeRef.current) !== keyOf(scope) ||
      sessionRef.current?.layout.panes[pane]?.active_object_id !== sourceId
    )
      return;
    if (promoted) {
      await closeLocalDocument(promoted.document.token);
      promotedDocuments.current.delete(inputToken);
    }
  }
  const receiveDocuments = useRef(acceptLocalDocuments);
  receiveDocuments.current = acceptLocalDocuments;
  useEffect(() => {
    if (!isTauri() || loading) return;
    let live = true;
    const drain = async () => {
      const result = await takeOpenDocuments();
      if (live) await receiveDocuments.current(result);
    };
    const listener = listen('spanseek:open-files', () => {
      setDraggingDocuments(false);
      void drain().catch(failed);
    });
    void listener.then(() => drain()).catch(failed);
    const dragListener = getCurrentWebview().onDragDropEvent(({ payload }) =>
      setDraggingDocuments(payload.type === 'enter' || payload.type === 'over'),
    );
    const preventNavigation = (event: DragEvent) => event.preventDefault();
    window.addEventListener('dragover', preventNavigation);
    window.addEventListener('drop', preventNavigation);
    return () => {
      live = false;
      void listener.then((unlisten) => unlisten());
      void dragListener.then((unlisten) => unlisten());
      window.removeEventListener('dragover', preventNavigation);
      window.removeEventListener('drop', preventNavigation);
    };
  }, [loading]);
  async function applyTrashOutcome(outcome: TrashActionOutcome) {
    if (outcome.kind !== 'delete') return;
    const ids = outcome.items
      .filter((item) => item.status === 'done')
      .map((item) => item.object_id);
    if (!ids.length) return;
    await updateLayout((layout) => withoutObjects(layout, ids), true);
    const removed = new Set(ids);
    setTitles((old) => Object.fromEntries(Object.entries(old).filter(([id]) => !removed.has(id))));
    setDraftTitles((old) =>
      Object.fromEntries(Object.entries(old).filter(([id]) => !removed.has(id))),
    );
    setSourceLocations((old) =>
      Object.fromEntries(
        Object.entries(old).filter(([, location]) => !removed.has(location.sourceId)),
      ),
    );
    setReferenceLocations((old) =>
      Object.fromEntries(
        Object.entries(old).filter(([, location]) => !removed.has(location.target.target_id)),
      ),
    );
    setAddingAnnotation(null);
  }
  async function changeTrash(
    operation?: { kind: 'delete'; preview: TrashPreview } | { kind: 'restore'; items: TrashItem[] },
  ) {
    const controller = trashLifecycle.controller;
    const scope = identity;
    if (!controller || !scope || transitionRef.current) return;
    let finish: (() => void) | undefined;
    let thaw: ((release: boolean) => void) | undefined;
    let attempted = false;
    try {
      finish = beginTransition(
        t('正在保存并确认回收站操作…', 'Saving and confirming the Trash action…'),
      );
      thaw = await registry.current.freezeAndFlush();
      await flushLayout(true);
      attempted = true;
      const outcome =
        operation?.kind === 'delete'
          ? await controller.delete(operation.preview)
          : operation?.kind === 'restore'
            ? await controller.restore(operation.items)
            : await controller.retry();
      if (outcome && keyOf(activeRef.current) === keyOf(scope)) await applyTrashOutcome(outcome);
    } catch (error) {
      failed(error);
    } finally {
      if (attempted) {
        for (const unsubscribe of controllerSubscriptions.current.values()) unsubscribe();
        controllerSubscriptions.current.clear();
        setEditorEpoch((value) => value + 1);
      }
      try {
        thaw?.(attempted);
      } catch (error) {
        failed(error);
      }
      finish?.();
      if (attempted) saved();
    }
  }
  function failed(error: unknown) {
    setError(message(error));
  }
  async function visit(direction: -1 | 1) {
    if (activeLocalDocument && !transitionRef.current) {
      await openLocalDocument(null);
      return;
    }
    if (
      visiting.current ||
      transitionRef.current ||
      !identity ||
      !sessionRef.current ||
      document.querySelector(modalGuardOutsideDrawer)
    )
      return;
    const history = visits.current;
    const destination = history.peek(direction);
    if (!destination) return;
    const scope = identity;
    const epoch = ++viewEpoch.current;
    const queue = layoutQueue.current;
    const paneSequences = sessionRef.current.layout.panes.map((_, index) => {
      const sequence = (navigationSequence.current[index] ?? 0) + 1;
      navigationSequence.current[index] = sequence;
      return sequence;
    });
    const current = () =>
      keyOf(activeRef.current) === keyOf(scope) &&
      viewEpoch.current === epoch &&
      layoutQueue.current === queue &&
      paneSequences.every((sequence, index) => navigationSequence.current[index] === sequence) &&
      visits.current === history &&
      !transitionRef.current;
    visiting.current = true;
    try {
      const target = destination.state;
      const content = target.view === 'notes' || target.view === 'reader';
      const applied = await navigateAfterBarriers(
        [
          () => registry.current.flush(),
          () => flushSourceReadings(scope),
          () => queue?.flush() ?? Promise.resolve(),
          async () => {
            if (!content) return;
            const objects = await Promise.all(
              target.panes.flatMap((tab) =>
                tab
                  ? [
                      call(
                        { kind: 'objects_metadata_get', payload: { object_id: tab.object_id } },
                        'object_metadata',
                        scope,
                      ),
                    ]
                  : [],
              ),
            );
            if (objects.some((object) => object.trashed))
              throw new Error(
                t(
                  '历史位置的内容已在回收站，请先恢复。',
                  'This destination is in Trash. Restore it first.',
                ),
              );
          },
        ],
        current,
        async () => {
          if (content)
            await updateLayout((layout) =>
              !current()
                ? layout
                : {
                    ...layout,
                    active_pane: target.activePane,
                    panes: target.panes.map((tab, index) => {
                      const pane = layout.panes[index] ?? {
                        tabs: [],
                        active_object_id: null,
                        scroll_offsets: [],
                      };
                      return {
                        ...pane,
                        active_object_id: tab?.object_id ?? null,
                        tabs:
                          !tab || pane.tabs.some((item) => item.object_id === tab.object_id)
                            ? pane.tabs
                            : [...pane.tabs, tab],
                      };
                    }),
                  },
            );
        },
      );
      if (!applied || !current()) return;
      setSourceLocations(target.sourceLocations);
      setReferenceLocations(
        Object.fromEntries(
          Object.entries(target.referenceLocations).map(([pane, location]) => [
            pane,
            { ...location, requestId: crypto.randomUUID() },
          ]),
        ),
      );
      setInbox(target.inbox);
      setTags(target.tags);
      setFilter(target.filter);
      setTitleQuery(target.filter.trim());
      setSort(target.sort);
      setGallery(target.gallery);
      setAnswerId(target.answerId);
      setInferenceScope(target.inferenceScope);
      setProposalId(target.proposalId);
      setPageNavigation(target.pageState);
      setSidebarDrawerOpen(false);
      setListDrawerOpen(false);
      setViewState(target.view);
      history.accept(destination.id);
      setNavigationAvailability({ back: !!history.peek(-1), forward: !!history.peek(1) });
    } catch (error) {
      if (current()) failed(error);
    } finally {
      if (visits.current === history) visiting.current = false;
    }
  }
  useEffect(() => {
    if (
      !identity ||
      !session ||
      keyOf(session.identity) !== workspaceKey ||
      visiting.current ||
      transitionRef.current
    )
      return;
    const content = view === 'notes' || view === 'reader';
    const panes = session.layout.panes.map(
      (pane) => pane.tabs.find((tab) => tab.object_id === pane.active_object_id) ?? null,
    );
    const key = JSON.stringify([
      view,
      content
        ? [
            session.layout.active_pane,
            panes,
            gallery,
            inbox,
            sourceLocations[session.layout.active_pane],
            referenceLocations[session.layout.active_pane]?.target,
            referenceLocations[session.layout.active_pane]?.occurrenceId,
          ]
        : null,
      view === 'collections' ? (pageNavigation['collections.selected'] ?? null) : null,
      view === 'views' ? (pageNavigation['views.selected'] ?? null) : null,
      view === 'search' || view === 'answers' ? inferenceScope : null,
      view === 'answers'
        ? [
            answerId,
            pageNavigation['chat.selected'] ?? null,
            pageNavigation[`ai.strict/${answerId ?? 'new'}`] ?? !!answerId,
          ]
        : null,
      view === 'proposal' ? proposalId : null,
    ]);
    const location: NavigationLocation = {
      view,
      panes,
      activePane: session.layout.active_pane,
      sourceLocations,
      referenceLocations,
      inbox,
      tags,
      filter,
      sort,
      gallery,
      answerId,
      inferenceScope,
      proposalId,
      pageState: pageNavigation,
    };
    visits.current.observe(
      key,
      location,
      isPaneLayoutChange(visits.current.current()?.state, location),
    );
    const next = { back: !!visits.current.peek(-1), forward: !!visits.current.peek(1) };
    setNavigationAvailability((old) =>
      old.back === next.back && old.forward === next.forward ? old : next,
    );
  }, [
    identity,
    session,
    workspaceKey,
    view,
    inbox,
    tags,
    filter,
    sort,
    gallery,
    sourceLocations,
    referenceLocations,
    answerId,
    inferenceScope,
    proposalId,
    pageNavigation,
  ]);
  const visitRef = useRef(visit);
  visitRef.current = visit;
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      const direction = navigationKey(event);
      if (direction === null || event.defaultPrevented) return;
      event.preventDefault();
      void visitRef.current(direction);
    };
    const mouse = (event: MouseEvent) => {
      const direction = mouseNavigation(event.button);
      if (direction === null) return;
      // Cancel native browser navigation and middle-button defaults at every phase;
      // exactly one mouseup owns the application visit.
      event.preventDefault();
      if (event.type === 'mouseup') void visitRef.current(direction);
    };
    window.addEventListener('keydown', keydown);
    for (const type of ['mousedown', 'mouseup', 'auxclick'] as const)
      window.addEventListener(type, mouse, true);
    return () => {
      window.removeEventListener('keydown', keydown);
      for (const type of ['mousedown', 'mouseup', 'auxclick'] as const)
        window.removeEventListener(type, mouse, true);
    };
  }, []);
  const saved = () => {
    setOrganizationVersion((version) => version + 1);
    void notes.loadFirst().catch(failed);
  };
  function showKnowledgePage(mode: 'all' | 'inbox' = 'all') {
    setView(
      'knowledge',
      () => {
        updatePageNavigation('knowledge.mode', () => mode);
        setInbox(null);
        setTags([]);
        setFocusMode(false);
      },
      mode === 'inbox' ? 'inbox' : 'knowledge',
    );
  }

  function showLibrary(state: InboxState | null = null) {
    setView('notes', () => {
      setInbox(state);
      setTags([]);
      setFocusMode(false);
      setGallery(true);
    });
  }
  function openWorkspaceTab(index: number, tab: OpenTab) {
    if (tab.kind === 'note') {
      void openNote(tab.object_id, index).catch(failed);
    } else if (tab.kind === 'source') {
      void openSource(
        tab.object_id,
        sourceLocations[index]?.sourceId === tab.object_id
          ? sourceLocations[index].versionId
          : undefined,
        index,
      ).catch(failed);
    } else {
      setView('notes', () => {
        setGallery(false);
        void updateLayout((layout) => ({
          ...layout,
          active_pane: index,
          panes: layout.panes.map((pane, paneIndex) =>
            paneIndex === index ? { ...pane, active_object_id: tab.object_id } : pane,
          ),
        })).catch(failed);
      });
    }
  }
  const renderTabTitle = (tab: OpenTab) => (
    <TabTitle
      identity={identity!}
      noteId={tab.object_id}
      kind={tab.kind}
      title={draftTitles[tab.object_id] ?? titles[tab.object_id]}
      onTitle={rememberTitle}
      fallback={tab.kind === 'source' ? t('资料', 'Document') : t('笔记', 'Note')}
    />
  );
  const notesHeading =
    inbox === 'pending'
      ? t('收件箱', 'Inbox')
      : inbox === 'snoozed'
        ? t('稍后整理', 'Later')
        : t('全部笔记', 'All notes');
  const welcomeView = !identity && view === 'notes' && !activeLocalDocument;
  const listRequested =
    view === 'notes' &&
    !!identity &&
    !focusMode &&
    !gallery &&
    noteListOpen &&
    !activeLocalDocument;
  const columns = fitColumns(windowWidth, sidebar && !focusMode && !welcomeView, listRequested, {
    sidebar_width: session?.layout.sidebar_width ?? 216,
    note_list_width: session?.layout.note_list_width ?? null,
    ...columnPreview,
  });
  const sidebarVisible = columns.sidebar > 0;
  const listVisible = columns.list > 0;
  const sidebarOverlay = sidebarDrawerOpen && !columns.sidebarAllowed && !welcomeView && !focusMode;
  const listOverlay = listDrawerOpen && !columns.listAllowed && listRequested;
  const sidebarExpanded = sidebarVisible || sidebarOverlay;
  const sidebarLabel = sidebarExpanded
    ? t('收起侧栏', 'Hide sidebar')
    : t('展开侧栏', 'Show sidebar');
  const knowledgeInboxActive = view === 'knowledge' && pageNavigation['knowledge.mode'] === 'inbox';
  const activeContentId = session?.layout.panes[session.layout.active_pane]?.active_object_id;
  const currentProductSection = productSection(
    view,
    knowledgeInboxActive,
    activeContentId ? contentOrigins[activeContentId] : undefined,
  );
  const knowledgeAllActive = view === 'knowledge' && !knowledgeInboxActive;
  const secondaryNavigationActive = [
    'review',
    'voice',
    'answers',
    'proposal',
    'collections',
    'views',
    'canvas',
    'automation',
  ].includes(view);
  // The list and sidebar can be resized independently of the window. Keep both
  // editors visible only when each has its minimum writing width plus chrome.
  const compactPanes =
    windowWidth - columns.sidebar - columns.list - Number(listVisible) <
    EDITOR_MIN * 2 + paneChrome(sidebarVisible, 2);
  useEffect(() => {
    if (columns.sidebarAllowed || welcomeView || focusMode) setSidebarDrawerOpen(false);
    if (columns.listAllowed || !listRequested) setListDrawerOpen(false);
  }, [columns.sidebarAllowed, columns.listAllowed, welcomeView, focusMode, listRequested]);
  if (!startupReveal || (loading && !app && !startupFailed))
    return (
      <LanguageContext.Provider value={language}>
        <div className="startup-screen" role="status" aria-live="polite">
          <div className="startup-brand">
            <BrandMark size={64} />
            <div className="startup-wordmark">
              <strong>{t('跨索', 'spanseek')}</strong>
              <span>
                {t('spanseek · 本地优先的个人知识库', 'A local-first knowledge workspace')}
              </span>
            </div>
          </div>
          <p className="startup-status">
            <LoaderCircle className="spin" size={16} aria-hidden="true" />
            {t('正在连接本地知识库', 'Connecting to your workspace')}
          </p>
        </div>
      </LanguageContext.Provider>
    );
  return (
    <LanguageContext.Provider value={language}>
      <PageNavigationContext.Provider value={pageNavigationContext}>
        <KnowledgeTaskNavigation.Provider
          value={{ start: beginKnowledgeTask, open: openKnowledgeTask }}
        >
          <div
            ref={application}
            inert={!!transition}
            className={`application ${sidebarVisible ? 'sidebar-visible' : 'sidebar-hidden'} ${compactPanes ? 'compact-panes' : ''} ${(view === 'notes' || view === 'reader') && identity ? 'notes-workbench' : ''}`}
          >
            {sidebarExpanded && (
              <LayoutDrawer
                overlay={sidebarOverlay}
                label={t('工作区导航', 'Workspace navigation')}
                closeLabel={t('关闭导航', 'Close navigation')}
                onDismiss={() => setSidebarDrawerOpen(false)}
              >
                <aside
                  id="workspace-sidebar"
                  className="sidebar resizable"
                  style={
                    sidebarVisible
                      ? { width: columns.sidebar, flexBasis: columns.sidebar }
                      : undefined
                  }
                >
                  <div className="workspace-toolbar">
                    <Tooltip label={t('返回', 'Back')} shortcut="Alt + ←">
                      <button
                        className="icon-button"
                        aria-label={t('返回', 'Back')}
                        disabled={busy || !navigationAvailability.back}
                        onClick={() => void visit(-1)}
                      >
                        <ArrowLeft size={18} />
                      </button>
                    </Tooltip>
                    <Tooltip label={t('前进', 'Forward')} shortcut="Alt + →">
                      <button
                        className="icon-button"
                        aria-label={t('前进', 'Forward')}
                        disabled={busy || !navigationAvailability.forward}
                        onClick={() => void visit(1)}
                      >
                        <ArrowRight size={18} />
                      </button>
                    </Tooltip>
                    <WorkspaceMenu
                      compact
                      app={app}
                      busy={busy || !!transition || loading}
                      onSettings={openSettings}
                      onModels={() => setView('models')}
                      onDiagnostics={() => setView('diagnostics')}
                      onBackup={() => setView('backup')}
                      onOpen={() => void bootstrap(false)}
                      onCreate={() => void bootstrap(true)}
                      onClose={() => void closeWorkspace()}
                      onSelect={(target) => void bootstrap(false, target)}
                    />
                    <span className="spacer" />
                    <Tooltip
                      label={
                        identity
                          ? t('本地知识库已打开', 'Local workspace open')
                          : t('尚未打开知识库', 'No workspace open')
                      }
                    >
                      <span
                        className={`workspace-status ${identity ? 'is-open' : ''}`}
                        role="img"
                        aria-label={
                          identity
                            ? t('本地知识库已打开', 'Local workspace open')
                            : t('尚未打开知识库', 'No workspace open')
                        }
                      />
                    </Tooltip>
                    <Tooltip label={sidebarLabel} shortcut="Ctrl + \">
                      <button
                        className="icon-button"
                        aria-label={sidebarLabel}
                        aria-expanded={sidebarExpanded}
                        aria-controls="workspace-sidebar"
                        onClick={toggleSidebar}
                      >
                        <ChevronsLeft size={20} />
                      </button>
                    </Tooltip>
                  </div>
                  <div className="workspace-capture-row">
                    <div className="workspace-capture-group">
                      <button
                        className="workspace-home"
                        aria-label={
                          identity
                            ? t('返回笔记库', 'Go to notes')
                            : t('选择知识库', 'Choose workspace')
                        }
                        disabled={busy || loading}
                        onClick={() => showLibrary()}
                      >
                        <Home size={17} />
                      </button>
                      <PopupMenu
                        label={t('更多页面', 'More pages')}
                        className={`workspace-page-menu ${view === 'views' || (view === 'notes' && inbox === 'snoozed') ? 'active' : ''}`}
                        placement="bottom-start"
                        trigger={<ChevronDown size={12} />}
                        items={[
                          {
                            id: 'later',
                            label: t('稍后整理', 'Later'),
                            icon: <Clock3 />,
                            disabled: !identity,
                            onSelect: () => showLibrary('snoozed'),
                          },
                          {
                            id: 'views',
                            label: t('保存视图', 'Saved views'),
                            icon: <ListFilter />,
                            disabled: !identity,
                            onSelect: () => setView('views'),
                          },
                          {
                            id: 'graph',
                            label: t('知识图谱', 'Knowledge graph'),
                            icon: <Network />,
                            disabled: !identity,
                            onSelect: () => setView('graph'),
                          },
                          {
                            id: 'canvas',
                            label: t('画布', 'Canvases'),
                            icon: <LayoutGrid />,
                            disabled: !identity,
                            onSelect: () => setView('canvas'),
                          },
                        ]}
                      />
                      <button
                        className="capture-button"
                        aria-label={t('新建笔记', 'New note')}
                        onClick={() => void createNote()}
                        disabled={
                          !identity || !session || !creation.controller || busy || !lifecycleReady
                        }
                        title="Ctrl + N"
                      >
                        <Plus size={16} />
                        <span>{t('新建笔记', 'New note')}</span>
                      </button>
                      <PopupMenu
                        label={t('按类型或模板新建', 'Create from type or template')}
                        className="capture-type-menu"
                        placement="bottom-start"
                        trigger={<ChevronDown size={12} />}
                        disabled={
                          !identity || !session || !creation.controller || busy || !lifecycleReady
                        }
                        items={[
                          ...objectTypeMenuItems(
                            t,
                            (id) => void createNote(undefined, undefined, id),
                            'note',
                            customObjectTypes,
                          ),
                          ...noteTemplates.map((template, index) => ({
                            id: `template-${template.id}`,
                            label: t(template.nameZh, template.nameEn),
                            detail: t(template.descriptionZh, template.descriptionEn),
                            icon: <LayoutTemplate size={16} />,
                            separatorBefore: index === 0,
                            groupLabel: index === 0 ? t('模板', 'Templates') : undefined,
                            onSelect: () =>
                              void createNote(undefined, undefined, undefined, template.id),
                          })),
                        ]}
                      />
                    </div>
                    <Tooltip label={t('搜索知识库', 'Search workspace')}>
                      <button
                        className="workspace-search"
                        aria-label={t('搜索知识库', 'Search workspace')}
                        disabled={!identity}
                        onClick={() => openKnowledgeSearch(null)}
                      >
                        <Search size={19} />
                      </button>
                    </Tooltip>
                  </div>
                  <div
                    className="workspace-quick-actions"
                    aria-label={t('快捷操作', 'Quick actions')}
                  >
                    <button
                      className="secondary-button"
                      disabled={!identity || openingDocuments || busy}
                      onClick={() => void openDocuments()}
                    >
                      <FolderOpen size={15} aria-hidden="true" />
                      {t('打开资料', 'Open files')}
                    </button>
                    <button
                      className="secondary-button"
                      disabled={!identity}
                      aria-pressed={view === 'voice'}
                      onClick={() =>
                        setView('voice', () => updatePageNavigation('voice.selected', () => null))
                      }
                    >
                      <Mic size={15} aria-hidden="true" />
                      {t('语音速记', 'Voice capture')}
                    </button>
                  </div>
                  <ProductNavigation
                    view={view}
                    section={currentProductSection}
                    ready={Boolean(identity)}
                    onNavigate={setView}
                    onKnowledge={showKnowledgePage}
                  />
                  {identity && session && (
                    <WorkspaceTabs
                      panes={session.layout.panes}
                      activePane={session.layout.active_pane}
                      contentActive={(view === 'notes' || view === 'reader') && !gallery}
                      busy={busy || !lifecycleReady}
                      renderTitle={renderTabTitle}
                      onOpen={openWorkspaceTab}
                      onClose={(index, objectId, mode) => void closeTab(index, objectId, mode)}
                      onNew={() => void createNote()}
                    />
                  )}
                  {identity && (
                    <details className="sidebar-tags">
                      <summary>
                        {t('标签', 'Tags')}
                        {tags.length > 0 && <span>{tags.length}</span>}
                      </summary>
                      <TagFilter
                        identity={identity}
                        selected={tags}
                        generation={organizationVersion}
                        showHeading={false}
                        onSelect={(tags) => {
                          setView('notes', () => {
                            setTags(tags);
                            setFocusMode(false);
                            setGallery(true);
                          });
                        }}
                      />
                    </details>
                  )}
                  {identity && (
                    <SyncStatusButton identity={identity} onOpen={() => setView('backup')} />
                  )}
                  <div className="sidebar-bottom">
                    <Tooltip label={t('自动化', 'Automation')}>
                      <button
                        className={`icon-button ${view === 'automation' ? 'active' : ''}`}
                        aria-label={t('自动化', 'Automation')}
                        disabled={!identity}
                        onClick={() => setView('automation')}
                      >
                        <Zap size={18} aria-hidden="true" />
                      </button>
                    </Tooltip>
                    <Tooltip label={t('设置', 'Settings')} shortcut="Ctrl + ,">
                      <button
                        className="icon-button"
                        aria-label={t('设置', 'Settings')}
                        onClick={openSettings}
                      >
                        <Settings2 size={18} />
                      </button>
                    </Tooltip>
                    <Tooltip label={t('备份与恢复', 'Backup & restore')}>
                      <button
                        className="icon-button"
                        aria-label={t('备份与恢复', 'Backup & restore')}
                        onClick={() => setView('backup')}
                      >
                        <Archive size={18} />
                      </button>
                    </Tooltip>
                    <span className="spacer" />
                    <Tooltip label={t('任务中心', 'Activity')}>
                      <button
                        className={`icon-button ${view === 'operations' ? 'active' : ''}`}
                        aria-label={t('任务中心', 'Activity')}
                        onClick={() => openTaskCenter()}
                      >
                        <Activity size={18} />
                      </button>
                    </Tooltip>
                    <Tooltip label={t('回收站', 'Trash')}>
                      <button
                        className={`icon-button ${view === 'trash' ? 'active' : ''}`}
                        aria-label={t('回收站', 'Trash')}
                        disabled={!identity}
                        onClick={() => setView('trash')}
                      >
                        <Trash2 size={18} />
                      </button>
                    </Tooltip>
                  </div>
                </aside>
              </LayoutDrawer>
            )}
            {sidebarVisible && (
              <ColumnResizer
                label={t('调整侧栏宽度', 'Resize sidebar')}
                controls="workspace-sidebar"
                value={columns.sidebar}
                min={160}
                max={columns.sidebarMax}
                disabled={!session || !!transition}
                onPreview={(value) => setColumnPreview((old) => ({ ...old, sidebar_width: value }))}
                onCommit={(value) => void resizeColumn('sidebar_width', value).catch(failed)}
                onCancel={cancelColumnResize}
              />
            )}
            <main className="main-area" data-product-section={currentProductSection}>
              <header className="topbar" hidden={welcomeView}>
                <Tooltip label={sidebarLabel} shortcut="Ctrl + \">
                  <button
                    className="icon-button sidebar-toggle"
                    aria-label={sidebarLabel}
                    aria-expanded={sidebarExpanded}
                    aria-controls="workspace-sidebar"
                    onClick={toggleSidebar}
                  >
                    <SidebarIcon />
                  </button>
                </Tooltip>
                <strong>
                  {activeLocalDocument
                    ? t('本地阅读', 'Local reader')
                    : view === 'knowledge'
                      ? t('知识索引', 'Knowledge index')
                      : view === 'notes'
                        ? notesHeading
                        : view === 'voice'
                          ? t('语音速记', 'Voice capture')
                          : view === 'review'
                            ? t('回顾', 'Review')
                            : view === 'automation'
                              ? t('自动化', 'Automation')
                              : view === 'diagnostics'
                                ? t('统计与诊断', 'Statistics & diagnostics')
                                : view === 'search'
                                  ? t('搜索', 'Search')
                                  : view === 'applications'
                                    ? t('知识任务', 'Knowledge tasks')
                                    : view === 'answers'
                                      ? t('AI 助手', 'AI assistant')
                                      : view === 'proposal'
                                        ? t('审阅提案', 'Review proposal')
                                        : view === 'models'
                                          ? t('模型与检索', 'Models & search')
                                          : view === 'sources' || view === 'reader'
                                            ? t('资料库', 'Library')
                                            : view === 'trash'
                                              ? t('回收站', 'Trash')
                                              : view === 'operations'
                                                ? t('任务中心', 'Activity')
                                                : view === 'graph'
                                                  ? t('知识图谱', 'Knowledge graph')
                                                  : view === 'collections'
                                                    ? t('集合', 'Collections')
                                                    : view === 'views'
                                                      ? t('保存视图', 'Saved views')
                                                      : view === 'canvas'
                                                        ? t('画布', 'Canvases')
                                                        : view === 'backup'
                                                          ? t('备份与恢复', 'Backup & restore')
                                                          : t('设置', 'Settings')}
                </strong>
                <span className="spacer" />
                {view === 'notes' && identity && (
                  <>
                    {!gallery && session?.layout.panes.length === 2 && (
                      <div
                        className="pane-switch view-switch"
                        role="group"
                        aria-label={t('内容面板', 'Content panes')}
                      >
                        {[0, 1].map((index) => (
                          <button
                            key={index}
                            aria-pressed={session.layout.active_pane === index}
                            aria-label={t(`切换到面板 ${index + 1}`, `Switch to pane ${index + 1}`)}
                            onClick={() =>
                              updateLayout((layout) => ({ ...layout, active_pane: index }))
                            }
                          >
                            {index + 1}
                          </button>
                        ))}
                      </div>
                    )}
                    <div
                      className="view-switch"
                      role="group"
                      aria-label={t('浏览方式', 'Browse layout')}
                    >
                      <button
                        aria-label={t('列表视图', 'List view')}
                        title={t('列表视图', 'List view')}
                        aria-pressed={!gallery}
                        onClick={() => revealNoteList()}
                      >
                        <List size={16} />
                      </button>
                      <button
                        aria-label={t('卡片视图', 'Card view')}
                        title={t('卡片视图', 'Card view')}
                        aria-pressed={gallery}
                        onClick={() => {
                          setFocusMode(false);
                          setGallery(true);
                        }}
                      >
                        <LayoutGrid size={15} />
                      </button>
                    </div>
                    {!gallery && (
                      <button
                        className="icon-button"
                        aria-label={
                          focusMode
                            ? t('退出专注模式', 'Leave focus mode')
                            : t('专注模式', 'Focus mode')
                        }
                        title={
                          focusMode
                            ? t('退出专注模式', 'Leave focus mode')
                            : t('专注模式', 'Focus mode')
                        }
                        onClick={() => setFocusMode(!focusMode)}
                      >
                        {focusMode ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
                      </button>
                    )}
                  </>
                )}
              </header>
              {error && (
                <div className="global-error" role="alert">
                  <AlertCircle size={18} />
                  <span>{error}</span>
                  {startupFailed && (
                    <button
                      type="button"
                      className="text-button"
                      onClick={() => {
                        setError(null);
                        void reconnect();
                      }}
                    >
                      {t('重试', 'Retry')}
                    </button>
                  )}
                  <button
                    onClick={() => setError(null)}
                    className="icon-button"
                    aria-label={t('关闭提示', 'Dismiss message')}
                  >
                    <X size={17} />
                  </button>
                </div>
              )}
              {tagSyncError?.scope === keyOf(identity) && (
                <div className="global-error" role="alert">
                  <AlertCircle size={18} />
                  <span>{tagSyncError.text}</span>
                  <button className="text-button" onClick={() => setView('backup')}>
                    {t('查看同步', 'Review sync')}
                  </button>
                  <button
                    className="icon-button"
                    aria-label={t('关闭标签同步提示', 'Dismiss tag sync message')}
                    onClick={() => setTagSyncError(null)}
                  >
                    <X size={17} />
                  </button>
                </div>
              )}
              {!runtimeNoticeClosed && runtimeNotice.level === 'warn' && (
                <div className="global-error" role="alert">
                  <AlertCircle size={18} />
                  <span>{runtimeNotice.text}</span>
                  <button
                    onClick={() => setRuntimeNoticeClosed(true)}
                    className="icon-button"
                    aria-label={t('关闭提示', 'Dismiss message')}
                  >
                    <X size={17} />
                  </button>
                </div>
              )}{' '}
              {!componentNoticeClosed && !!app?.parse_components.length && (
                <div className="global-error" role="alert">
                  <AlertCircle size={18} />
                  <span>
                    {app.parse_components.map((issue) => parseComponentNotice(issue, t)).join(' ')}
                  </span>
                  <button
                    onClick={() => setComponentNoticeClosed(true)}
                    className="icon-button"
                    aria-label={t('关闭提示', 'Dismiss message')}
                  >
                    <X size={17} />
                  </button>
                </div>
              )}
              {identity && sessionError && (
                <PageLoadFailure
                  error={sessionError}
                  retry={() => setSessionAttempt((value) => value + 1)}
                />
              )}
              {!identity && !activeLocalDocument && localDocuments.length > 0 && (
                <div
                  className="local-documents-resume"
                  aria-label={t('已打开资料', 'Opened documents')}
                >
                  <span>{t('继续阅读', 'Continue reading')}</span>
                  {localDocuments.map((document) => (
                    <button
                      className="text-button"
                      key={document.token}
                      title={displayPath(document.location_hint)}
                      onClick={() => void openLocalDocument(document.token).catch(failed)}
                    >
                      {document.display_name}
                    </button>
                  ))}
                </div>
              )}
              {identity && trashActions.state && (
                <ActionNotice state={trashActions.state} retry={() => void changeNoteTrash()} />
              )}
              {identity && trashLifecycle.error && (
                <PageLoadFailure error={trashLifecycle.error} retry={trashLifecycle.reload} />
              )}
              {identity && trashLifecycle.state && (
                <TrashActionNotice
                  state={trashLifecycle.state}
                  onRetry={() => void changeTrash()}
                  onDismiss={() => trashLifecycle.controller?.dismissResult()}
                />
              )}
              {identity && creation.error && (
                <PageLoadFailure error={creation.error} retry={creation.reload} />
              )}
              {identity &&
                creation.state &&
                (creation.state.result ? (
                  <div className="org-notice" role="status">
                    <span>
                      {t(
                        '笔记已创建，可以继续打开。',
                        'Your note was created and is ready to open.',
                      )}
                    </span>
                    <button
                      className="text-button"
                      disabled={busy}
                      onClick={() => void createNote()}
                    >
                      {t('打开笔记', 'Open note')}
                    </button>
                  </div>
                ) : (
                  <ActionNotice state={creation.state} retry={() => void createNote()} />
                ))}
              {loading ? (
                <div className="empty-state">
                  <LoaderCircle className="spin" />
                  <h1>{t('正在连接本地知识库', 'Connecting to your workspace')}</h1>
                </div>
              ) : activeLocalDocument ? (
                <LocalDocumentPage
                  key={activeLocalDocument.token}
                  documents={localDocuments}
                  active={activeLocalDocument}
                  position={localPositions.current[activeLocalDocument.token]}
                  busy={busy || openingDocuments}
                  onSelect={(token) => void openLocalDocument(token).catch(failed)}
                  onPosition={(token, position) => {
                    if (localDocumentsRef.current.some((item) => item.token === token))
                      localPositions.current[token] = position;
                  }}
                  onSave={() => saveLocalDocument(activeLocalDocument.token)}
                  onClose={() => void closeLocalDocument(activeLocalDocument.token).catch(failed)}
                  onBack={() => void openLocalDocument(null).catch(failed)}
                />
              ) : welcomeView ? (
                <WelcomePage
                  workspaces={app?.workspaces ?? []}
                  busy={busy}
                  desktopAvailable={isTauri()}
                  backendUnavailable={!isTauri() || startupFailed}
                  onActivity={() => openTaskCenter()}
                  onSettings={openSettings}
                  onCreate={() => void bootstrap(true)}
                  onOpen={(target) => void bootstrap(false, target)}
                  onForget={(target) => void forgetWorkspace(target)}
                  onRestore={() => setView('backup')}
                  onReconnect={() => void reconnect()}
                  onOpenPdf={() => void openDocuments(true)}
                />
              ) : view === 'tags' && identity ? (
                <TagsPage
                  key={`${workspaceKey}/tags`}
                  identity={identity}
                  refreshVersion={organizationVersion}
                  onOpenObject={(object) => {
                    if (object.kind === 'note') {
                      void openNote(object.id);
                    } else if (object.kind === 'collection') {
                      setView('collections', () => {
                        updatePageNavigation('collections.selected', () => object.id);
                        updatePageNavigation('collections.mode', () =>
                          object.archived ? 'archived' : 'active',
                        );
                      });
                    } else {
                      if (
                        object.kind === 'annotation' &&
                        (!object.source_id || !object.source_version_id)
                      ) {
                        failed(
                          new Error(
                            t(
                              '摘录的来源暂不可用，请重新读取。',
                              'The excerpt source is unavailable. Reload and try again.',
                            ),
                          ),
                        );
                        return;
                      }
                      const target: ResolvedSourceTarget | undefined =
                        object.kind === 'annotation'
                          ? {
                              source_id: object.source_id!,
                              source_version_id: object.source_version_id!,
                              annotation_id: object.id,
                              locator: null,
                              locator_status: null,
                              parse_id: null,
                              block_offset: null,
                            }
                          : undefined;
                      void openSource(
                        object.source_id ?? object.id,
                        object.source_version_id ?? undefined,
                        undefined,
                        target,
                      ).catch(failed);
                    }
                  }}
                  onOpenView={(id) =>
                    setView('views', () => updatePageNavigation('views.selected', () => id))
                  }
                  onOpenLibrary={() => showKnowledgePage('all')}
                  onOpenTrash={() => setView('trash')}
                  onContentChanged={saved}
                  onChanged={(result) => {
                    const tag = result.tag.tag;
                    setTags((current) =>
                      result.deleted
                        ? current.filter((item) => item.id !== tag.id)
                        : current.map((item) => (item.id === tag.id ? tag : item)),
                    );
                    saved();
                  }}
                />
              ) : identity && view === 'automation' ? (
                <Suspense fallback={<p className="muted">{t('正在读取…', 'Loading…')}</p>}>
                  <AutomationPage
                    identity={identity}
                    onOpenNote={(id) => openNote(id)}
                    onOpenTarget={(target) =>
                      void openReference(sessionRef.current?.layout.active_pane ?? 0, target).catch(
                        failed,
                      )
                    }
                  />
                </Suspense>
              ) : identity && view === 'review' ? (
                <Suspense fallback={<p className="muted">{t('正在读取…', 'Loading…')}</p>}>
                  <ReviewPage
                    identity={identity}
                    onOpen={(target) =>
                      void openReference(sessionRef.current?.layout.active_pane ?? 0, target).catch(
                        failed,
                      )
                    }
                  />
                </Suspense>
              ) : identity && ['search', 'answers', 'applications', 'proposal'].includes(view) ? (
                <Suspense fallback={<p className="muted">{t('正在读取…', 'Loading…')}</p>}>
                  {view === 'search' ? (
                    <SearchPage
                      key={`${workspaceKey}/search/${inferenceScope?.collection_id ?? 'all'}`}
                      identity={identity}
                      initialScope={inferenceScope ?? undefined}
                      onOpenEvidence={openEvidence}
                      onOpenAnswer={openAnswer}
                      onOpenModels={() => setView('models')}
                    />
                  ) : view === 'proposal' && proposalId ? (
                    <ProposalPage
                      key={`${workspaceKey}/proposal/${proposalId}`}
                      identity={identity}
                      proposalId={proposalId}
                      onOpenNote={(id) => openNote(id)}
                      withSavedNote={withSavedNote}
                      onOpenTrash={() => setView('trash')}
                      onOpenReference={(target, occurrence) =>
                        openReference(
                          sessionRef.current?.layout.active_pane ?? 0,
                          target,
                          occurrence,
                        )
                      }
                    />
                  ) : (
                    <ReferenceProvider
                      identity={identity}
                      generation={`${workspaceKey}/${organizationVersion}`}
                      onOpenReference={(target, occurrence) =>
                        openReference(
                          sessionRef.current?.layout.active_pane ?? 0,
                          target,
                          occurrence,
                        )
                      }
                    >
                      <AnswersPage
                        key={`${workspaceKey}/answers/${inferenceScope?.collection_id ?? 'all'}/${answerId ?? 'history'}`}
                        taskMode={view === 'applications'}
                        materialRequest={taskMaterialRequest}
                        onMaterialsHandled={(id) =>
                          setTaskMaterialRequest((current) => (current?.id === id ? null : current))
                        }
                        settingsRevision={settingsRevision}
                        onTaskModeChange={(enabled) =>
                          setView(enabled ? 'applications' : 'answers')
                        }
                        onOpenNote={(id) => openNote(id)}
                        identity={identity}
                        answerId={answerId}
                        initialScope={inferenceScope ?? undefined}
                        onSelectAnswer={openAnswer}
                        onOpenEvidence={openEvidence}
                        onOpenProposal={openProposal}
                        onOpenModels={() => setView('models')}
                      />
                    </ReferenceProvider>
                  )}
                </Suspense>
              ) : view === 'knowledge' && identity ? (
                <KnowledgeIndexPage
                  identity={identity}
                  objectTypes={objectTypeOptions}
                  onOpenNote={(id) => void openNote(id)}
                  onOpenSource={(id, version, annotationId) => {
                    const target: ResolvedSourceTarget | undefined =
                      annotationId && version
                        ? {
                            source_id: id,
                            source_version_id: version,
                            annotation_id: annotationId,
                            locator: null,
                            locator_status: null,
                            parse_id: null,
                            block_offset: null,
                          }
                        : undefined;
                    void openSource(id, version, undefined, target).catch(failed);
                  }}
                  onManageCollections={() => setView('collections')}
                  onOpenGraph={(id) => {
                    updatePageNavigation('graph.collection', () => id);
                    setView('graph');
                  }}
                  onOpenCanvas={(id) => {
                    updatePageNavigation('canvas.collection', () => id);
                    setView('canvas');
                  }}
                  onOpenAdvancedSearch={openAdvancedSearch}
                />
              ) : view === 'sources' && identity ? (
                <Suspense
                  fallback={<p className="muted">{t('正在读取资料库…', 'Loading library…')}</p>}
                >
                  <SourcesPage
                    identity={identity}
                    onManageCollections={() => setView('collections')}
                    onOpenSource={(id, version) => void openSource(id, version).catch(failed)}
                    importRequest={sourceImportRequest}
                    onImportRequestHandled={(id) =>
                      setSourceImportRequest((current) =>
                        current?.id === id ? undefined : current,
                      )
                    }
                    onOpenImportedSource={(id, version, token) =>
                      void openImportedSource(id, version, token).catch(failed)
                    }
                    onOpenTasks={() => openTaskCenter({ section: 'sources' })}
                    onChanged={saved}
                  />
                </Suspense>
              ) : view === 'trash' && identity ? (
                <TrashPage
                  identity={identity}
                  generation={organizationVersion}
                  busy={!lifecycleReady || !!transition}
                  state={trashLifecycle.state}
                  onRestore={(items) => changeTrash({ kind: 'restore', items })}
                  onDelete={(preview) => changeTrash({ kind: 'delete', preview })}
                />
              ) : view === 'operations' ? (
                <TaskCenter
                  key={workspaceKey}
                  identity={identity}
                  onOpenWorkspace={(target) => void bootstrap(false, target)}
                  onOpenSource={(id, version) => void openSource(id, version).catch(failed)}
                  onOpenResearch={(taskId) =>
                    setView('automation', () => {
                      updatePageNavigation('automation.tab', () => 'research');
                      updatePageNavigation('automation.researchTask', () => taskId);
                    })
                  }
                />
              ) : view === 'backup' ? (
                <BackupPage
                  identity={identity}
                  withSavedWorkspace={withSavedWorkspace}
                  onTasks={openTaskCenter}
                  onOpenTags={() => setView('tags')}
                  onOpenView={(id) =>
                    setView('views', () => updatePageNavigation('views.selected', () => id))
                  }
                  onOpenTagSyncItem={(snapshot) => {
                    if (!identity) return;
                    if (snapshot.kind === 'tag') {
                      setView('tags', () =>
                        updatePageNavigation('tags.selected', () => snapshot.entity_id),
                      );
                      return;
                    }
                    const payload = snapshot.payload ?? snapshot.pending?.payload;
                    if (
                      payload?.sync_kind === 'tag_assignments' &&
                      payload.assignments.object_kind === 'collection'
                    ) {
                      const scope = identity;
                      const epoch = viewEpoch.current;
                      void call(
                        { kind: 'collections_get', payload: { collection_id: snapshot.entity_id } },
                        'collection',
                        scope,
                      )
                        .then((collection) => {
                          if (
                            keyOf(activeRef.current) !== keyOf(scope) ||
                            viewEpoch.current !== epoch
                          )
                            return;
                          setView('collections', () => {
                            updatePageNavigation('collections.selected', () => collection.id);
                            updatePageNavigation('collections.mode', () =>
                              collection.metadata.archived ? 'archived' : 'active',
                            );
                          });
                        })
                        .catch(failed);
                    } else
                      void openReference(sessionRef.current?.layout.active_pane ?? 0, {
                        target_id: snapshot.entity_id,
                        target_block_id: null,
                        target_revision_id: null,
                      }).catch(failed);
                  }}
                  onHandoffImported={() => {
                    setOrganizationVersion((version) => version + 1);
                    refreshNotesRef.current();
                  }}
                  onConfigurationChanged={async () => {
                    const next = await call({ kind: 'app_state' }, 'app_state');
                    setApp(next);
                    setOrganizationVersion((version) => version + 1);
                  }}
                />
              ) : view === 'voice' && identity ? (
                <VoiceCapturePage
                  key={workspaceKey}
                  identity={identity}
                  onOpenAsrSettings={() => showSettings('asr')}
                  settingsRevision={settingsRevision}
                  onRegisterSave={(flush) => {
                    voiceSaveRef.current = flush;
                    return () => {
                      if (voiceSaveRef.current === flush) voiceSaveRef.current = null;
                    };
                  }}
                  onOpenNote={(id) => void openNote(id)}
                />
              ) : view === 'graph' && identity ? (
                <GraphPage
                  identity={identity}
                  onOpenNote={(id) => void openNote(id)}
                  onOpenTarget={(target) =>
                    openReference(sessionRef.current?.layout.active_pane ?? 0, target)
                  }
                  onOpenCanvas={(id) => {
                    updatePageNavigation('canvas.selected', () => id);
                    setView('canvas');
                  }}
                />
              ) : view === 'collections' && identity ? (
                <CollectionsPage
                  identity={identity}
                  onOpenNote={(id) => void openNote(id)}
                  onOpenTarget={(target) =>
                    openReference(sessionRef.current?.layout.active_pane ?? 0, target)
                  }
                  onSearchCollection={(id) => openCollectionQuery(id, 'search')}
                  onAskCollection={(id) => openCollectionQuery(id, 'answers')}
                  onCreateInCollection={(id, rowVersion) => void createNote(id, rowVersion)}
                  onChanged={saved}
                />
              ) : view === 'views' && identity ? (
                <SavedViewsPage
                  customTypes={customObjectTypes}
                  identity={identity}
                  onOpenNote={(id) => void openNote(id)}
                  onChanged={saved}
                />
              ) : view === 'canvas' && identity ? (
                <CanvasPage
                  identity={identity}
                  onOpenNote={(id) => void openNote(id)}
                  onOpenTarget={(target) =>
                    openReference(sessionRef.current?.layout.active_pane ?? 0, target)
                  }
                  onChanged={() => {
                    setError(null);
                    saved();
                  }}
                  withSavedNote={(id, action) => withSavedNote(id, action, 'relation')}
                  onOpenTrash={() => setView('trash')}
                  registerSave={registerCanvasSave}
                />
              ) : (
                <div className={`workbench ${focusMode ? 'focus-mode' : ''}`}>
                  {!focusMode && view !== 'reader' && (gallery || listVisible || listOverlay) && (
                    <LayoutDrawer
                      overlay={listOverlay}
                      label={t('笔记列表', 'Note list')}
                      closeLabel={t('关闭笔记列表', 'Close note list')}
                      onDismiss={() => setListDrawerOpen(false)}
                    >
                      <LibraryBrowser
                        key={workspaceKey}
                        identity={identity!}
                        toolbar={
                          gallery ? (
                            <>
                              {!sidebarVisible && (
                                <button
                                  className="icon-button"
                                  aria-label={sidebarLabel}
                                  aria-expanded={sidebarExpanded}
                                  aria-controls="workspace-sidebar"
                                  onClick={toggleSidebar}
                                >
                                  <SidebarIcon />
                                </button>
                              )}
                              <div
                                className="view-switch"
                                role="group"
                                aria-label={t('浏览方式', 'Browse layout')}
                              >
                                <button
                                  aria-label={t('列表视图', 'List view')}
                                  aria-pressed={false}
                                  onClick={() => revealNoteList()}
                                >
                                  <List size={16} />
                                </button>
                                <button
                                  aria-label={t('卡片视图', 'Card view')}
                                  aria-pressed={true}
                                  onClick={() => setGallery(true)}
                                >
                                  <LayoutGrid size={15} />
                                </button>
                              </div>
                            </>
                          ) : undefined
                        }
                        recent={
                          inbox === 'pending' ? (
                            <RecentObjects
                              identity={identity!}
                              activeId={
                                session?.layout.panes[session.layout.active_pane]
                                  ?.active_object_id ?? null
                              }
                              busy={busy || !lifecycleReady}
                              onOpen={(id) => void openNote(id)}
                            />
                          ) : undefined
                        }
                        notes={notes}
                        width={listOverlay ? Math.min(320, windowWidth - 16) : columns.list}
                        title={notesHeading}
                        gallery={gallery}
                        filter={filter}
                        objectTypes={objectTypeOptions}
                        typeFilter={typeFilter}
                        onTypeFilter={setTypeFilter}
                        onFilter={setFilter}
                        tags={tags}
                        onClearTag={(id) =>
                          setTags((items) => (id ? items.filter((tag) => tag.id !== id) : []))
                        }
                        bulk={
                          identity
                            ? {
                                identity,
                                getMetadata: metadataFor,
                                onChanged: saved,
                                propertyFields: bulkPropertyFields,
                                customTypes: customObjectTypes,
                              }
                            : undefined
                        }
                        sort={sort}
                        onSort={setSort}
                        activeId={
                          session?.layout.panes[session.layout.active_pane]?.active_object_id ??
                          null
                        }
                        onReload={() => void notes.refresh()}
                        onCreate={() => void createNote()}
                        onOpen={openNote}
                        onTrash={(id) => void changeNoteTrash(id)}
                        busy={busy || !lifecycleReady}
                      />
                    </LayoutDrawer>
                  )}
                  {listVisible && (
                    <ColumnResizer
                      label={t('调整笔记列表宽度', 'Resize note list')}
                      controls="library-note-list"
                      value={columns.list}
                      min={200}
                      max={columns.listMax}
                      disabled={!session || !!transition}
                      onPreview={(value) =>
                        setColumnPreview((old) => ({ ...old, note_list_width: value }))
                      }
                      onCommit={(value) =>
                        void resizeColumn('note_list_width', value).catch(failed)
                      }
                      onCancel={cancelColumnResize}
                    />
                  )}
                  {!gallery &&
                    !lifecycleReady &&
                    !trashActions.state?.pending &&
                    !trashLifecycle.state?.pending && (
                      <PageLoadFailure error={trashActions.error} retry={trashActions.reload} />
                    )}
                  {!gallery && lifecycleReady && (
                    <div
                      inert={busy}
                      aria-busy={busy}
                      className={`pane-host ${session?.layout.panes.length === 2 ? 'split' : ''}`}
                    >
                      {session?.layout.panes.map((pane, index) => (
                        <section
                          className={`content-pane ${session.layout.active_pane === index ? 'active-pane' : ''}`}
                          key={index}
                          onFocusCapture={() => {
                            if (sessionRef.current?.layout.active_pane !== index)
                              updateLayout((layout) => ({ ...layout, active_pane: index }));
                          }}
                        >
                          <div className="pane-toolbar">
                            <Tooltip
                              label={t('返回上一页', 'Back to previous page')}
                              shortcut="Alt + ←"
                            >
                              <button
                                className="icon-button"
                                aria-label={t('返回上一页', 'Back to previous page')}
                                disabled={!navigationAvailability.back || !!transition}
                                onClick={() => void visit(-1)}
                              >
                                <ArrowLeft size={17} aria-hidden="true" />
                              </button>
                            </Tooltip>
                            {!sidebarVisible && (
                              <button
                                className="icon-button"
                                aria-label={sidebarLabel}
                                aria-expanded={sidebarExpanded}
                                aria-controls="workspace-sidebar"
                                onClick={toggleSidebar}
                              >
                                <SidebarIcon />
                              </button>
                            )}
                            <PopupMenu
                              label={t(
                                `面板 ${index + 1} 的文档`,
                                `Documents in pane ${index + 1}`,
                              )}
                              className="pane-document-menu"
                              placement="bottom-start"
                              trigger={
                                <>
                                  <FileText size={16} />
                                  <span>
                                    {pane.tabs.find(
                                      (tab) => tab.object_id === pane.active_object_id,
                                    )
                                      ? renderTabTitle(
                                          pane.tabs.find(
                                            (tab) => tab.object_id === pane.active_object_id,
                                          )!,
                                        )
                                      : t('打开的文档', 'Open documents')}
                                  </span>
                                  <ChevronDown size={12} />
                                </>
                              }
                              disabled={!pane.tabs.length}
                              items={pane.tabs.map((tab) => ({
                                id: tab.object_id,
                                label:
                                  draftTitles[tab.object_id] ??
                                  titles[tab.object_id] ??
                                  (tab.kind === 'source'
                                    ? t('资料', 'Document')
                                    : t('笔记', 'Note')),
                                icon: tab.kind === 'source' ? <BookOpen /> : <FileText />,
                                checked: tab.object_id === pane.active_object_id,
                                radio: true,
                                onSelect: () => openWorkspaceTab(index, tab),
                              }))}
                            />
                            <span className="spacer" />
                            {session.layout.panes.length === 2 && (
                              <div
                                className="pane-switch view-switch"
                                role="group"
                                aria-label={t('内容面板', 'Content panes')}
                              >
                                {[0, 1].map((paneIndex) => (
                                  <button
                                    key={paneIndex}
                                    aria-label={t(
                                      `切换到面板 ${paneIndex + 1}`,
                                      `Switch to pane ${paneIndex + 1}`,
                                    )}
                                    aria-pressed={session.layout.active_pane === paneIndex}
                                    onClick={() =>
                                      void updateLayout((layout) => ({
                                        ...layout,
                                        active_pane: paneIndex,
                                      })).catch(failed)
                                    }
                                  >
                                    {paneIndex + 1}
                                  </button>
                                ))}
                              </div>
                            )}
                            <button
                              className="icon-button"
                              aria-label={t('显示笔记列表', 'Show note list')}
                              aria-pressed={listVisible || listOverlay}
                              onClick={() => revealNoteList(true)}
                            >
                              <List size={17} />
                            </button>
                            <PopupMenu
                              label={t(`面板 ${index + 1} 操作`, `Pane ${index + 1} actions`)}
                              trigger={<Ellipsis size={19} />}
                              placement="bottom-start"
                              items={[
                                {
                                  id: 'gallery',
                                  label: t('卡片视图', 'Card view'),
                                  icon: <LayoutGrid />,
                                  onSelect: () => {
                                    setView('notes', () => {
                                      setFocusMode(false);
                                      setGallery(true);
                                    });
                                  },
                                },
                                {
                                  id: 'focus',
                                  label: focusMode
                                    ? t('退出专注模式', 'Leave focus mode')
                                    : t('专注模式', 'Focus mode'),
                                  icon: focusMode ? <Minimize2 /> : <Maximize2 />,
                                  onSelect: () => setFocusMode(!focusMode),
                                },
                                {
                                  id: 'close',
                                  label: t('关闭当前文档', 'Close current document'),
                                  icon: <X />,
                                  disabled: !pane.active_object_id,
                                  separatorBefore: true,
                                  onSelect: () => {
                                    if (pane.active_object_id)
                                      void closeTab(index, pane.active_object_id);
                                  },
                                },
                                {
                                  id: 'trash',
                                  label: t('移入回收站', 'Move to trash'),
                                  icon: <Trash2 />,
                                  disabled:
                                    !pane.active_object_id ||
                                    pane.tabs.find((tab) => tab.object_id === pane.active_object_id)
                                      ?.kind !== 'note',
                                  onSelect: () => {
                                    if (pane.active_object_id)
                                      void changeNoteTrash(pane.active_object_id);
                                  },
                                },
                              ]}
                            />
                            <button
                              className="icon-button"
                              aria-label={
                                session.layout.panes.length === 2
                                  ? t('关闭第二面板', 'Close second pane')
                                  : t('打开第二面板', 'Open second pane')
                              }
                              aria-pressed={session.layout.panes.length === 2}
                              title={t('双面板', 'Split view')}
                              onClick={() => void toggleSplit(index).catch(failed)}
                            >
                              <Columns2 size={17} />
                            </button>
                          </div>
                          {pane.active_object_id &&
                          identity &&
                          pane.tabs.find((tab) => tab.object_id === pane.active_object_id)?.kind ===
                            'source' ? (
                            <Suspense
                              fallback={
                                <p className="muted">{t('正在准备阅读器…', 'Preparing reader…')}</p>
                              }
                            >
                              <ReferenceProvider
                                identity={identity}
                                generation={String(organizationVersion)}
                                onOpenReference={(target, occurrenceId) =>
                                  openReference(index, target, occurrenceId)
                                }
                              >
                                <SourceReader
                                  identity={identity}
                                  active={session.layout.active_pane === index}
                                  sourceId={pane.active_object_id}
                                  versionId={
                                    sourceLocations[index]?.sourceId === pane.active_object_id
                                      ? sourceLocations[index].versionId
                                      : undefined
                                  }
                                  initialTarget={
                                    sourceLocations[index]?.sourceId === pane.active_object_id
                                      ? sourceLocations[index].target
                                      : undefined
                                  }
                                  initialPdfView={
                                    sourceLocations[index]?.sourceId === pane.active_object_id
                                      ? sourceLocations[index].initialPdfView
                                      : undefined
                                  }
                                  onAddToNote={(annotations) =>
                                    setAddingAnnotation({ annotations, pane: index })
                                  }
                                  onOpenSource={(nextSourceId, nextVersionId, annotationId) => {
                                    const target: ResolvedSourceTarget = {
                                      source_id: nextSourceId,
                                      source_version_id: nextVersionId,
                                      annotation_id: annotationId,
                                      locator: null,
                                      locator_status: null,
                                      parse_id: null,
                                      block_offset: null,
                                    };
                                    void openSource(
                                      nextSourceId,
                                      nextVersionId,
                                      index,
                                      target,
                                    ).catch(failed);
                                  }}
                                  onTitleChange={rememberTitle}
                                  onChanged={saved}
                                  hideSidebar={session.layout.panes.length === 2}
                                  onClose={() => setView('sources')}
                                />
                              </ReferenceProvider>
                            </Suspense>
                          ) : pane.active_object_id &&
                            identity &&
                            pane.tabs.find((tab) => tab.object_id === pane.active_object_id)
                              ?.kind !== 'note' ? (
                            <div className="empty-state" role="status">
                              <p>
                                {t(
                                  '当前版本尚不支持打开此类页签。',
                                  'This tab type is not available in this version.',
                                )}
                              </p>
                            </div>
                          ) : pane.active_object_id && identity ? (
                            <NotePane
                              key={`${workspaceKey}/${pane.active_object_id}/${editorEpoch}`}
                              noteId={pane.active_object_id}
                              positions={layoutPositions.current!}
                              identity={identity}
                              pane={String(index)}
                              controllerFor={controllerFor}
                              paused={!!transition}
                              onSaved={saved}
                              onExport={exportNote}
                              onDuplicate={duplicateNote}
                              onOpenReference={(target, occurrenceId) =>
                                openReference(index, target, occurrenceId)
                              }
                              onOpenNote={(id) => void openNote(id).catch(failed)}
                              onReturnReference={() => visit(-1)}
                              returnAvailable={navigationAvailability.back}
                              referenceLocation={
                                referenceLocations[index]?.target.target_id ===
                                pane.active_object_id
                                  ? referenceLocations[index]
                                  : undefined
                              }
                              organizationVersion={organizationVersion}
                              onTitleChange={rememberTitle}
                              customTypes={customObjectTypes}
                            />
                          ) : (
                            <div className="empty-state">
                              <span className="empty-icon">
                                <BookOpen size={32} strokeWidth={1.5} />
                              </span>
                              <h2>
                                {t('留一点空间，给新的思考。', 'A little space for a new thought.')}
                              </h2>
                              <p>
                                {t(
                                  '打开一篇笔记，或记录此刻的想法。',
                                  'Open a note, or capture what is on your mind.',
                                )}
                              </p>
                              <button className="primary-button" onClick={() => void createNote()}>
                                <Plus size={17} />
                                {t('新建笔记', 'New note')}
                              </button>
                              <small>Ctrl + N</small>
                            </div>
                          )}
                        </section>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </main>
          </div>
          <CommandPalette
            open={commandPaletteOpen}
            items={commandItems}
            onClose={() => setCommandPaletteOpen(false)}
          />{' '}
          {addingAnnotation && identity && (
            <Suspense fallback={null}>
              <AnnotationToNoteDialog
                key={`${workspaceKey}/${addingAnnotation.annotations.map((item) => item.id).join('/')}`}
                identity={identity}
                annotations={addingAnnotation.annotations}
                controllerFor={controllerFor}
                onSaved={saved}
                onOpenNote={(id) =>
                  void openNote(
                    id,
                    sessionRef.current?.layout.panes.length === 2
                      ? 1 - addingAnnotation.pane
                      : addingAnnotation.pane,
                  ).catch(failed)
                }
                onClose={() => setAddingAnnotation(null)}
              />
            </Suspense>
          )}
          {localLibraryDialog && (
            <LocalDocumentLibraryDialog
              name={
                localDocuments.find((document) => document.token === localLibraryDialog)
                  ?.display_name ?? ''
              }
              workspaces={app?.workspaces ?? []}
              onChoose={(create, recent) => void chooseLocalLibrary(create, recent)}
              onCancel={() => setLocalLibraryDialog(null)}
            />
          )}
          {draggingDocuments && (
            <div className="document-drop-overlay" role="status">
              <FolderOpen size={32} />
              <strong>{t('松开以打开资料', 'Drop to open documents')}</strong>
              <span>PDF · Markdown · TXT · DOCX · DOC · EPUB · MOBI · CSV · XLSX · XLS · PPTX</span>
            </div>
          )}
          {transition && (
            <div className="transition-backdrop" role="status" aria-live="polite">
              <div className="transition-card">
                <LoaderCircle className="spin" />
                <p>{transition}</p>
                {transitionError && <p role="alert">{transitionError}</p>}
                {transitionError && (
                  <button className="primary-button" onClick={() => retryTransition.current?.()}>
                    {t('重新确认结果', 'Check result again')}
                  </button>
                )}
                {transitionOperation && (
                  <button
                    className="secondary-button"
                    disabled={transitionCancelState !== 'idle'}
                    aria-busy={transitionCancelState === 'requesting'}
                    onClick={() => void cancelTransitionOperation()}
                  >
                    {transitionCancelState === 'requesting'
                      ? t('正在请求取消…', 'Requesting cancellation…')
                      : transitionCancelState === 'requested'
                        ? t('取消请求已发送', 'Cancellation requested')
                        : t('取消操作', 'Cancel operation')}
                  </button>
                )}
              </div>
            </div>
          )}
          {settingsMounted && app && (
            <SettingsPanel
              visible={settingsOpen}
              key={workspaceKey ?? 'no-workspace'}
              initialSection={pageNavigation['settings.section'] as SettingsSection | undefined}
              onSectionChange={(section) => updatePageNavigation('settings.section', () => section)}
              app={app}
              identity={identity}
              busy={preferenceBusy || busy}
              error={preferenceError ?? error}
              onDismiss={dismissSettings}
              update={updatePreferences}
              open={() => {
                dismissSettings();
                void bootstrap(false);
              }}
              create={() => {
                dismissSettings();
                void bootstrap(true);
              }}
              close={() => {
                dismissSettings();
                void closeWorkspace();
              }}
              backup={() => {
                dismissSettings();
                setView('backup');
              }}
              onTasks={(destination) => {
                dismissSettings();
                openTaskCenter(destination ?? { section: 'exports' });
              }}
              components={app.parse_components ?? []}
            />
          )}
          <ToastHost />
        </KnowledgeTaskNavigation.Provider>
      </PageNavigationContext.Provider>
    </LanguageContext.Provider>
  );
}

function NotePane({
  noteId,
  positions,
  identity,
  pane,
  controllerFor,
  onSaved,
  paused,
  organizationVersion,
  onTitleChange,
  onExport,
  onDuplicate,
  onOpenReference,
  onOpenNote,
  referenceLocation,
  onReturnReference,
  returnAvailable,
  customTypes,
}: {
  onOpenReference: OpenReference;
  onOpenNote: (id: string) => void;
  referenceLocation?: ReferenceLocation;
  onReturnReference?: () => Promise<void> | void;
  returnAvailable?: boolean;
  customTypes: readonly ObjectTypeDefinition[];
  noteId: string;
  positions: SessionPositions;
  onExport: (
    controller: DraftController,
    format: ExportFormat,
    citationStyleId: string | null,
  ) => Promise<void>;
  onDuplicate: (controller: DraftController) => Promise<void>;
  identity: WorkspaceIdentity;
  pane: string;
  controllerFor: (noteId: string) => Promise<DraftController>;
  onSaved: () => void;
  paused: boolean;
  organizationVersion: number;
  onTitleChange: (noteId: string, title: string) => void;
}) {
  const { t } = useI18n();
  const [controller, setController] = useState<DraftController | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (paused) return;
    let live = true;
    setError(null);
    void controllerFor(noteId)
      .then((controller) => {
        if (live) {
          setController(controller);
          if (controller.getSnapshot().status === 'local')
            void controller.flush().catch(() => undefined);
        }
      })
      .catch((error) => {
        if (live) setError(message(error));
      });
    return () => {
      live = false;
    };
  }, [noteId, controllerFor, paused, attempt]);
  if (error)
    return (
      <p className="inline-error" role="alert">
        {error}
        <button className="text-button" onClick={() => setAttempt((value) => value + 1)}>
          {t('重新读取', 'Reload')}
        </button>
      </p>
    );
  if (!controller)
    return (
      <div className="empty-state">
        <LoaderCircle className="spin" />
        <p>{t('正在读取正文与本地草稿…', 'Loading note and local draft…')}</p>
      </div>
    );
  return (
    <EditorScrollSession
      positions={positions}
      pane={Number(pane)}
      noteId={noteId}
      referenceRequest={referenceLocation?.requestId}
    >
      <Suspense fallback={<p className="muted">{t('正在准备编辑器…', 'Preparing editor…')}</p>}>
        <EditorPane
          controller={controller}
          identity={identity}
          pane={pane}
          onSaved={onSaved}
          onExport={onExport}
          onDuplicate={onDuplicate}
          onOpenReference={onOpenReference}
          onOpenNote={onOpenNote}
          referenceLocation={referenceLocation}
          onReturnReference={onReturnReference}
          returnAvailable={returnAvailable}
          organizationVersion={organizationVersion}
          onTitleChange={onTitleChange}
          customTypes={customTypes}
        />
      </Suspense>
    </EditorScrollSession>
  );
}

function TabTitle({
  identity,
  noteId,
  title,
  onTitle,
  fallback,
  kind = 'note',
}: {
  identity: WorkspaceIdentity;
  noteId: string;
  title?: string;
  onTitle: (id: string, title: string) => void;
  fallback: string;
  kind?: TabKind;
}) {
  useEffect(() => {
    if (title !== undefined || (kind !== 'note' && kind !== 'source')) return;
    let live = true;
    const request =
      kind === 'source'
        ? call({ kind: 'sources_get', payload: { source_id: noteId } }, 'source', identity).then(
            (source) => ({ id: source.id, title: source.display_title }),
          )
        : call({ kind: 'notes_get', payload: { note_id: noteId } }, 'note', identity);
    void request
      .then((note) => {
        if (live) onTitle(note.id, note.title);
      })
      .catch(() => {
        /* The pane presents the actual read error when opened. */
      });
    return () => {
      live = false;
    };
  }, [identity, noteId, title, onTitle, kind]);
  return title || fallback;
}
