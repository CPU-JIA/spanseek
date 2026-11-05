import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { AlertCircle, ClipboardCopy, Download, Eye, RefreshCw } from 'lucide-react';
import type {
  AppCapabilities,
  DiagnosticCategory,
  DesktopData,
  DiagnosticVersions,
  OcrStatus,
  ParseComponentIssue,
  WorkspaceIdentity,
  WorkspaceStatistics,
} from '@pkms/contracts';
import { invoke } from '@tauri-apps/api/core';
import { call, choose, message } from '../api';
import { portableActions, usePortableActions } from '../portability/actions';
import {
  acceptedTaskDestination,
  type OpenTasks,
  type TaskDestination,
} from '../portability/task-navigation';
import { parseComponentNotice } from '../sources/source-import-failure';
import { useI18n } from '../i18n';
import { WorkspaceInventory } from './WorkspaceInventory';
import { DiagnosticsPreview } from './DiagnosticsPreview';
import { RuntimeEnvironment } from './RuntimeEnvironment';
import { diagnosticCategoryLabel, diagnosticReportText } from './diagnostics-report';
import { useRetrievalSettings } from '../inference/retrieval-settings';
import { PageState } from '../layout/PageState';
import { PageFrame } from '../layout/PageFrame';
import { SectionSwitch } from '../ui/SectionSwitch';
import './diagnostics.css';

type DiagnosticsPreview = Extract<DesktopData, { kind: 'diagnostics_preview' }>['payload'];

export type DiagnosticsPageProps = {
  identity: WorkspaceIdentity;
  onTasks: OpenTasks;
  /**
   * Parse components this installation cannot use. The kernel reports them with the
   * exact path it looked for, which is local information: the exported bundle stays
   * path-free, so this list is shown here instead of inside the export.
   */
  components: ParseComponentIssue[];
  embedded?: boolean;
};
export function DiagnosticsPage(props: DiagnosticsPageProps) {
  return (
    <DiagnosticsScope
      key={`${props.identity.local_instance_id}/${props.identity.workspace_id}`}
      {...props}
    />
  );
}
function DiagnosticsScope({
  identity,
  onTasks,
  components,
  embedded = false,
}: DiagnosticsPageProps) {
  const retrieval = useRetrievalSettings(identity);
  const { t, language } = useI18n();
  const [section, setSection] = useState<'overview' | 'runtime' | 'export'>('overview');
  const [statistics, setStatistics] = useState<WorkspaceStatistics | null>(null);
  const [categories, setCategories] = useState<DiagnosticCategory[]>(['versions', 'statistics']);
  const [preview, setPreview] = useState<DiagnosticsPreview | null>(null);
  const [activity, setActivity] = useState<'statistics' | 'preview' | 'export' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [statisticsError, setStatisticsError] = useState<string | null>(null);
  const [accepted, setAccepted] = useState<TaskDestination | null>(null);
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const [environment, setEnvironment] = useState<DiagnosticVersions | null>(null);
  const [environmentState, setEnvironmentState] = useState<'loading' | 'ready' | 'failed'>(
    'loading',
  );
  const [environmentError, setEnvironmentError] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<AppCapabilities | null>(null);
  const [capabilitiesState, setCapabilitiesState] = useState<'loading' | 'ready' | 'failed'>(
    'loading',
  );
  const [capabilitiesError, setCapabilitiesError] = useState<string | null>(null);
  const [ocr, setOcr] = useState<OcrStatus | null>(null);
  const [focusRequest, setFocusRequest] = useState<{ target: HTMLButtonElement } | null>(null);
  const requests = usePortableActions();
  useEffect(() => {
    let live = true;
    void invoke<OcrStatus>('ocr_status').then(
      (value) => {
        if (live) setOcr(value);
      },
      () => {
        if (live) setOcr(null);
      },
    );
    return () => {
      live = false;
    };
  }, []);
  const alive = useRef(true);
  const locked = useRef(false);
  const previewSequence = useRef(0);
  const pending = requests.pending.filter(
    (item) =>
      item.command.kind === 'diagnostics_export' &&
      item.identity?.local_instance_id === identity.local_instance_id &&
      item.identity.workspace_id === identity.workspace_id,
  );
  const blocked = !!activity || !requests.loaded || pending.length > 0;
  const queueError = !requests.loaded || pending.length > 0 ? requests.error : null;
  useEffect(() => {
    alive.current = true;
    void refreshStatistics();
    void loadEnvironment();
    void loadCapabilities();
    return () => {
      alive.current = false;
      ++previewSequence.current;
    };
  }, []);
  useLayoutEffect(() => {
    const target = focusRequest?.target;
    if (
      !alive.current ||
      locked.current ||
      !target?.isConnected ||
      target.disabled ||
      target.closest('[inert]') ||
      (document.activeElement !== document.body && document.activeElement !== target)
    )
      return;
    target.focus({ preventScroll: true });
  }, [focusRequest]);
  async function runWithButtonFocus(target: HTMLButtonElement, run: () => Promise<void>) {
    const wasFocused = document.activeElement === target;
    try {
      await run();
    } finally {
      if (alive.current && wasFocused) setFocusRequest({ target });
    }
  }
  async function refreshStatistics() {
    if (locked.current) return;
    locked.current = true;
    setActivity('statistics');
    setStatisticsError(null);
    try {
      const result = await call({ kind: 'workspace_statistics' }, 'workspace_statistics', identity);
      if (alive.current) {
        setStatistics(result);
        setStatisticsError(null);
      }
    } catch (error) {
      if (alive.current) setStatisticsError(message(error));
    } finally {
      locked.current = false;
      if (alive.current) setActivity(null);
    }
  }
  /**
   * The runtime versions, read on their own so the page can name an outdated WebView2
   * before anyone previews an export. The kernel reports the version; which version is
   * enough lives in `webview-compat.ts`. This read is display-only: no export uses its
   * fingerprint.
   */
  async function loadEnvironment() {
    setEnvironmentState('loading');
    setEnvironmentError(null);
    try {
      const result = await call(
        { kind: 'diagnostics_preview', payload: { categories: ['versions'] } },
        'diagnostics_preview',
        identity,
      );
      if (!alive.current) return;
      setEnvironment(result.snapshot.versions);
      setEnvironmentState('ready');
    } catch (error) {
      if (!alive.current) return;
      setEnvironment(null);
      setEnvironmentError(message(error));
      setEnvironmentState('failed');
    }
  }
  async function loadCapabilities() {
    setCapabilitiesState('loading');
    setCapabilitiesError(null);
    try {
      const result = await call({ kind: 'capabilities' }, 'capabilities');
      if (!alive.current) return;
      setCapabilities(result);
      setCapabilitiesState('ready');
    } catch (error) {
      if (!alive.current) return;
      setCapabilities(null);
      setCapabilitiesError(message(error));
      setCapabilitiesState('failed');
    }
  }
  async function copySummary() {
    if (!preview) return;
    try {
      if (!navigator.clipboard)
        throw new Error(t('无法访问剪贴板。', 'The clipboard is unavailable.'));
      await navigator.clipboard.writeText(diagnosticReportText(preview.snapshot, language));
      if (alive.current) setCopyState('copied');
    } catch {
      if (alive.current) setCopyState('failed');
    }
  }
  function toggle(category: DiagnosticCategory) {
    if (locked.current) return;
    ++previewSequence.current;
    setPreview(null);
    setAccepted(null);
    setCopyState('idle');
    setCategories((old) =>
      old.includes(category) ? old.filter((item) => item !== category) : [...old, category],
    );
  }
  async function makePreview() {
    if (locked.current || !categories.length) return;
    locked.current = true;
    setActivity('preview');
    setPreview(null);
    setError(null);
    setAccepted(null);
    setCopyState('idle');
    const sequence = ++previewSequence.current;
    try {
      const result = await call(
        { kind: 'diagnostics_preview', payload: { categories: [...categories] } },
        'diagnostics_preview',
        identity,
      );
      if (alive.current && sequence === previewSequence.current) setPreview(result);
    } catch (error) {
      if (alive.current && sequence === previewSequence.current) setError(message(error));
    } finally {
      locked.current = false;
      if (alive.current) setActivity(null);
    }
  }
  async function exportPreview() {
    if (blocked || locked.current || !preview) return;
    locked.current = true;
    setActivity('export');
    setError(null);
    setAccepted(null);
    const frozen = structuredClone(preview);
    try {
      const grant = await choose('export_file', identity);
      if (!grant || !alive.current) return;
      if (
        grant.purpose !== 'export_file' ||
        grant.identity?.local_instance_id !== identity.local_instance_id ||
        grant.identity.workspace_id !== identity.workspace_id ||
        !Number.isFinite(Date.parse(grant.expires_at)) ||
        Date.parse(grant.expires_at) <= Date.now()
      ) {
        throw new Error(
          t(
            '所选位置已失效，请重新选择。',
            'The selected location is unavailable. Select it again.',
          ),
        );
      }
      const commandId = crypto.randomUUID();
      const result = await portableActions.submit({
        id: commandId,
        identity,
        command: {
          kind: 'diagnostics_export',
          payload: {
            command_id: commandId,
            token: grant.token,
            categories: frozen.categories,
            fingerprint: frozen.fingerprint,
          },
        },
        displayName: t('诊断包', 'Diagnostics bundle'),
        createdAt: new Date().toISOString(),
      });
      if (alive.current) {
        setAccepted(acceptedTaskDestination(result));
        setPreview(null);
        setCopyState('idle');
      }
    } catch (error) {
      if (alive.current) {
        const code = (error as { details?: { code?: string } })?.details?.code;
        if (code === 'app_state_conflict') {
          setPreview(null);
          setError(
            t(
              '统计或诊断信息已变化，请重新预览后导出。',
              'Diagnostics have changed. Preview the current information before exporting.',
            ),
          );
        } else setError(message(error));
      }
    } finally {
      locked.current = false;
      if (alive.current) setActivity(null);
    }
  }
  async function retry(id: string) {
    if (locked.current) return;
    locked.current = true;
    setActivity('export');
    setError(null);
    try {
      const result = await portableActions.retry(id);
      if (alive.current) {
        setAccepted(acceptedTaskDestination(result));
        setPreview(null);
      }
    } catch (error) {
      if (alive.current) {
        const code = (error as { details?: { code?: string } })?.details?.code;
        if (code === 'app_state_conflict') {
          setPreview(null);
          setError(
            t(
              '统计或诊断信息已变化，请重新预览后导出。',
              'Diagnostics have changed. Preview the current information before exporting.',
            ),
          );
        } else setError(message(error));
      }
    } finally {
      locked.current = false;
      if (alive.current) setActivity(null);
    }
  }
  return (
    <PageFrame
      className="diagnostics-page"
      title={t('统计与诊断', 'Statistics and diagnostics')}
      titleHidden={embedded}
      description={t(
        '查看知识库的真实使用情况，按需导出本地诊断信息。',
        'View workspace inventory and export local diagnostics when needed.',
      )}
      toolbar={
        <SectionSwitch
          label={t('诊断分类', 'Diagnostic sections')}
          value={section}
          onChange={setSection}
          options={[
            { value: 'overview', label: t('知识库概览', 'Workspace overview') },
            { value: 'runtime', label: t('运行环境', 'Runtime environment') },
            { value: 'export', label: t('诊断导出', 'Export diagnostics') },
          ]}
        />
      }
    >
      {(error || queueError) && (
        <div className="diagnostics-alert" role="alert">
          <AlertCircle size={17} aria-hidden="true" />
          <span>{error || queueError}</span>
          {!requests.loaded && requests.error && (
            <button
              className="secondary-button"
              onClick={() =>
                void portableActions.load().catch((error) => {
                  if (alive.current) setError(message(error));
                })
              }
            >
              {t('重新读取请求记录', 'Reload request records')}
            </button>
          )}
        </div>
      )}
      <section hidden={section !== 'runtime'} aria-label={t('运行环境', 'Runtime environment')}>
        <h2>{t('运行环境', 'Runtime environment')}</h2>
        {environmentState === 'failed' ? (
          <PageState
            className="diagnostics-state"
            compact
            kind="error"
            title={t('无法读取运行环境', 'Could not read the runtime environment')}
            detail={
              environmentError ??
              t('无法读取运行环境信息。', 'The runtime environment could not be read.')
            }
            retryLabel={t('重新读取运行环境', 'Reload runtime environment')}
            onRetry={() => void loadEnvironment()}
          />
        ) : !environment ? (
          <PageState
            className="diagnostics-state"
            compact
            kind="loading"
            title={t('正在读取运行环境…', 'Reading the runtime environment…')}
          />
        ) : (
          <RuntimeEnvironment versions={environment} />
        )}
      </section>
      <section hidden={section !== 'runtime'} aria-label={t('格式支持', 'Format support')}>
        <h2>{t('格式支持', 'Format support')}</h2>
        {capabilitiesState === 'failed' ? (
          <div className="diagnostics-alert" role="alert">
            <AlertCircle size={17} aria-hidden="true" />
            <span>
              {capabilitiesError || t('无法读取格式支持信息。', 'Format support is unavailable.')}
            </span>
            <button className="secondary-button" onClick={() => void loadCapabilities()}>
              {t('重新读取格式支持', 'Reload format support')}
            </button>
          </div>
        ) : !capabilities ? (
          <p className="diagnostics-muted" role="status">
            {t('正在读取格式支持…', 'Reading format support…')}
          </p>
        ) : (
          <div className="diagnostics-counts">
            <dl>
              <div>
                <dt>{t('正文格式', 'Document schemas')}</dt>
                <dd>{capabilities.document_schemas.map((schema) => 'v' + schema).join(' · ')}</dd>
              </div>
              <div>
                <dt>{t('图片格式', 'Image formats')}</dt>
                <dd>{capabilities.image_formats.join(' · ')}</dd>
              </div>
            </dl>
          </div>
        )}
      </section>
      <section hidden={section !== 'overview'} aria-busy={activity === 'statistics'}>
        <div className="diagnostics-toolbar">
          <h2>{t('知识库统计', 'Workspace inventory')}</h2>
          <button
            disabled={!!activity}
            onClick={(event) => void runWithButtonFocus(event.currentTarget, refreshStatistics)}
            aria-label={t('刷新统计', 'Refresh statistics')}
          >
            <RefreshCw size={15} aria-hidden="true" />
          </button>
        </div>
        {statisticsError && statistics && (
          <div className="diagnostics-alert" role="alert">
            <AlertCircle size={17} aria-hidden="true" />
            <span>{statisticsError}</span>
            <button
              className="secondary-button"
              disabled={!!activity}
              onClick={() => void refreshStatistics()}
            >
              {t('重新读取统计', 'Reload statistics')}
            </button>
          </div>
        )}
        {statistics ? (
          <WorkspaceInventory
            statistics={statistics}
            prices={{
              input: retrieval.settings?.params.cost_per_million_input ?? null,
              output: retrieval.settings?.params.cost_per_million_output ?? null,
            }}
          />
        ) : activity === 'statistics' ? (
          <PageState
            className="diagnostics-state"
            compact
            kind="loading"
            title={t('正在读取统计…', 'Loading statistics…')}
          />
        ) : statisticsError ? (
          <PageState
            className="diagnostics-state"
            compact
            kind="error"
            title={t('无法读取知识库统计', 'Could not load workspace inventory')}
            detail={statisticsError}
            retryLabel={t('重新读取统计', 'Reload statistics')}
            onRetry={() => void refreshStatistics()}
          />
        ) : (
          <p className="diagnostics-muted">
            {t('统计尚未读取，请刷新重试。', 'Statistics are unavailable. Refresh to try again.')}
          </p>
        )}
      </section>
      <section hidden={section !== 'runtime'} aria-label={t('解析组件', 'Parse components')}>
        <h2>{t('解析组件', 'Parse components')}</h2>
        {components.length ? (
          <ul className="diagnostics-components">
            {components.map((issue) => (
              <li key={`${issue.code}:${issue.path ?? ''}`} title={issue.code}>
                {parseComponentNotice(issue, t)}
              </li>
            ))}
          </ul>
        ) : (
          <p className="diagnostics-muted">
            {t(
              '解析组件齐全，PDF、Word（DOCX/DOC）、Excel（XLSX/XLS）、PowerPoint（PPTX）、EPUB/MOBI、CSV、Markdown 和 TXT 都能导入。',
              'Parse components are in place, so PDF, Word (DOCX/DOC), Excel (XLSX/XLS), PowerPoint (PPTX), EPUB/MOBI, CSV, Markdown and TXT can be imported.',
            )}
          </p>
        )}
      </section>
      <section
        hidden={section !== 'runtime'}
        aria-label={t('OCR（扫描版 PDF）', 'OCR (scanned PDFs)')}
      >
        <h2>{t('OCR（扫描版 PDF）', 'OCR (scanned PDFs)')}</h2>
        {ocr === null ? (
          <p className="diagnostics-muted">{t('正在读取 OCR 状态…', 'Reading OCR status…')}</p>
        ) : ocr.available ? (
          <p className="diagnostics-muted" title={ocr.executable ?? undefined}>
            {t('已启用本机 OCR。语言包：', 'Local OCR is enabled. Language packs: ')}
            {ocr.languages.length ? ocr.languages.join(', ') : t('未找到', 'none found')}
            {' · '}
            {ocr.configured_languages}
            {' · '}
            {ocr.dpi} DPI
          </p>
        ) : (
          <p className="diagnostics-muted">
            {t(
              '未找到可用的本地 OCR：扫描版 PDF 只会标记“需要 OCR”，不会静默生成文本。正式安装包应随附 tesseract/ 及简体中文、英文语言包；开发或自定义安装也可设置 PKMS_TESSERACT_EXE / PKMS_TESSDATA_DIR。',
              'No usable local OCR engine: scanned PDFs are only flagged as needing OCR and never get silent text. The installer should contain tesseract/ with Simplified Chinese and English packs; development or custom installs can set PKMS_TESSERACT_EXE / PKMS_TESSDATA_DIR.',
            )}
          </p>
        )}
      </section>
      <section hidden={section !== 'export'}>
        <h2>{t('导出诊断包', 'Export diagnostics')}</h2>
        <p className="diagnostics-muted">
          {t(
            '先选择和预览，再复制摘要或保存到本机。诊断包不包含笔记正文、模型输入、密钥或本机路径。',
            'Choose what to include, preview it, then copy the summary or save it locally. The bundle excludes note content, model inputs, secrets and local paths.',
          )}
        </p>
        <fieldset className="diagnostics-categories" disabled={blocked}>
          <legend>{t('包含的信息', 'Include')}</legend>
          {(['versions', 'statistics', 'errors'] as const).map((category) => (
            <label key={category}>
              <input
                type="checkbox"
                checked={categories.includes(category)}
                onChange={() => toggle(category)}
              />
              {diagnosticCategoryLabel(category, language)}
            </label>
          ))}
        </fieldset>
        <div className="diagnostics-toolbar">
          <button
            disabled={blocked || !categories.length}
            onClick={(event) => void runWithButtonFocus(event.currentTarget, makePreview)}
          >
            <Eye size={15} aria-hidden="true" />
            {activity === 'preview'
              ? t('正在预览…', 'Loading preview…')
              : t('预览导出内容', 'Preview contents')}
          </button>
          <button disabled={blocked || !preview} onClick={() => void copySummary()}>
            <ClipboardCopy size={15} aria-hidden="true" />
            {t('复制摘要', 'Copy summary')}
          </button>
          <button
            className="primary"
            disabled={blocked || !preview}
            onClick={(event) => void runWithButtonFocus(event.currentTarget, exportPreview)}
          >
            <Download size={15} aria-hidden="true" />
            {activity === 'export'
              ? t('正在提交…', 'Submitting…')
              : t('选择位置并导出', 'Choose location and export')}
          </button>
        </div>
        {preview && copyState === 'copied' && (
          <p role="status">
            {t('诊断摘要已复制到剪贴板。', 'The summary was copied to the clipboard.')}
          </p>
        )}
        {preview && copyState === 'failed' && (
          <p role="alert">
            {t(
              '复制失败，可以改用「选择位置并导出」保存诊断包。',
              'Copying failed. Use “Choose location and export” to save the bundle instead.',
            )}
          </p>
        )}
        {preview && <DiagnosticsPreview snapshot={preview.snapshot} />}
        {pending.map((item) => (
          <p role="status" key={item.id}>
            {t('导出请求的结果尚待确认。', 'The export request needs confirmation.')}{' '}
            <button
              disabled={!!activity || requests.busy.includes(item.id)}
              onClick={() => void retry(item.id)}
            >
              {t('确认请求结果', 'Confirm request')}
            </button>
          </p>
        ))}
        {accepted && (
          <p role="status">
            {t(
              '导出任务已创建，完成状态和结果见任务中心。',
              'Export task created. View its progress and result in Tasks.',
            )}{' '}
            <button onClick={() => onTasks(accepted)}>{t('查看任务', 'View tasks')}</button>
          </p>
        )}
      </section>
    </PageFrame>
  );
}
