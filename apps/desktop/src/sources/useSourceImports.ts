import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ResourceGrant, SourceJob, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { latestSourceJob } from '../portability/source-task-state';
import { getSourceActions, useSourceActions } from './useSourceActions';
import {
  prepareScannedEntries,
  prepareSourceFiles,
  splitScannedEntries,
  submitSourceFiles,
  type ScannedSkip,
  type SourceImportSelection,
  type SourceFileImport,
  type SourceScanReport,
} from './source-import-batch';

export type { SourceImportSelection, SourceScanReport } from './source-import-batch';

export type SourceImportRequest = { id: string; grants: ResourceGrant[] };

const isActive = (job: SourceJob) => job.status === 'queued' || job.status === 'running';

export function useSourceImports({
  identity,
  importRequest,
  onImportRequestHandled,
  onOpenSource,
  onOpenImportedSource,
  onChanged,
}: {
  identity: WorkspaceIdentity;
  importRequest?: SourceImportRequest | null;
  onImportRequestHandled?: (id: string) => void;
  onOpenSource: (sourceId: string, versionId?: string) => void;
  onOpenImportedSource?: (
    sourceId: string,
    versionId: string | undefined,
    inputToken: string,
  ) => void;
  onChanged: () => void;
}) {
  const { t } = useI18n();
  const actions = useSourceActions(identity, 'sources');
  const [items, setItems] = useState<SourceFileImport[]>([]);
  const [jobs, setJobs] = useState<SourceJob[]>([]);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [jobsError, setJobsError] = useState<string | null>(null);
  const [jobsLoaded, setJobsLoaded] = useState(false);
  const [pollRetry, setPollRetry] = useState(0);
  const [scan, setScan] = useState<SourceScanReport | null>(null);
  const cancelled = useRef(false);
  const itemsRef = useRef(items);
  const jobsRef = useRef(jobs);
  const busyRef = useRef(false);
  const alive = useRef(true);
  const autoOpen = useRef<{ token?: string; jobId?: string } | null>(null);
  const handled = useRef<string | null>(null);
  const callbacks = useRef({
    onChanged,
    onOpenSource,
    onOpenImportedSource,
    onImportRequestHandled,
  });
  callbacks.current = { onChanged, onOpenSource, onOpenImportedSource, onImportRequestHandled };
  itemsRef.current = items;
  jobsRef.current = jobs;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  useEffect(() => {
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const recent = await call(
          { kind: 'source_jobs_list', payload: { limit: 20 } },
          'source_jobs',
          identity,
        );
        if (!live) return;
        const recentIds = new Set(recent.map((job) => job.id));
        const known = new Map(jobsRef.current.map((job) => [job.id, job]));
        const trackedIds = new Set(
          itemsRef.current.flatMap((item) => (item.jobId ? [item.jobId] : [])),
        );
        if (autoOpen.current?.jobId) trackedIds.add(autoOpen.current.jobId);
        const missing = [...trackedIds].filter(
          (id) => !recentIds.has(id) && (!known.has(id) || isActive(known.get(id)!)),
        );
        const updates = [...recent];
        let readError: string | null = null;
        // A batch can exceed the recent-task window. Follow its actual receipts,
        // with bounded concurrency, instead of inferring success from that window.
        for (let offset = 0; live && offset < missing.length; offset += 6) {
          const results = await Promise.allSettled(
            missing
              .slice(offset, offset + 6)
              .map((job_id) =>
                call({ kind: 'source_job_get', payload: { job_id } }, 'source_job', identity),
              ),
          );
          if (!live) return;
          for (const result of results) {
            if (result.status === 'fulfilled') updates.push(result.value);
            else readError = message(result.reason);
          }
        }
        if (!live) return;
        const previous = new Map(jobsRef.current.map((job) => [job.id, job]));
        for (const job of updates) known.set(job.id, latestSourceJob(known.get(job.id), job));
        const next = [...known.values()]
          .filter((job) => recentIds.has(job.id) || trackedIds.has(job.id))
          .sort((left, right) => right.updated_at.localeCompare(left.updated_at));
        if (
          updates.some((job) => {
            const old = previous.get(job.id);
            return old && (old.stage !== job.stage || old.status !== job.status);
          })
        )
          callbacks.current.onChanged();
        jobsRef.current = next;
        setJobs(next);
        setJobsError(readError);
        setJobsLoaded(true);
        const target = autoOpen.current;
        const ready =
          target &&
          next.find(
            (job) =>
              (target.jobId ? job.id === target.jobId : job.input.input_token === target.token) &&
              job.status === 'succeeded' &&
              job.source_id &&
              job.source_version_id,
          );
        if (ready) {
          autoOpen.current = null;
          if (callbacks.current.onOpenImportedSource)
            callbacks.current.onOpenImportedSource(
              ready.source_id!,
              ready.source_version_id!,
              ready.input.input_token,
            );
          else callbacks.current.onOpenSource(ready.source_id!, ready.source_version_id!);
        }
      } catch (caught) {
        if (live) setJobsError(message(caught));
      }
      if (live) timer = setTimeout(() => void poll(), 2200);
    }
    void poll();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [identity.local_instance_id, identity.workspace_id, actions.generation, pollRetry]);

  function updateItem(
    token: string,
    state: SourceFileImport['state'],
    itemError?: string,
    jobId?: string,
  ) {
    setItems((old) =>
      old.map((item) =>
        item.token === token
          ? { ...item, state, error: itemError, jobId: jobId ?? item.jobId }
          : item,
      ),
    );
  }
  async function duringImport(task: () => Promise<boolean>) {
    if (busyRef.current || getSourceActions(identity, 'sources').getSnapshot().locked) return false;
    busyRef.current = true;
    setImporting(true);
    setError(null);
    try {
      return await task();
    } catch (caught) {
      if (alive.current) setError(message(caught));
      return false;
    } finally {
      busyRef.current = false;
      if (alive.current) setImporting(false);
    }
  }
  async function acceptGrants(grants: ResourceGrant[], selection: SourceImportSelection) {
    if (!grants.length) return false;
    cancelled.current = false;
    const files = prepareSourceFiles(grants, selection.collectionId, selection.tagNames);
    autoOpen.current =
      files.length === 1 && files[0].state === 'waiting' ? { token: files[0].token } : null;
    if (alive.current) {
      setScan(null);
      setItems(files);
    }
    await submitSourceFiles(files, getSourceActions(identity, 'sources'), updateItem);
    if (alive.current) setPollRetry((value) => value + 1);
    return true;
  }
  function importFiles(selection: SourceImportSelection) {
    return duringImport(async () => {
      const grants = await invoke<ResourceGrant[]>('choose_source_files', { identity });
      return acceptGrants(grants, selection);
    });
  }
  function importGrants(
    grants: ResourceGrant[],
    selection: SourceImportSelection = { collectionId: null, tagNames: [] },
  ) {
    return duringImport(() => acceptGrants(grants, selection));
  }
  function importWebpage(address: string, selection: SourceImportSelection) {
    return duringImport(async () => {
      let url: URL;
      try {
        url = new URL(address.trim());
      } catch {
        throw new Error(
          t('请输入完整的 HTTP 或 HTTPS 网页地址。', 'Enter a complete HTTP or HTTPS webpage URL.'),
        );
      }
      if (!['http:', 'https:'].includes(url.protocol))
        throw new Error(
          t('请输入 HTTP 或 HTTPS 网页地址。', 'Enter an HTTP or HTTPS webpage URL.'),
        );
      const commandId = crypto.randomUUID();
      if (alive.current) {
        setScan(null);
        setItems([]);
      }
      const accepted = await actions.execute(
        {
          kind: 'sources_import_webpage',
          payload: {
            command_id: commandId,
            request: {
              url: url.href,
              collection_id: selection.collectionId,
              add_version_to: null,
              base_row_version: null,
            },
            tag_names: selection.tagNames,
          },
        },
        'job_accepted',
      );
      const lastJob = getSourceActions(identity, 'sources').getSnapshot().lastJob;
      if (lastJob?.commandId === commandId) autoOpen.current = { jobId: lastJob.jobId };
      return accepted;
    });
  }
  /**
   * Folder import: one granted root, one scan, then the same per-item pipeline
   * the file dialog uses. Entries the scan skipped are reported, not dropped.
   */
  function importDirectory(selection: SourceImportSelection) {
    return duringImport(async () => {
      const grant = await invoke<ResourceGrant | null>('choose_source_directory', { identity });
      if (!grant) return false;
      if (alive.current) {
        setItems([]);
        setScan(null);
      }
      const result = await call(
        { kind: 'source_directory_scan', payload: { token: grant.token } },
        'source_directory_scan',
        identity,
      );
      const { ready, duplicates, skipped, importable } = splitScannedEntries(result.entries);
      cancelled.current = false;
      if (alive.current)
        setScan({
          rootName: result.root_name,
          visited: result.visited,
          truncated: result.truncated,
          limited: result.limited,
          deepSkipped: result.deep_skipped,
          ready: ready.length,
          duplicates: duplicates.map((entry) => ({
            relativePath: entry.relative_path,
            existingTitle: entry.existing_title,
            existingTrashed: entry.existing_trashed,
          })),
          notHashed: ready.filter((entry) => entry.note === 'not_hashed').length,
          skipped,
        });
      const files = prepareScannedEntries(importable, selection.collectionId, selection.tagNames);
      if (!files.length) return true;
      autoOpen.current = files.length === 1 ? { token: files[0].token } : null;
      if (alive.current) setItems(files);
      await submitSourceFiles(
        files,
        getSourceActions(identity, 'sources'),
        updateItem,
        () => cancelled.current,
      );
      if (alive.current) setPollRetry((value) => value + 1);
      return true;
    });
  }

  /**
   * Stops the running batch: unsubmitted items stay pending so they can be
   * continued, and jobs already queued are cancelled through the same command
   * the task centre uses.
   */
  async function cancelRemaining() {
    cancelled.current = true;
    for (const item of itemsRef.current) {
      if (item.state !== 'accepted' || !item.jobId) continue;
      const job = jobsRef.current.find((value) => value.id === item.jobId);
      if (job && job.status !== 'queued' && job.status !== 'running') continue;
      try {
        const updated = await call(
          { kind: 'source_job_cancel', payload: { job_id: item.jobId } },
          'source_job',
          identity,
        );
        if (alive.current)
          setJobs((old) =>
            old.map((value) => (value.id === updated.id ? latestSourceJob(value, updated) : value)),
          );
      } catch {
        // A job that already finished cannot be cancelled; polling shows its state.
      }
    }
    if (alive.current) setPollRetry((value) => value + 1);
  }
  async function retryJob(job: SourceJob) {
    const controller = getSourceActions(identity, 'sources');
    if (busyRef.current || controller.getSnapshot().locked) return;
    busyRef.current = true;
    setImporting(true);
    setError(null);
    const commandId = crypto.randomUUID();
    const item = itemsRef.current.find(
      (value) => value.jobId === job.id || value.token === job.input.input_token,
    );
    try {
      const accepted = await controller.execute(
        {
          kind: 'source_job_retry',
          payload: { command_id: commandId, job_id: job.id },
        },
        'job_accepted',
      );
      const snapshot = controller.getSnapshot();
      if (item && accepted && snapshot.lastJob?.commandId === commandId) {
        updateItem(item.token, 'accepted', undefined, snapshot.lastJob.jobId);
      } else if (item && snapshot.pending) {
        updateItem(item.token, 'unconfirmed', snapshot.error ?? undefined);
      } else if (item && snapshot.error) {
        updateItem(item.token, 'failed', snapshot.error);
      }
      if (alive.current) setPollRetry((value) => value + 1);
    } finally {
      busyRef.current = false;
      if (alive.current) setImporting(false);
    }
  }
  async function retry() {
    const controller = getSourceActions(identity, 'sources');
    if (busyRef.current || controller.getSnapshot().busy) return;
    busyRef.current = true;
    setImporting(true);
    const pending = controller.getSnapshot().pending;
    try {
      await controller.retry();
      const snapshot = controller.getSnapshot();
      if (pending?.command.kind === 'source_job_retry') {
        const attemptedJobId = pending.command.payload.job_id;
        const commandId = pending.command.payload.command_id;
        const item = itemsRef.current.find((value) => value.jobId === attemptedJobId);
        if (item)
          updateItem(
            item.token,
            snapshot.pending ? 'unconfirmed' : snapshot.error ? 'failed' : 'accepted',
            snapshot.error ?? undefined,
            snapshot.lastJob?.commandId === commandId ? snapshot.lastJob.jobId : undefined,
          );
      } else if (pending?.command.kind === 'sources_import') {
        updateItem(
          pending.command.payload.request.input_token,
          snapshot.pending ? 'unconfirmed' : snapshot.error ? 'failed' : 'accepted',
          snapshot.error ?? undefined,
          snapshot.lastJob?.commandId === pending.command.payload.command_id
            ? snapshot.lastJob.jobId
            : undefined,
        );
      } else if (
        pending?.command.kind === 'sources_import_webpage' &&
        snapshot.lastJob?.commandId === pending.command.payload.command_id
      ) {
        autoOpen.current = { jobId: snapshot.lastJob.jobId };
      }
      if (alive.current) setPollRetry((value) => value + 1);
    } finally {
      busyRef.current = false;
      if (alive.current) setImporting(false);
    }
  }
  function continueRemaining() {
    return duringImport(async () => {
      await submitSourceFiles(itemsRef.current, getSourceActions(identity, 'sources'), updateItem);
      return true;
    });
  }
  useEffect(() => {
    if (!importRequest || handled.current === importRequest.id || actions.locked || importing)
      return;
    handled.current = importRequest.id;
    callbacks.current.onImportRequestHandled?.(importRequest.id);
    void importGrants(importRequest.grants);
  }, [importRequest?.id, actions.locked, importing]);

  return {
    actions,
    items,
    jobs,
    importing,
    error,
    jobsError,
    jobsLoaded,
    scan,
    importFiles,
    importDirectory,
    importWebpage,
    cancelRemaining,
    clearScan: () => setScan(null),
    retry,
    retryJob,
    continueRemaining,
    retryJobs: () => setPollRetry((value) => value + 1),
    clearError: () => setError(null),
  };
}
