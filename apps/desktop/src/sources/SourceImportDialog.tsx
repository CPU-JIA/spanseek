import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { FileUp, FolderOpen, Globe, X } from 'lucide-react';
import type { SourceJob, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { SourceCollectionPicker, type SourceCollectionChoice } from './SourceCollectionPicker';
import { SourceImportTagPicker } from './SourceImportTagPicker';
import { tagToneClass } from '../ui/tag-color';
import {
  failedSourceJobs,
  scanSkipReason,
  sourceBatchProgress,
  sourceBatchProgressLabel,
  sourceBatchProgressParts,
  sourceImportFailures,
  type SourceFileImport,
  type SourceImportSelection,
  type SourceScanReport,
} from './source-import-batch';
import {
  groupImportFailures,
  importFailureSummary,
  sourceImportFailure,
  sourceImportFailureGroup,
  sourceImportFailureGroupLabel,
} from './source-import-failure';

export function SourceLibraryDialog({
  title,
  children,
  onClose,
  className = '',
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  className?: string;
}) {
  const { t } = useI18n();
  const titleId = useId();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.showModal();
    return () => {
      dialog.current?.close();
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialog}
      className={'source-library-dialog ' + className}
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const bounds = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < bounds.left ||
          event.clientX > bounds.right ||
          event.clientY < bounds.top ||
          event.clientY > bounds.bottom
        )
          onClose();
      }}
    >
      <header className="source-library-dialog-header">
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('关闭', 'Close')}
          onClick={onClose}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      {children}
    </dialog>
  );
}

export function SourceImportDialog({
  identity,
  importing,
  locked,
  error,
  onFiles,
  onDirectory,
  onWebpage,
  initialCollection,
  onClose,
}: {
  identity: WorkspaceIdentity;
  importing: boolean;
  locked: boolean;
  error: string | null;
  onFiles: (selection: SourceImportSelection) => Promise<boolean>;
  onDirectory: (selection: SourceImportSelection) => Promise<boolean>;
  onWebpage: (address: string, selection: SourceImportSelection) => Promise<boolean>;
  initialCollection?: SourceCollectionChoice | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [mode, setMode] = useState<'files' | 'folder' | 'web'>('files');
  const [url, setUrl] = useState('');
  const [collection, setCollection] = useState<SourceCollectionChoice | null>(
    initialCollection ?? null,
  );
  const [organizationMode, setOrganizationMode] = useState<'now' | 'later'>(
    initialCollection ? 'now' : 'later',
  );
  const [tagNames, setTagNames] = useState<string[]>([]);
  const id = useId();
  const fileButton = useRef<HTMLButtonElement>(null);
  const addressInput = useRef<HTMLInputElement>(null);
  const pending = importing || locked;
  useEffect(() => {
    fileButton.current?.focus();
  }, []);
  async function submit() {
    const selection: SourceImportSelection =
      organizationMode === 'now'
        ? { collectionId: collection?.id ?? null, tagNames }
        : { collectionId: null, tagNames: [] };
    const started =
      mode === 'files'
        ? await onFiles(selection)
        : mode === 'folder'
          ? await onDirectory(selection)
          : await onWebpage(url, selection);
    if (started) onClose();
  }
  return (
    <SourceLibraryDialog title={t('导入资料', 'Import documents')} onClose={onClose}>
      <div
        className="source-import-kinds"
        role="tablist"
        aria-label={t('导入方式', 'Import method')}
      >
        {(['files', 'folder', 'web'] as const).map((kind) => (
          <button
            key={kind}
            type="button"
            role="tab"
            id={id + '-' + kind}
            aria-selected={mode === kind}
            aria-controls={id + '-panel'}
            tabIndex={mode === kind ? 0 : -1}
            disabled={importing}
            onClick={() => setMode(kind)}
            onKeyDown={(event) => {
              if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
                event.preventDefault();
                const order = ['files', 'folder', 'web'] as const;
                const index = order.indexOf(mode);
                const next =
                  event.key === 'Home'
                    ? 'files'
                    : event.key === 'End'
                      ? 'web'
                      : order[
                          (index + (event.key === 'ArrowLeft' ? order.length - 1 : 1)) %
                            order.length
                        ];
                setMode(next);
                requestAnimationFrame(() => document.getElementById(id + '-' + next)?.focus());
              }
            }}
          >
            {kind === 'files' ? (
              <FileUp size={17} aria-hidden="true" />
            ) : kind === 'folder' ? (
              <FolderOpen size={17} aria-hidden="true" />
            ) : (
              <Globe size={17} aria-hidden="true" />
            )}
            {kind === 'files'
              ? t('本地文件', 'Local files')
              : kind === 'folder'
                ? t('本地文件夹', 'Local folder')
                : t('网页链接', 'Webpage URL')}
          </button>
        ))}
      </div>
      <form
        className="source-import-form"
        role="tabpanel"
        id={id + '-panel'}
        aria-labelledby={id + '-' + mode}
        onSubmit={(event) => {
          event.preventDefault();
          if (!pending) void submit();
        }}
      >
        {mode === 'files' ? (
          <div className="source-file-intro">
            <FileUp size={32} aria-hidden="true" />
            <strong>{t('打开文件，开始阅读', 'Open a file and start reading')}</strong>
            <p>
              {t(
                'PDF、DOCX、DOC、Markdown、UTF-8 TXT、EPUB、MOBI、CSV、XLS/XLSX 和 PPTX，可一次选择多个文件。',
                'PDF, DOCX, DOC, Markdown, UTF-8 TXT, EPUB, MOBI, CSV, XLS/XLSX and PPTX. Select multiple files at once.',
              )}
            </p>
            <small>
              {t(
                '文件保存在当前知识库，原件保持不变。',
                'Documents are saved in this workspace. Original files stay unchanged.',
              )}
            </small>
          </div>
        ) : mode === 'folder' ? (
          <div className="source-file-intro">
            <FolderOpen size={32} aria-hidden="true" />
            <strong>{t('扫描文件夹并导入', 'Scan a folder and import')}</strong>
            <p>
              {t(
                '递归扫描所选文件夹及其子文件夹，导入支持的全部文档、电子书、表格和演示格式。不支持或无法读取的文件会逐项列出原因。',
                'Recursively scans the chosen folder and its subfolders for every supported document, ebook, spreadsheet and presentation format. Files that cannot be imported are listed with a reason.',
              )}
            </p>
            <small>
              {t(
                '一次最多受理 500 个文件；扫描只读取，不修改原文件。',
                'Up to 500 files per scan; the scan only reads and never modifies originals.',
              )}
            </small>
          </div>
        ) : (
          <label className="source-field-label">
            <span>{t('公开网页地址', 'Public webpage URL')}</span>
            <input
              ref={addressInput}
              type="url"
              required
              value={url}
              placeholder="https://"
              disabled={pending}
              onChange={(event) => setUrl(event.target.value)}
            />
            <small>
              {t(
                '保存可阅读的网页快照。需要登录的页面可能无法提取正文。',
                'Save a readable snapshot. Pages requiring sign-in may have no extractable text.',
              )}
            </small>
          </label>
        )}
        <section className="source-import-organization">
          <div
            className="source-import-organization-mode"
            role="radiogroup"
            aria-label={t('导入后的整理方式', 'Organization after import')}
          >
            <span>{t('整理方式', 'Organization')}</span>
            <div>
              <button
                type="button"
                role="radio"
                aria-checked={organizationMode === 'now'}
                disabled={pending}
                onClick={() => setOrganizationMode('now')}
              >
                {t('现在整理', 'Organize now')}
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={organizationMode === 'later'}
                disabled={pending}
                onClick={() => {
                  setCollection(null);
                  setTagNames([]);
                  setOrganizationMode('later');
                }}
              >
                {t('稍后整理', 'Organize later')}
              </button>
            </div>
          </div>
          {organizationMode === 'now' && (
            <>
              <SourceCollectionPicker
                identity={identity}
                value={collection}
                onChange={setCollection}
                disabled={pending}
                label={t('分类（可选）', 'Category (optional)')}
              />
              <SourceImportTagPicker
                identity={identity}
                selected={tagNames}
                onChange={setTagNames}
                disabled={pending}
              />
            </>
          )}
          <small className="source-collection-hint">
            {organizationMode === 'now'
              ? t(
                  '选择分类或标签后，资料导入即标记为已整理；分类和标签都留空时仍会进入收件箱。',
                  'Choosing a category or tag marks the document as organized on import. With neither, it waits in the inbox.',
                )
              : t(
                  '资料会进入统一收件箱，可稍后批量分类、加标签或完成整理。',
                  'The document goes to the unified inbox for later classification, tagging or completion.',
                )}
          </small>
        </section>
        {error && (
          <p className="source-dialog-error" role="alert">
            {error}
          </p>
        )}
        {importing && (
          <p className="source-import-background" role="status">
            {t(
              '收起后导入将继续，可在导入记录中查看。',
              'Import continues in the background. Check import history for results.',
            )}
          </p>
        )}
        <footer className="source-library-dialog-footer">
          <button type="button" onClick={onClose}>
            {importing ? t('收起', 'Hide') : t('取消', 'Cancel')}
          </button>
          <button
            ref={fileButton}
            className="primary-button"
            disabled={pending || (mode === 'web' && !url.trim())}
          >
            {importing
              ? t('正在导入…', 'Importing…')
              : mode === 'files'
                ? t('选择并导入', 'Choose and import')
                : mode === 'folder'
                  ? t('选择文件夹并扫描', 'Choose folder and scan')
                  : t('保存网页', 'Save webpage')}
          </button>
        </footer>
      </form>
    </SourceLibraryDialog>
  );
}

export function sourceJobLabel(job: SourceJob, t: (zh: string, en: string) => string) {
  if (job.status === 'failed') return t('导入失败', 'Import failed');
  if (job.status === 'canceled') return t('已取消', 'Cancelled');
  if (job.stage === 'needs_ocr')
    return t('已保存 · 含无文字页面', 'Saved · Includes pages without text');
  if (job.stage === 'existing_trashed_source')
    return t('重复资料 · 已在回收站', 'Existing document · In trash');
  if (job.stage === 'existing_historical_version')
    return t('已有资料的历史版本', 'An existing document version');
  if (job.stage === 'existing_source')
    return t('已有资料 · 可以阅读', 'Already in library · Ready to read');
  if (job.status === 'succeeded') return t('可以阅读', 'Ready to read');
  if (job.stage === 'parsing') return t('正在读取正文', 'Reading text');
  if (job.stage === 'indexing') return t('正在准备检索', 'Preparing search');
  if (job.stage === 'capturing') return t('正在保存原件', 'Saving original');
  return t('正在导入', 'Importing');
}

export function SourceImportActivity({
  items,
  jobs,
  loaded,
  error,
  pending,
  requestError,
  importing,
  locked,
  scan,
  onRetry,
  onRetryJob,
  onConfirmPending,
  onAcknowledge,
  onContinue,
  onCancelRemaining,
  onDismissScan,
  onOpenJob,
  onOpenTasks,
  onGoInbox,
  onOrganizeImported,
}: {
  items: SourceFileImport[];
  jobs: SourceJob[];
  loaded: boolean;
  error: string | null;
  pending: boolean;
  requestError: string | null;
  importing: boolean;
  locked: boolean;
  scan: SourceScanReport | null;
  onRetry: () => void;
  onRetryJob?: (job: SourceJob) => void;
  onConfirmPending: () => void;
  onAcknowledge?: () => void;
  onContinue: () => void;
  onCancelRemaining: () => void;
  onDismissScan: () => void;
  onOpenJob: (job: SourceJob) => void;
  onOpenTasks?: () => void;
  onGoInbox?: () => void;
  onOrganizeImported?: (sourceIds: string[]) => void;
}) {
  const { t } = useI18n();
  const batchTokens = new Set(items.map((item) => item.token));
  const jobsById = new Map(jobs.map((job) => [job.id, job]));
  const jobsByToken = new Map(jobs.map((job) => [job.input.input_token, job]));
  const recent = jobs.filter((job) => !batchTokens.has(job.input.input_token));
  const progress = sourceBatchProgress(items, jobs);
  const progressParts = sourceBatchProgressParts(progress, t);
  // One grouping rule for both lists: what the batch produced, and what the
  // library already recorded, bundle by the cause the user has to act on. A
  // cancellation stays its own group so it is never read as a failure.
  const batchFailures = groupImportFailures(sourceImportFailures(items, jobs), (entry) =>
    sourceImportFailureGroup(entry.code),
  );
  const recentFailed = failedSourceJobs(recent);
  const failedIds = new Set(recentFailed.map((job) => job.id));
  const recentRest = recent.filter((job) => !failedIds.has(job.id));
  const recentFailures = groupImportFailures(recentFailed, (job) =>
    sourceImportFailureGroup(job.error_code),
  );
  const importedSourceIds = [
    ...new Set(
      items.flatMap((item) => {
        const job = item.jobId ? jobsById.get(item.jobId) : jobsByToken.get(item.token);
        return job?.source_id ? [job.source_id] : [];
      }),
    ),
  ];

  function jobDetails(job: SourceJob) {
    const tagNames = job.tag_names ?? [];
    return (
      <>
        <span className="source-import-status" data-status={job.status}>
          {sourceJobLabel(job, t)}
        </span>
        {job.source_id && (
          <button type="button" onClick={() => onOpenJob(job)}>
            {t('打开原文', 'Open document')}
          </button>
        )}
        {tagNames.length > 0 && (
          <span className="source-import-job-tags">
            {tagNames.map((name) => (
              <span key={name} className={`tag-inline ${tagToneClass(name)}`}>
                #{name}
              </span>
            ))}
          </span>
        )}
        {job.error_code && (
          <small className="source-import-error" title={job.error_code}>
            {sourceImportFailure(job.error_code, t)}
          </small>
        )}
        {(job.status === 'failed' || job.status === 'canceled') && onRetryJob && (
          <button type="button" disabled={importing || locked} onClick={() => onRetryJob(job)}>
            {t('重试此项', 'Retry this item')}
          </button>
        )}
      </>
    );
  }
  function jobRow(job: SourceJob) {
    return (
      <li key={job.id}>
        <strong title={job.input.original_name}>{job.input.original_name}</strong>
        {jobDetails(job)}
      </li>
    );
  }
  return (
    <div className="source-import-activity">
      {pending && (
        <div className="source-pending-import" role="status">
          <p>
            {t(
              '导入结果尚未确认，原请求已保留。',
              'Import acceptance is unconfirmed. The original request is retained.',
            )}
          </p>
          <button type="button" disabled={importing} onClick={onConfirmPending}>
            {t('核对原请求', 'Check request')}
          </button>
        </div>
      )}
      {requestError && (
        <p className="source-dialog-error" role="alert">
          {requestError}
        </p>
      )}
      {scan && (
        <section className="source-import-scan">
          <h3>{t(`文件夹扫描：${scan.rootName}`, `Folder scan: ${scan.rootName}`)}</h3>
          <p className="muted">
            {t(
              `已检查 ${scan.visited} 个文件：可直接导入 ${scan.ready} 个，与库内内容相同 ${scan.duplicates.length} 个，未导入 ${scan.skipped.length} 个。`,
              `Inspected ${scan.visited} files: ${scan.ready} importable, ${scan.duplicates.length} already in the library, ${scan.skipped.length} not imported.`,
            )}
          </p>
          {scan.truncated && (
            <p className="source-dialog-error" role="alert">
              {t(
                '文件夹内文件过多，本次只检查了前 5000 个。可分批选择子文件夹继续。',
                'The folder holds more files than one scan reads; only the first 5000 were inspected. Scan subfolders in batches.',
              )}
            </p>
          )}
          {scan.limited > 0 && (
            <p className="source-dialog-error" role="alert">
              {t(
                `另有 ${scan.limited} 个文件超过单次 500 个的受理上限，本次未提交。`,
                `${scan.limited} more files are past the 500-file limit and were not submitted.`,
              )}
            </p>
          )}
          {scan.deepSkipped > 0 && (
            <p className="source-dialog-error" role="alert">
              {t(
                `有 ${scan.deepSkipped} 个子文件夹超过 32 层深度上限，本次未扫描；可单独选择它们继续。`,
                `${scan.deepSkipped} subfolders are deeper than the 32-level cap and were not scanned. Select them separately to continue.`,
              )}
            </p>
          )}
          {scan.notHashed > 0 && (
            <p className="muted">
              {t(
                `其中 ${scan.notHashed} 个文件较大，扫描时未比对重复，导入时会再判断。`,
                `${scan.notHashed} larger files were not compared during the scan; import decides then.`,
              )}
            </p>
          )}
          {scan.duplicates.length > 0 && (
            <>
              <h4>{t('与库内内容相同', 'Same content as a document already here')}</h4>
              <ul>
                {scan.duplicates.map((entry) => (
                  <li key={entry.relativePath}>
                    <strong title={entry.relativePath}>{entry.relativePath}</strong>
                    <span className="source-import-status">
                      {entry.existingTitle
                        ? t(
                            `已指向「${entry.existingTitle}」`,
                            `Points at “${entry.existingTitle}”`,
                          )
                        : t('已存在相同内容', 'Identical content already exists')}
                    </span>
                    {entry.existingTrashed && (
                      <small className="source-import-error">
                        {t('已有资料在回收站', 'The existing document is in the trash')}
                      </small>
                    )}
                  </li>
                ))}
              </ul>
            </>
          )}
          {scan.skipped.length > 0 && (
            <>
              <h4>{t('未导入的文件', 'Files not imported')}</h4>
              <ul>
                {scan.skipped.map((entry) => (
                  <li key={entry.relativePath}>
                    <strong title={entry.relativePath}>{entry.relativePath}</strong>
                    <span className="source-import-status">{scanSkipReason(entry.status, t)}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
          <div className="source-import-controls">
            <button
              type="button"
              disabled={!importing && !items.some((item) => item.state === 'waiting')}
              onClick={onCancelRemaining}
            >
              {t('取消剩余', 'Cancel remaining')}
            </button>
            <button type="button" onClick={onDismissScan}>
              {t('收起扫描结果', 'Hide scan results')}
            </button>
          </div>
        </section>
      )}{' '}
      {items.length > 0 && (
        <section>
          <h3>{t('本次导入', 'This import')}</h3>
          <div className="source-import-progress-row">
            <span
              className="source-import-progress-track"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={progress.percent}
              aria-label={t('本次导入进度', 'Import progress')}
            >
              <span style={{ width: `${progress.percent}%` }} />
            </span>
            <p role="status">
              <strong>{sourceBatchProgressLabel(progress, t)}</strong>
              {progressParts.length > 0 && <small>{progressParts.join(' · ')}</small>}
            </p>
          </div>
          {batchFailures.length > 0 && (
            <p className="source-import-failure-summary" role="status">
              {importFailureSummary(batchFailures, t)}
            </p>
          )}
          <ul>
            {items.map((item) => {
              const job = item.jobId ? jobsById.get(item.jobId) : jobsByToken.get(item.token);
              return (
                <li key={item.token}>
                  <strong title={item.name}>{item.name}</strong>
                  {job ? (
                    jobDetails(job)
                  ) : (
                    <span className="source-import-status" data-status={item.state}>
                      {item.state === 'accepted'
                        ? t('已加入队列', 'Added to queue')
                        : item.state === 'waiting'
                          ? t('等待提交', 'Waiting to submit')
                          : item.state === 'failed'
                            ? t('未导入', 'Not imported')
                            : t('等待确认', 'Awaiting confirmation')}
                    </span>
                  )}
                  {item.error && <small className="source-import-error">{item.error}</small>}
                </li>
              );
            })}
          </ul>
          {items.some((item) => item.state === 'waiting') && (
            <button type="button" disabled={importing || locked} onClick={onContinue}>
              {t('继续导入剩余文件', 'Continue remaining files')}
            </button>
          )}
          {onGoInbox && (
            <button type="button" onClick={onGoInbox}>
              {t('去收件箱继续整理', 'Continue organizing in the inbox')}
            </button>
          )}
          {onOrganizeImported && importedSourceIds.length > 0 && (
            <button
              type="button"
              disabled={importing || locked}
              onClick={() => onOrganizeImported(importedSourceIds)}
            >
              {t('整理本次导入', 'Organize this import')}
            </button>
          )}
        </section>
      )}
      {recent.length > 0 && (
        <section>
          <h3>{t('最近导入', 'Recent imports')}</h3>
          {recentFailures.length > 0 && (
            <p className="source-import-failure-summary" role="status">
              {importFailureSummary(recentFailures, t)}
            </p>
          )}
          {recentFailures.map((group) => (
            <div className="source-import-failure-group" key={group.group}>
              <h4>
                {sourceImportFailureGroupLabel(group.group, t)}
                {' · '}
                {group.count}
              </h4>
              <ul>{group.records.map(jobRow)}</ul>
            </div>
          ))}
          {recentRest.length > 0 && <ul>{recentRest.map(jobRow)}</ul>}
        </section>
      )}
      {!loaded && !error && (
        <p role="status">{t('正在读取导入记录…', 'Loading import history…')}</p>
      )}
      {loaded && !items.length && !jobs.length && (
        <p className="source-activity-empty">{t('还没有导入记录。', 'No imports yet.')}</p>
      )}
      {error && (
        <p className="source-dialog-error" role="alert">
          {error}
          <button type="button" onClick={onRetry}>
            {t('重试', 'Retry')}
          </button>
        </p>
      )}
      {(onOpenTasks || onAcknowledge) && (
        <footer className="source-library-dialog-footer">
          {onOpenTasks && (
            <button type="button" onClick={onOpenTasks}>
              {t('查看全部任务与详情', 'View all activity and details')}
            </button>
          )}
          {onAcknowledge && (
            <button type="button" onClick={onAcknowledge}>
              {t('已查看结果', 'Mark results as seen')}
            </button>
          )}
        </footer>
      )}
    </div>
  );
}
