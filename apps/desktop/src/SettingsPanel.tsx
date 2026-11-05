import { Select } from './ui/Select';
import { AsrSettingsEditor } from './voice/AsrSettingsEditor';
import { lazy, Suspense, useEffect, useId, useMemo, useRef, useState } from 'react';
import {
  Check,
  Activity,
  Archive,
  FolderOpen,
  FolderPlus,
  HardDrive,
  Info,
  Keyboard,
  Languages,
  LayoutTemplate,
  Eraser,
  LogOut,
  Moon,
  Monitor,
  Mic,
  SlidersHorizontal,
  Shapes,
  Sun,
  Type,
  X,
} from 'lucide-react';
import type {
  AppPreferences,
  ParseComponentIssue,
  CleaningParams,
  DesktopAppState,
  DiagnosticVersions,
  RetrievalParams,
  WorkspaceIdentity,
} from '@pkms/contracts';
import {
  DEFAULT_RETRIEVAL_PARAMS,
  RetrievalParamsForm,
  saveRetrievalParams,
  useRetrievalSettings,
} from './inference/retrieval-settings';
import {
  CleaningParamsForm,
  DEFAULT_CLEANING_PARAMS,
  saveCleaningParams,
  useCleaningSettings,
} from './sources/cleaning-settings';
import { useI18n } from './i18n';
import { SHORTCUT_OPTIONS, shortcutParts } from './layout/shortcuts';
import { cleaningParamsEqual, retrievalParamsEqual } from './settings-param-state';
import { BrandMark } from './ui/BrandMark';
import { ActionConfirmDialog } from './ui/ActionConfirmDialog';
import type { OpenTasks } from './portability/task-navigation';
import { ObjectTypesSettings } from './organization/ObjectTypesSettings';
import { TemplateSettings } from './organization/TemplateSettings';
import { readObjectTypeRegistry } from './organization/object-types';
import { call, message as errorMessage } from './api';
import { displayPath } from './format';
import './settings.css';

const ModelsPage = lazy(() =>
  import('./inference/ModelsPage').then((module) => ({ default: module.ModelsPage })),
);
const DiagnosticsPage = lazy(() =>
  import('./diagnostics/DiagnosticsPage').then((module) => ({ default: module.DiagnosticsPage })),
);

export type SettingsSection =
  | 'appearance'
  | 'objects'
  | 'templates'
  | 'retrieval'
  | 'models'
  | 'diagnostics'
  | 'asr'
  | 'cleaning'
  | 'workspace'
  | 'shortcuts'
  | 'about';

function settingsConflict(failure: unknown): boolean {
  const code = (failure as { details?: { code?: string } } | null)?.details?.code;
  return code === 'revision_conflict' || code === 'metadata_conflict';
}

export function SettingsPanel({
  app,
  identity,
  busy,
  update,
  open,
  create,
  close,
  backup,
  onTasks,
  onDismiss,
  error,
  embedded = false,
  visible = true,
  components = [],
  initialSection = 'appearance',
  onSectionChange,
}: {
  app: DesktopAppState;
  identity: WorkspaceIdentity | null;
  busy: boolean;
  update: (change: Partial<AppPreferences>) => Promise<boolean>;
  open: () => void;
  create: () => void;
  close: () => void;
  backup: () => void;
  onTasks: OpenTasks;
  onDismiss: () => void;
  error?: string | null;
  embedded?: boolean;
  visible?: boolean;
  components?: ParseComponentIssue[];
  initialSection?: SettingsSection;
  onSectionChange?: (section: SettingsSection) => void;
}) {
  const { t, language } = useI18n();
  const prefs = app.preferences.preferences;
  const objectTypeRegistryJson =
    app.workspace_metadata?.object_type_registry ?? prefs.object_type_registry;
  const templateRegistryJson =
    app.workspace_metadata?.note_template_registry ?? prefs.note_template_registry;
  const customTypes = useMemo(
    () => readObjectTypeRegistry(objectTypeRegistryJson, identity?.workspace_id).types,
    [identity?.workspace_id, objectTypeRegistryJson],
  );
  const workspaceKey = identity ? `${identity.local_instance_id}/${identity.workspace_id}` : '';
  const workspaceScope = useRef({ key: workspaceKey, generation: 0 });
  if (workspaceScope.current.key !== workspaceKey) {
    workspaceScope.current = {
      key: workspaceKey,
      generation: workspaceScope.current.generation + 1,
    };
  }
  const retrieval = useRetrievalSettings(identity);
  const [params, setParams] = useState<RetrievalParams>(DEFAULT_RETRIEVAL_PARAMS);
  const [embedding, setEmbedding] = useState<{ model: string; dimensions: number } | null>(null);
  const [retrievalBusy, setRetrievalBusy] = useState(false);
  const [retrievalMessage, setRetrievalMessage] = useState<string | null>(null);
  const [retrievalConflict, setRetrievalConflict] = useState(false);
  const cleaning = useCleaningSettings(identity);
  const [cleaningParams, setCleaningParams] = useState<CleaningParams>(DEFAULT_CLEANING_PARAMS);
  const [cleaningBusy, setCleaningBusy] = useState(false);
  const [cleaningMessage, setCleaningMessage] = useState<string | null>(null);
  const [cleaningConflict, setCleaningConflict] = useState(false);
  const [reloadSection, setReloadSection] = useState<'retrieval' | 'cleaning' | null>(null);
  const [aboutVersions, setAboutVersions] = useState<DiagnosticVersions | null>(null);
  const [aboutError, setAboutError] = useState<string | null>(null);
  const [aboutBusy, setAboutBusy] = useState(false);
  const [aboutRefresh, setAboutRefresh] = useState(0);
  const settingsContent = useRef<HTMLElement>(null);
  useEffect(() => {
    setParams(DEFAULT_RETRIEVAL_PARAMS);
    setCleaningParams(DEFAULT_CLEANING_PARAMS);
    setRetrievalMessage(null);
    setCleaningMessage(null);
    setRetrievalConflict(false);
    setCleaningConflict(false);
    setRetrievalBusy(false);
    setCleaningBusy(false);
    setReloadSection(null);
  }, [workspaceKey]);
  useEffect(() => {
    if (!visible) setReloadSection(null);
  }, [visible]);
  useEffect(() => {
    const loaded = retrieval.settings;
    if (loaded) setParams(loaded.params);
  }, [retrieval.settings]);
  useEffect(() => {
    const loaded = cleaning.settings;
    if (loaded) setCleaningParams(loaded.params);
  }, [cleaning.settings]);
  useEffect(() => {
    let live = true;
    if (!identity) {
      setEmbedding(null);
      return () => {
        live = false;
      };
    }
    void call({ kind: 'embedding_profile_get' }, 'embedding_profile', identity)
      .then((profile) => {
        if (live)
          setEmbedding(profile ? { model: profile.model, dimensions: profile.dimensions } : null);
      })
      .catch(() => {
        if (live) setEmbedding(null);
      });
    return () => {
      live = false;
    };
  }, [identity]);
  async function persistRetrieval() {
    const loaded = retrieval.settings;
    if (!identity || !loaded || retrieval.busy || retrievalConflict) return;
    const generation = workspaceScope.current.generation;
    setRetrievalBusy(true);
    setRetrievalMessage(null);
    try {
      const saved = await saveRetrievalParams(identity, loaded.row_version, params);
      if (workspaceScope.current.generation !== generation) return;
      retrieval.setSettings(saved);
      setRetrievalConflict(false);
      setRetrievalMessage(t('已保存检索参数。', 'Retrieval parameters saved.'));
    } catch (failure) {
      if (workspaceScope.current.generation !== generation) return;
      setRetrievalConflict(settingsConflict(failure));
      setRetrievalMessage(
        settingsConflict(failure)
          ? t('设置有更新，修改已保留。请重新读取。', 'Settings changed; edits kept. Reload saved.')
          : failure instanceof Error
            ? failure.message
            : t('保存失败，请重试。', 'Could not save; try again.'),
      );
    } finally {
      if (workspaceScope.current.generation === generation) setRetrievalBusy(false);
    }
  }
  async function persistCleaning() {
    const loaded = cleaning.settings;
    if (!identity || !loaded || cleaning.busy || cleaningConflict) return;
    const generation = workspaceScope.current.generation;
    setCleaningBusy(true);
    setCleaningMessage(null);
    try {
      const saved = await saveCleaningParams(identity, loaded.row_version, cleaningParams);
      if (workspaceScope.current.generation !== generation) return;
      cleaning.setSettings(saved);
      setCleaningConflict(false);
      setCleaningMessage(t('已保存清洗设置。', 'Cleaning settings saved.'));
    } catch (failure) {
      if (workspaceScope.current.generation !== generation) return;
      setCleaningConflict(settingsConflict(failure));
      setCleaningMessage(
        settingsConflict(failure)
          ? t('设置有更新，修改已保留。请重新读取。', 'Settings changed; edits kept. Reload saved.')
          : failure instanceof Error
            ? failure.message
            : t('保存失败，请重试。', 'Could not save; try again.'),
      );
    } finally {
      if (workspaceScope.current.generation === generation) setCleaningBusy(false);
    }
  }
  async function reloadSavedSettings(target: 'retrieval' | 'cleaning') {
    const generation = workspaceScope.current.generation;
    const setMessage = target === 'retrieval' ? setRetrievalMessage : setCleaningMessage;
    setReloadSection(null);
    setMessage(t('正在读取已保存设置…', 'Loading saved settings…'));
    const loaded = await (target === 'retrieval' ? retrieval.reload() : cleaning.reload());
    if (workspaceScope.current.generation !== generation) return;
    if (!loaded) {
      setMessage(
        t(
          '读取失败。你的修改仍保留，请重试。',
          'Could not reload. Your edits are kept; try again.',
        ),
      );
      restoreReloadFocus(target, generation, false);
      return;
    }
    if (target === 'retrieval') setRetrievalConflict(false);
    else setCleaningConflict(false);
    setMessage(t('已重新读取最新设置。', 'Latest saved settings loaded.'));
    restoreReloadFocus(target, generation, true);
  }
  function restoreReloadFocus(
    target: 'retrieval' | 'cleaning',
    generation: number,
    succeeded: boolean,
  ) {
    requestAnimationFrame(() => {
      const content = settingsContent.current;
      if (
        workspaceScope.current.generation !== generation ||
        !content?.checkVisibility() ||
        content.dataset.settingsSection !== target
      )
        return;
      const control = succeeded
        ? content.querySelector<HTMLInputElement>('.retrieval-params input:not(:disabled)')
        : content.parentElement?.querySelector<HTMLButtonElement>(
            '.settings-footer-actions button:not(:disabled)',
          );
      control?.focus();
    });
  }
  function requestSettingsReload(target: 'retrieval' | 'cleaning') {
    const dirty = target === 'retrieval' ? retrievalDirty : cleaningDirty;
    if (dirty) setReloadSection(target);
    else void reloadSavedSettings(target);
  }
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const sectionId = useId();
  const [section, setSection] = useState<SettingsSection>(initialSection);
  const [visited, setVisited] = useState<SettingsSection[]>([initialSection]);
  function showSection(next: SettingsSection) {
    setSection(next);
    setVisited((current) => (current.includes(next) ? current : [...current, next]));
    onSectionChange?.(next);
  }
  useEffect(() => {
    setSection(initialSection);
    setVisited((current) =>
      current.includes(initialSection) ? current : [...current, initialSection],
    );
  }, [initialSection]);
  const sections = [
    { id: 'appearance', label: t('外观', 'Appearance'), icon: Sun },
    { id: 'objects', label: t('对象类型', 'Object types'), icon: Shapes },
    { id: 'templates', label: t('模板', 'Templates'), icon: LayoutTemplate },
    { id: 'models', label: t('模型与连接', 'Models & connections'), icon: SlidersHorizontal },
    { id: 'retrieval', label: t('检索', 'Retrieval'), icon: SlidersHorizontal },
    { id: 'asr', label: t('转写服务', 'Transcription service'), icon: Mic },
    { id: 'cleaning', label: t('清洗', 'Cleaning'), icon: Eraser },
    { id: 'workspace', label: t('知识库', 'Workspace'), icon: HardDrive },
    { id: 'shortcuts', label: t('快捷键', 'Keyboard shortcuts'), icon: Keyboard },
    { id: 'diagnostics', label: t('统计与诊断', 'Statistics & diagnostics'), icon: Activity },
    { id: 'about', label: t('关于', 'About'), icon: Info },
  ] as const;
  useEffect(() => {
    let live = true;
    if (section !== 'about' || !identity) {
      setAboutVersions(null);
      setAboutError(null);
      setAboutBusy(false);
      return () => {
        live = false;
      };
    }
    setAboutBusy(true);
    setAboutError(null);
    void call(
      { kind: 'diagnostics_preview', payload: { categories: ['versions'] } },
      'diagnostics_preview',
      identity,
    )
      .then((result) => {
        if (live) setAboutVersions(result.snapshot.versions);
      })
      .catch((failure) => {
        if (live) setAboutError(errorMessage(failure));
      })
      .finally(() => {
        if (live) setAboutBusy(false);
      });
    return () => {
      live = false;
    };
  }, [section, identity, aboutRefresh]);
  const sectionTitle = sections.find((item) => item.id === section)!.label;
  // Retrieval and cleaning are stored per workspace and each keeps its own row
  // version, so every section owns its save action; the panel renders it in one
  // fixed footer instead of a button that scrolls away with the form.
  const retrievalDirty = Boolean(
    retrieval.settings && !retrievalParamsEqual(params, retrieval.settings.params),
  );
  const cleaningDirty = Boolean(
    cleaning.settings && !cleaningParamsEqual(cleaningParams, cleaning.settings.params),
  );
  const retrievalAtDefaults = retrievalParamsEqual(params, DEFAULT_RETRIEVAL_PARAMS);
  const cleaningAtDefaults = cleaningParamsEqual(cleaningParams, DEFAULT_CLEANING_PARAMS);
  const saveAction =
    section === 'retrieval'
      ? {
          run: persistRetrieval,
          busy: retrievalBusy || retrieval.busy,
          saving: retrievalBusy,
          message: retrievalMessage,
          ready: Boolean(identity && retrieval.settings),
          dirty: retrievalDirty,
          conflict: retrievalConflict,
          reloadAvailable: retrievalConflict || !!retrieval.error,
          reload: () => requestSettingsReload('retrieval'),
          reset: () => {
            setParams(DEFAULT_RETRIEVAL_PARAMS);
            setRetrievalMessage(
              retrieval.settings &&
                retrievalParamsEqual(retrieval.settings.params, DEFAULT_RETRIEVAL_PARAMS)
                ? t('已恢复默认值。', 'Defaults restored.')
                : t('已恢复默认值，尚未保存。', 'Defaults restored. Save to apply them.'),
            );
          },
          resetAvailable: !retrievalAtDefaults,
        }
      : section === 'cleaning'
        ? {
            run: persistCleaning,
            busy: cleaningBusy || cleaning.busy,
            saving: cleaningBusy,
            message: cleaningMessage,
            ready: Boolean(identity && cleaning.settings),
            dirty: cleaningDirty,
            conflict: cleaningConflict,
            reloadAvailable: cleaningConflict || !!cleaning.error,
            reload: () => requestSettingsReload('cleaning'),
            reset: () => {
              setCleaningParams(DEFAULT_CLEANING_PARAMS);
              setCleaningMessage(
                cleaning.settings &&
                  cleaningParamsEqual(cleaning.settings.params, DEFAULT_CLEANING_PARAMS)
                  ? t('已恢复默认值。', 'Defaults restored.')
                  : t('已恢复默认值，尚未保存。', 'Defaults restored. Save to apply them.'),
              );
            },
            resetAvailable: !cleaningAtDefaults,
          }
        : null;

  useEffect(() => {
    if (embedded || !visible) return;
    const element = dialog.current;
    if (!element) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    return () => {
      if (element.open) element.close();
      if (
        previous?.isConnected &&
        (document.activeElement === document.body || element.contains(document.activeElement))
      ) {
        previous.focus({ preventScroll: true });
      }
    };
  }, [embedded, visible]);

  const content = (
    <div className="settings-shell">
      <aside className="settings-sidebar">
        <div className="settings-brand">
          <h1 id={titleId}>{embedded ? t('系统配置', 'Settings') : t('设置', 'Settings')}</h1>
          <p>跨索 spanseek · {t('个人知识库', 'Personal knowledge')}</p>
        </div>
        <nav className="settings-nav" aria-label={t('设置分类', 'Settings categories')}>
          {sections.map(({ id, label, icon: Icon }) => (
            <button
              type="button"
              key={id}
              aria-current={section === id ? 'page' : undefined}
              aria-controls={sectionId}
              onClick={(event) => {
                showSection(id);
                event.currentTarget.focus({ preventScroll: true });
              }}
            >
              <Icon size={17} aria-hidden="true" />
              <span>{label}</span>
            </button>
          ))}
          {embedded && (
            <button type="button" onClick={backup}>
              <Archive size={17} aria-hidden="true" />
              <span>{t('同步、备份与恢复', 'Sync & backup')}</span>
            </button>
          )}
        </nav>
        <footer className="settings-version">
          <span>跨索 spanseek {aboutVersions?.application ?? __PKMS_APP_VERSION__}</span>
          <span>{t('稳定版', 'Stable')}</span>
        </footer>
      </aside>
      <div className="settings-main">
        <header className="settings-header">
          <h2 id={`${sectionId}-title`}>{sectionTitle}</h2>
          <button
            type="button"
            className="settings-dismiss"
            aria-label={t('关闭设置', 'Close settings')}
            disabled={busy}
            onClick={() => (embedded ? onDismiss() : dialog.current?.close())}
            autoFocus
          >
            <X size={18} aria-hidden="true" />
          </button>
        </header>
        {error && (
          <p className="settings-error" role="alert">
            {error}
          </p>
        )}
        {busy && (
          <p className="settings-busy" role="status">
            {t('正在处理，请稍候…', 'Working, please wait…')}
          </p>
        )}
        <section
          ref={settingsContent}
          data-settings-section={section}
          id={sectionId}
          className={`settings-content ${['models', 'diagnostics'].includes(section) ? 'settings-content-subpage' : ''}`}
          aria-labelledby={`${sectionId}-title`}
          aria-busy={busy}
        >
          {section === 'appearance' && (
            <>
              <fieldset className="settings-themes" disabled={busy}>
                <legend>{t('界面主题', 'Color theme')}</legend>
                <p className="settings-description">
                  {t('为不同的光线，选择舒服的底色。', 'A comfortable background for any light.')}
                </p>
                <div className="settings-theme-grid">
                  {(['light', 'dark', 'system'] as const).map((theme) => {
                    const Icon = theme === 'light' ? Sun : theme === 'dark' ? Moon : Monitor;
                    const label =
                      theme === 'light'
                        ? t('浅色', 'Light')
                        : theme === 'dark'
                          ? t('深色', 'Dark')
                          : t('跟随系统', 'System');
                    return (
                      <button
                        key={theme}
                        type="button"
                        className="settings-theme-choice"
                        aria-pressed={prefs.theme === theme}
                        onClick={() => update({ theme })}
                      >
                        <span className={`settings-theme-preview ${theme}`} aria-hidden="true">
                          <span className="settings-preview-sidebar">
                            <i />
                            <i />
                            <i />
                          </span>
                          <span className="settings-preview-page">
                            <b />
                            <i />
                            <i />
                            <i />
                          </span>
                        </span>
                        <span className="settings-theme-label">
                          <Icon size={14} aria-hidden="true" />
                          <span>{label}</span>
                          {prefs.theme === theme && <Check size={14} aria-hidden="true" />}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </fieldset>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <Languages size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('语言', 'Language')}</strong>
                    <small>
                      {t(
                        '界面语言；笔记原文保持不变。',
                        'Interface language. Your writing stays as it is.',
                      )}
                    </small>
                  </span>
                </span>
                <Select
                  aria-label={t('语言', 'Language')}
                  value={language}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ language: nextValue as 'zh-CN' | 'en' })}
                >
                  <option value="zh-CN">简体中文</option>
                  <option value="en">English</option>
                </Select>
              </div>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <Type size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('正文字号', 'Editor font size')}</strong>
                    <small>
                      {t('找到适合长时间阅读的大小。', 'Keep longer reading sessions comfortable.')}
                    </small>
                  </span>
                </span>
                <Select
                  aria-label={t('正文字号', 'Editor font size')}
                  value={prefs.editor_font_size}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ editor_font_size: Number(nextValue) })}
                >
                  {[15, 17, 19, 21, 24].map((size) => (
                    <option key={size} value={size}>
                      {size} px
                    </option>
                  ))}
                </Select>
              </div>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <Type size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('界面字号', 'Interface font size')}</strong>
                    <small>
                      {t('菜单、列表与面板的字号。', 'Size of menus, lists and panels.')}
                    </small>
                  </span>
                </span>
                <Select
                  aria-label={t('界面字号', 'Interface font size')}
                  value={prefs.ui_font_size}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ ui_font_size: Number(nextValue) })}
                >
                  {[12, 13, 14, 16, 18, 20, 22, 24].map((size) => (
                    <option key={size} value={size}>
                      {size} px
                    </option>
                  ))}
                </Select>
              </div>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <SlidersHorizontal size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('界面缩放', 'Interface zoom')}</strong>
                    <small>
                      {t(
                        '与 Ctrl + 滚轮、Ctrl + +/− 同步，重启后保持。',
                        'Kept in step with Ctrl+wheel and Ctrl+±, and restored after a restart.',
                      )}
                    </small>
                  </span>
                </span>
                <Select
                  aria-label={t('界面缩放', 'Interface zoom')}
                  value={prefs.zoom_percent}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ zoom_percent: Number(nextValue) })}
                >
                  {[50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300].map((percent) => (
                    <option key={percent} value={percent}>
                      {percent}%
                    </option>
                  ))}
                </Select>
              </div>
            </>
          )}
          {section === 'objects' && (
            <ObjectTypesSettings
              registryJson={objectTypeRegistryJson}
              identity={identity}
              update={update}
              busy={busy}
            />
          )}{' '}
          {section === 'templates' && (
            <TemplateSettings
              registryJson={templateRegistryJson}
              identity={identity}
              customTypes={customTypes}
              update={update}
              busy={busy}
            />
          )}{' '}
          {section === 'retrieval' && (
            <>
              <h3>{t('检索与引用参数', 'Retrieval and citation parameters')}</h3>
              <p className="settings-description">
                {t(
                  '这些参数按知识库保存，并在每次检索或提问时以快照生效。',
                  'Stored per workspace and snapshotted for each search or answer.',
                )}
              </p>
              {identity ? (
                <>
                  <RetrievalParamsForm
                    value={params}
                    onChange={(next) => {
                      setParams(next);
                      if (!retrievalConflict) setRetrievalMessage(null);
                    }}
                    disabled={retrievalBusy || retrieval.busy || !retrieval.settings}
                    model={embedding?.model ?? null}
                    dimensions={embedding?.dimensions ?? null}
                  />
                  {retrieval.error && (
                    <p className="settings-error" role="alert">
                      {retrieval.error}
                    </p>
                  )}
                </>
              ) : (
                <p className="settings-description">
                  {t(
                    '打开知识库后即可配置检索参数。',
                    'Open a workspace to configure retrieval parameters.',
                  )}
                </p>
              )}
            </>
          )}{' '}
          {visited.includes('asr') && (
            <div hidden={section !== 'asr'}>
              {identity ? (
                <AsrSettingsEditor
                  key={`${identity.local_instance_id}/${identity.workspace_id}`}
                  identity={identity}
                />
              ) : (
                <p className="muted">
                  {t(
                    '打开知识库后可配置转写服务。',
                    'Open a workspace to configure transcription.',
                  )}
                </p>
              )}
            </div>
          )}
          {(['models', 'diagnostics'] as const).map(
            (id) =>
              visited.includes(id) && (
                <div key={id} className="settings-subpage" hidden={section !== id}>
                  {!identity ? (
                    <p className="settings-description">
                      {t(
                        '打开知识库后可查看和配置。',
                        'Open a workspace to view and configure this section.',
                      )}
                    </p>
                  ) : (
                    <Suspense
                      fallback={
                        <p className="settings-description">{t('正在读取…', 'Loading…')}</p>
                      }
                    >
                      {id === 'models' ? (
                        <ModelsPage identity={identity} embedded />
                      ) : (
                        <DiagnosticsPage
                          identity={identity}
                          components={components}
                          onTasks={onTasks}
                          embedded
                        />
                      )}
                    </Suspense>
                  )}
                </div>
              ),
          )}
          {section === 'cleaning' && (
            <>
              <h3>{t('正文清洗', 'Text cleaning')}</h3>
              <p className="settings-description">
                {t(
                  '清洗只作用于派生正文（阅读、检索、索引），原件字节始终不变。设置按知识库保存，对之后解析的资料生效。',
                  'Cleaning only shapes the derived text used for reading, search and indexing; originals stay byte for byte. Settings are stored per workspace and apply to documents parsed afterwards.',
                )}
              </p>
              {identity ? (
                <>
                  <CleaningParamsForm
                    value={cleaningParams}
                    onChange={(next) => {
                      setCleaningParams(next);
                      if (!cleaningConflict) setCleaningMessage(null);
                    }}
                    disabled={cleaningBusy || cleaning.busy || !cleaning.settings}
                  />
                  {cleaning.error && (
                    <p className="settings-error" role="alert">
                      {cleaning.error}
                    </p>
                  )}
                </>
              ) : (
                <p className="settings-description">
                  {t('打开知识库后即可配置清洗。', 'Open a workspace to configure cleaning.')}
                </p>
              )}
            </>
          )}{' '}
          {section === 'workspace' && (
            <>
              <h3>{t('存储在你的电脑上', 'Stored on your computer')}</h3>
              <p className="settings-description">
                {t('正文与附件保存在以下位置。', 'Your notes and attachments live here.')}
              </p>
              <p className="settings-workspace-path">
                {app.active
                  ? displayPath(app.active.path)
                  : t('尚未打开知识库', 'No workspace open')}
              </p>
              <div className="settings-workspace-actions">
                <button type="button" disabled={busy} onClick={backup}>
                  <Archive size={17} aria-hidden="true" />
                  {t('备份与恢复', 'Backup & restore')}
                </button>
                <button type="button" disabled={busy} onClick={open}>
                  <FolderOpen size={17} aria-hidden="true" />
                  {t('打开知识库', 'Open workspace')}
                </button>
                <button type="button" disabled={busy} onClick={create}>
                  <FolderPlus size={17} aria-hidden="true" />
                  {t('新建知识库', 'New workspace')}
                </button>
                {app.active && (
                  <button type="button" disabled={busy} onClick={close}>
                    <LogOut size={17} aria-hidden="true" />
                    {t('关闭知识库', 'Close workspace')}
                  </button>
                )}
              </div>
            </>
          )}
          {section === 'shortcuts' && (
            <>
              <h3>{t('可自定义', 'Customizable')}</h3>
              <p className="settings-description">
                {t(
                  '这两项可以更改；一项不能与另一项相同。',
                  'These two can be changed. One may not equal the other.',
                )}
              </p>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <Keyboard size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('新建笔记', 'New note')}</strong>
                  </span>
                </span>
                <Select
                  aria-label={t('新建笔记快捷键', 'New note shortcut')}
                  value={prefs.new_note_shortcut}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ new_note_shortcut: nextValue })}
                >
                  {SHORTCUT_OPTIONS.map((option) => (
                    <option key={option} value={option} disabled={option === prefs.search_shortcut}>
                      {option}{' '}
                      {option === prefs.search_shortcut
                        ? t('（已用于搜索）', '(used by search)')
                        : ''}
                    </option>
                  ))}
                </Select>
              </div>
              <div className="choice-field settings-field">
                <span className="settings-field-copy">
                  <Keyboard size={17} aria-hidden="true" />
                  <span>
                    <strong>{t('搜索笔记列表', 'Search the note list')}</strong>
                  </span>
                </span>
                <Select
                  aria-label={t('搜索笔记列表快捷键', 'Search shortcut')}
                  value={prefs.search_shortcut}
                  disabled={busy}
                  onValueChange={(nextValue) => update({ search_shortcut: nextValue })}
                >
                  {SHORTCUT_OPTIONS.map((option) => (
                    <option
                      key={option}
                      value={option}
                      disabled={option === prefs.new_note_shortcut}
                    >
                      {option}{' '}
                      {option === prefs.new_note_shortcut
                        ? t('（已用于新建）', '(used by new note)')
                        : ''}
                    </option>
                  ))}
                </Select>
              </div>
              <h3>{t('固定快捷键', 'Fixed shortcuts')}</h3>
              <p className="settings-description">
                {t(
                  '以下按键由应用固定，可在工作台直接使用。',
                  'These keys are fixed by the application and work throughout the workspace.',
                )}
              </p>
              <dl className="settings-shortcuts">
                {[
                  {
                    label: t('新建笔记', 'New note'),
                    parts: shortcutParts(prefs.new_note_shortcut),
                  },
                  {
                    label: t('搜索笔记列表', 'Search the note list'),
                    parts: shortcutParts(prefs.search_shortcut),
                  },
                  { label: t('打开资料', 'Open documents'), parts: ['Ctrl', 'O'] },
                  {
                    label: t('关闭当前文档标签页', 'Close current document tab'),
                    parts: ['Ctrl', 'W'],
                  },
                  { label: t('保存所有打开的笔记', 'Save all open notes'), parts: ['Ctrl', 'S'] },
                  { label: t('显示或隐藏侧边栏', 'Toggle sidebar'), parts: ['Ctrl', '\\'] },
                  { label: t('打开设置', 'Open settings'), parts: ['Ctrl', ','] },
                  {
                    label: t('打开命令面板', 'Open command palette'),
                    parts: ['Ctrl', 'Shift', 'P'],
                  },
                  { label: t('后退 / 前进', 'Back / forward'), parts: ['Alt', '← / →'] },
                  {
                    label: t('界面缩放', 'Interface zoom'),
                    parts: ['Ctrl', '− / + / 0'],
                  },
                  // One key, two real behaviours: inside a callout or comment block it
                  // continues after the block, anywhere else it is the editor's hard line
                  // break. The interface has to state both, not only the container case.
                  {
                    label: t(
                      '容器内继续到块外；容器外硬换行',
                      'Continue after a block inside a container; hard line break elsewhere',
                    ),
                    parts: ['Ctrl', 'Enter'],
                  },
                  {
                    label: t('编辑器内硬换行', 'Hard line break in the editor'),
                    parts: ['Shift', 'Enter'],
                  },
                ].map((shortcut) => (
                  <div key={shortcut.label}>
                    <dt>{shortcut.label}</dt>
                    <dd>
                      {shortcut.parts.map((part, index) => (
                        <span key={part}>
                          {index > 0 && <span aria-hidden="true"> + </span>}
                          <kbd>{part}</kbd>
                        </span>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            </>
          )}
          {section === 'about' && (
            <div className="settings-about">
              <div className="settings-about-hero">
                <BrandMark size={56} />
                <div>
                  <h3>跨索 spanseek</h3>
                  <p>
                    {t('让每一次求索，都有迹可循。', 'A local-first personal knowledge workspace.')}
                  </p>
                </div>
              </div>
              <dl className="settings-about-facts">
                <div>
                  <dt>{t('版本', 'Version')}</dt>
                  <dd>
                    {aboutBusy
                      ? t('读取中…', 'Loading…')
                      : (aboutVersions?.application ?? __PKMS_APP_VERSION__)}
                  </dd>
                </div>
                <div>
                  <dt>{t('构建', 'Build')}</dt>
                  <dd>{t('稳定版', 'Stable')}</dd>
                </div>
                <div>
                  <dt>{t('平台', 'Platform')}</dt>
                  <dd>
                    {aboutVersions
                      ? `${aboutVersions.platform} · ${aboutVersions.architecture}`
                      : '—'}
                  </dd>
                </div>
                <div>
                  <dt>{t('知识库格式', 'Workspace format')}</dt>
                  <dd>{aboutVersions?.knowledge_schema ?? '—'}</dd>
                </div>
                <div>
                  <dt>WebView2</dt>
                  <dd>{aboutVersions?.webview_version ?? '—'}</dd>
                </div>
                <div>
                  <dt>{t('当前知识库', 'Current workspace')}</dt>
                  <dd>{app.active ? displayPath(app.active.path) : '—'}</dd>
                </div>
              </dl>
              {aboutError && (
                <div className="settings-about-error" role="alert">
                  <span>{aboutError}</span>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => setAboutRefresh((value) => value + 1)}
                  >
                    {t('重新读取', 'Retry')}
                  </button>
                </div>
              )}
              {!identity && (
                <p className="settings-description">
                  {t(
                    '打开知识库后可读取应用、格式与运行环境版本。',
                    'Open a workspace to read application, format and runtime versions.',
                  )}
                </p>
              )}
              <div className="settings-about-actions">
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!identity}
                  onClick={() => showSection('diagnostics')}
                >
                  <Activity size={16} aria-hidden="true" />
                  {t('打开统计与诊断', 'Open diagnostics')}
                </button>
                <button type="button" className="secondary-button" onClick={backup}>
                  <Archive size={16} aria-hidden="true" />
                  {t('打开备份与恢复', 'Open backup & restore')}
                </button>
              </div>
              <p className="settings-about-copy">
                {t(
                  '知识库与附件保存在本机；应用不内置遥测。只有在你配置远程模型或检索服务并触发请求时，才会按该端点发送必要文本。',
                  'Workspaces and attachments stay on this computer; the app includes no telemetry. Text goes only to the remote model or retrieval endpoint you configure and use.',
                )}
              </p>
              <p className="settings-about-copy">
                {t(
                  '第三方组件许可随安装包的 licenses/ 目录分发。',
                  'Third-party licenses ship in the installation’s licenses/ folder.',
                )}
              </p>
            </div>
          )}
        </section>
        {saveAction && (
          <footer className="settings-footer">
            <span className="settings-footer-status" role="status">
              {saveAction.message ??
                (!saveAction.ready
                  ? ''
                  : saveAction.dirty
                    ? t('有未保存的更改。', 'Unsaved changes.')
                    : t('当前设置已保存。', 'All changes saved.'))}
            </span>
            <div className="settings-footer-actions">
              {saveAction.reloadAvailable && (
                <button
                  type="button"
                  className="secondary-button"
                  disabled={busy || saveAction.busy || !identity}
                  onClick={saveAction.reload}
                >
                  {t('重新读取', 'Reload saved')}
                </button>
              )}
              {!saveAction.reloadAvailable && (
                <button
                  type="button"
                  className="secondary-button"
                  disabled={
                    busy || saveAction.busy || !saveAction.ready || !saveAction.resetAvailable
                  }
                  onClick={saveAction.reset}
                >
                  {t('恢复默认值', 'Restore defaults')}
                </button>
              )}
              <button
                type="button"
                className="primary-button"
                disabled={
                  busy ||
                  saveAction.busy ||
                  !saveAction.ready ||
                  !saveAction.dirty ||
                  saveAction.conflict
                }
                onClick={() => void saveAction.run()}
              >
                {saveAction.saving ? t('正在保存…', 'Saving…') : t('保存', 'Save')}
              </button>
            </div>
          </footer>
        )}
      </div>
    </div>
  );
  const reloadConfirmation = visible && reloadSection && (
    <ActionConfirmDialog
      title={t('重新读取已保存设置？', 'Reload saved settings?')}
      description={t(
        '这会用当前知识库中最新保存的设置替换本页尚未保存的修改。取消会保留你的输入。',
        'This replaces the unsaved edits on this page with the latest saved settings in this workspace. Cancel to keep your input.',
      )}
      confirmLabel={t('放弃修改并重新读取', 'Discard edits and reload')}
      onCancel={() => setReloadSection(null)}
      onConfirm={() => void reloadSavedSettings(reloadSection)}
    />
  );
  if (embedded)
    return (
      <section className="settings-page" aria-labelledby={titleId}>
        {content}
        {reloadConfirmation}
      </section>
    );
  return (
    <dialog
      ref={dialog}
      className="settings-dialog"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onCancel={(event) => {
        if (busy) event.preventDefault();
      }}
      onClose={(event) => {
        if (!event.currentTarget.open && visible) onDismiss();
      }}
    >
      {content}
      {reloadConfirmation}
    </dialog>
  );
}
