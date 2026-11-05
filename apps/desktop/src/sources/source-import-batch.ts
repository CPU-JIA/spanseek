import type { DesktopCommand, DirectoryScanEntry, ResourceGrant, SourceJob } from '@pkms/contracts';
import type { SourceActions } from './source-actions';

type Translate = (zh: string, en: string) => string;

export type SourceImportSelection = {
  collectionId: string | null;
  tagNames: string[];
};

/** One place that maps an extension to the import format the core expects. */
export const SOURCE_FILE_FORMATS: Record<
  string,
  'pdf' | 'markdown' | 'text' | 'docx' | 'doc' | 'epub' | 'mobi' | 'csv' | 'xlsx' | 'xls' | 'pptx'
> = {
  pdf: 'pdf',
  md: 'markdown',
  markdown: 'markdown',
  txt: 'text',
  docx: 'docx',
  doc: 'doc',
  epub: 'epub',
  mobi: 'mobi',
  csv: 'csv',
  xlsx: 'xlsx',
  xls: 'xls',
  pptx: 'pptx',
};

export type SourceFileImport = {
  token: string;
  name: string;
  state: 'waiting' | 'accepted' | 'unconfirmed' | 'failed';
  error?: string;
  jobId?: string;
  command: Extract<DesktopCommand, { kind: 'sources_import' }>;
};
/** What one folder scan found; the import list and the activity panel share it. */
export type SourceScanReport = {
  rootName: string;
  visited: number;
  truncated: boolean;
  limited: number;
  /** Directories below the depth cap that the scan could not read. */
  deepSkipped: number;
  ready: number;
  duplicates: { relativePath: string; existingTitle: string | null; existingTrashed: boolean }[];
  /** Ready files too large to compare during the scan; import decides later. */
  notHashed: number;
  skipped: ScannedSkip[];
};

/** A scanned entry the user cannot import, kept so the result list is complete. */
export type ScannedSkip = {
  relativePath: string;
  name: string;
  status: DirectoryScanEntry['status'];
  note: string | null;
  existingTitle: string | null;
  existingTrashed: boolean;
};

/**
 * Splits one directory scan into what can be handed to `sources_import` and what
 * cannot. Duplicates stay importable: importing them points at the document that
 * already holds the same content, which the per-item result then shows.
 */
export function splitScannedEntries(entries: DirectoryScanEntry[]) {
  const ready: DirectoryScanEntry[] = [];
  const duplicates: DirectoryScanEntry[] = [];
  const skipped: ScannedSkip[] = [];
  for (const entry of entries) {
    if (!entry.grant || (entry.status !== 'ready' && entry.status !== 'duplicate')) {
      skipped.push({
        relativePath: entry.relative_path,
        name: entry.name,
        status: entry.status,
        note: entry.note,
        existingTitle: entry.existing_title,
        existingTrashed: entry.existing_trashed,
      });
      continue;
    }
    if (entry.status === 'duplicate') duplicates.push(entry);
    else ready.push(entry);
  }
  return { ready, duplicates, skipped, importable: [...ready, ...duplicates] };
}

/** The wording for a skipped entry; the core only reports machine codes. */
export function scanSkipReason(status: DirectoryScanEntry['status'], t: Translate): string {
  switch (status) {
    case 'unsupported':
      return t('不支持的类型', 'Unsupported type');
    case 'empty':
      return t('空文件', 'Empty file');
    case 'too_large':
      return t('超过 256 MiB', 'Larger than 256 MiB');
    case 'unreadable':
      return t('无法读取', 'Could not be read');
    case 'limit_reached':
      return t('超过单次受理上限', 'Past the per-scan limit');
    default:
      return status;
  }
}

/** Scanned entries already carry their own grant, so no new token is needed. */
export function prepareScannedEntries(
  entries: DirectoryScanEntry[],
  collectionId: string | null,
  tagNames: string[] = [],
): SourceFileImport[] {
  const files: SourceFileImport[] = [];
  for (const entry of entries) {
    const grant = entry.grant;
    if (!grant || !entry.format) continue;
    files.push({
      token: grant.token,
      name: entry.name,
      state: 'waiting',
      command: {
        kind: 'sources_import',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            input_token: grant.token,
            original_name: entry.name,
            format: entry.format,
            origin_url: null,
            collection_id: collectionId,
            add_version_to: null,
            base_row_version: null,
          },
          tag_names: tagNames,
        },
      },
    });
  }
  return files;
}

export function prepareSourceFiles(
  grants: ResourceGrant[],
  collectionId: string | null,
  tagNames: string[] = [],
): SourceFileImport[] {
  return grants.map((grant) => {
    const ext = grant.display_name.split('.').pop()?.toLowerCase() ?? '';
    const format = SOURCE_FILE_FORMATS[ext];
    return {
      token: grant.token,
      name: grant.display_name,
      state: format ? 'waiting' : 'failed',
      error: format
        ? undefined
        : '仅支持 PDF、Markdown、TXT、DOCX、DOC、EPUB、MOBI、CSV、XLSX、XLS 和 PPTX 文件 / Only PDF, Markdown, TXT, DOCX, DOC, EPUB, MOBI, CSV, XLSX, XLS and PPTX files are supported',
      command: {
        kind: 'sources_import',
        payload: {
          command_id: crypto.randomUUID(),
          request: {
            input_token: grant.token,
            original_name: grant.display_name,
            format: format ?? 'text',
            origin_url: null,
            collection_id: collectionId,
            add_version_to: null,
            base_row_version: null,
          },
          tag_names: tagNames,
        },
      },
    };
  });
}
export async function submitSourceFiles(
  files: SourceFileImport[],
  actions: Pick<SourceActions, 'execute' | 'getSnapshot'>,
  onResult: (
    token: string,
    state: SourceFileImport['state'],
    error?: string,
    jobId?: string,
  ) => void,
  shouldStop?: () => boolean,
) {
  for (const file of files) {
    if (file.state !== 'waiting') continue;
    // A cancelled batch keeps its unsubmitted items, so they can be continued
    // later instead of being reported as failures.
    if (shouldStop?.()) break;
    if (actions.getSnapshot().locked) break;
    const accepted = await actions.execute(file.command, 'job_accepted');
    const current = actions.getSnapshot();
    onResult(
      file.token,
      accepted ? 'accepted' : current.pending ? 'unconfirmed' : 'failed',
      accepted ? undefined : (current.error ?? undefined),
      current.lastJob?.commandId === file.command.payload.command_id
        ? current.lastJob.jobId
        : undefined,
    );
    if (current.pending || !current.ready) break;
  }
}
/** One batch item that will not import on its own, with the code explaining why. */
export type SourceImportFailure = {
  token: string;
  name: string;
  /** `source_jobs.error_code`; null when the request never reached the kernel. */
  code: string | null;
};

const isFailedJob = (job: SourceJob) => job.status === 'failed' || job.status === 'canceled';

/**
 * Jobs that ended without importing. A cancellation the user asked for and a
 * failure the user has to repair carry different wording, so the caller groups
 * them by cause instead of collapsing both into one "did not import" line.
 */
export function failedSourceJobs(jobs: readonly SourceJob[]): SourceJob[] {
  return jobs.filter((job) => isFailedJob(job));
}

/**
 * The batch items that will not move again without help: a failed job, a
 * cancellation, or a submission that never reached the kernel. Waiting,
 * unconfirmed and running items stay out, because the progress line reports those
 * as still moving.
 */
export function sourceImportFailures(
  items: readonly SourceFileImport[],
  jobs: readonly SourceJob[],
): SourceImportFailure[] {
  const jobsById = new Map(jobs.map((job) => [job.id, job]));
  const jobsByToken = new Map(jobs.map((job) => [job.input.input_token, job]));
  const failures: SourceImportFailure[] = [];
  for (const item of items) {
    const job = item.jobId ? jobsById.get(item.jobId) : jobsByToken.get(item.token);
    if (job) {
      if (isFailedJob(job))
        failures.push({ token: item.token, name: item.name, code: job.error_code });
    } else if (item.state === 'failed') {
      failures.push({ token: item.token, name: item.name, code: null });
    }
  }
  return failures;
}

/** How far one batch got; every batch item counts in exactly one bucket. */
export type SourceBatchProgress = {
  total: number;
  /** Items that reached a final state: imported, failed, cancelled or rejected. */
  finished: number;
  /** Succeeded jobs that resolved to an existing source instead of creating a new one. */
  duplicates: number;
  succeeded: number;
  failed: number;
  canceled: number;
  /** Accepted and still queued or running. */
  active: number;
  /** Never submitted, or submitted with a result that is still unconfirmed. */
  waiting: number;
  /** `finished / total` as a rounded 0–100; 0 for an empty batch. */
  percent: number;
};

/**
 * Counts one batch so the page and the activity panel can say
 * `已完成 137 / 500 份` instead of only "importing". An accepted item whose receipt
 * has not arrived yet stays in `active`, so the count never claims 100% before the
 * real job state does.
 */
export function sourceBatchProgress(
  items: readonly SourceFileImport[],
  jobs: readonly SourceJob[],
): SourceBatchProgress {
  const jobsById = new Map(jobs.map((job) => [job.id, job]));
  const jobsByToken = new Map(jobs.map((job) => [job.input.input_token, job]));
  const progress: SourceBatchProgress = {
    total: items.length,
    finished: 0,
    duplicates: 0,
    succeeded: 0,
    failed: 0,
    canceled: 0,
    active: 0,
    waiting: 0,
    percent: 0,
  };
  for (const item of items) {
    const job = item.jobId ? jobsById.get(item.jobId) : jobsByToken.get(item.token);
    if (!job) {
      // A rejected submission is finished; an unconfirmed one is still in flight.
      if (item.state === 'failed') {
        progress.failed += 1;
        progress.finished += 1;
      } else if (item.state === 'accepted') {
        progress.active += 1;
      } else {
        progress.waiting += 1;
      }
      continue;
    }
    if (job.status === 'succeeded') {
      if (
        job.stage === 'existing_source' ||
        job.stage === 'existing_historical_version' ||
        job.stage === 'existing_trashed_source'
      ) {
        progress.duplicates += 1;
      } else {
        progress.succeeded += 1;
      }
    } else if (job.status === 'failed') progress.failed += 1;
    else if (job.status === 'canceled') progress.canceled += 1;
    else {
      progress.active += 1;
      continue;
    }
    progress.finished += 1;
  }
  progress.percent = progress.total ? Math.round((progress.finished / progress.total) * 100) : 0;
  return progress;
}

/** The count line the sources page and the activity panel both show. */
export function sourceBatchProgressLabel(progress: SourceBatchProgress, t: Translate): string {
  if (!progress.total) return '';
  return t(
    `已完成 ${progress.finished} / ${progress.total} 份`,
    `${progress.finished} of ${progress.total} done`,
  );
}

/** The non-empty buckets of one batch, ready to join into a single muted line. */
export function sourceBatchProgressParts(progress: SourceBatchProgress, t: Translate): string[] {
  const parts: string[] = [];
  if (progress.succeeded)
    parts.push(t(`新增成功 ${progress.succeeded} 份`, `${progress.succeeded} new`));
  if (progress.duplicates)
    parts.push(
      t(`重复归并 ${progress.duplicates} 份`, `${progress.duplicates} already in library`),
    );
  if (progress.failed) parts.push(t(`失败 ${progress.failed} 份`, `${progress.failed} failed`));
  if (progress.canceled)
    parts.push(t(`已取消 ${progress.canceled} 份`, `${progress.canceled} cancelled`));
  if (progress.active)
    parts.push(t(`进行中 ${progress.active} 份`, `${progress.active} in progress`));
  if (progress.waiting) parts.push(t(`等待 ${progress.waiting} 份`, `${progress.waiting} waiting`));
  return parts;
}
