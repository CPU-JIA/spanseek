import { invoke, isTauri, convertFileSrc } from '@tauri-apps/api/core';
import type {
  ApiError,
  DesktopCommand,
  DesktopData,
  DesktopReply,
  GrantPurpose,
  ResourceGrant,
  WorkspaceIdentity,
  LocalDocuments,
} from '@pkms/contracts';
import { activeLanguage } from './language.ts';
import { apiErrorMessage, errorText } from './errors.ts';
import { announceTagChange, TAG_LOCAL_CHANGED_EVENT } from './portability/tag-sync-events';

export class ServiceError extends Error {
  readonly details: ApiError;
  constructor(details: ApiError) {
    super(apiErrorMessage(details.code, activeLanguage()));
    this.name = 'ServiceError';
    this.details = details;
    // The kernel sentence is Chinese only, so the message is resolved when it is
    // read rather than when the failure happened: switching language re-labels
    // an error that is still on screen.
    Object.defineProperty(this, 'message', {
      get: () => apiErrorMessage(details.code, activeLanguage()),
      enumerable: true,
      configurable: true,
    });
  }
}
export function message(error: unknown): string {
  return errorText(error);
}
export async function call<K extends DesktopData['kind']>(
  command: DesktopCommand,
  expected: K,
  identity: WorkspaceIdentity | null = null,
  timeoutMs = 0,
): Promise<Extract<DesktopData, { kind: K }> extends { payload: infer P } ? P : undefined> {
  if (!isTauri()) throw new Error('请通过 spanseek 桌面应用打开。浏览器预览无法访问本地知识库。');
  const requestId = crypto.randomUUID();
  // Dev-server-only fault seam for reproducible UI error-state samples. It is
  // inert unless a test sets the global list, and production builds dead-code-eliminate it.
  // Packaged real-Tauri runs cannot monkey-patch `__TAURI_INTERNALS__.invoke`
  // (the property is non-writable); use the debug-only Rust seam
  // `PKMS_TEST_FAULT_CONFIG` in `apps/desktop/src-tauri/src/commands.rs`.
  if (import.meta.env.DEV) {
    const faults = (globalThis as { __PKMS_FAULT_KINDS__?: unknown }).__PKMS_FAULT_KINDS__;
    if (Array.isArray(faults) && faults.includes(command.kind)) {
      throw new ServiceError({
        code: 'storage_busy',
        message: 'injected fault',
        request_id: requestId,
        retryable: true,
        details: null,
      });
    }
  }
  let response: DesktopReply;
  try {
    const request = invoke<DesktopReply>('desktop_request', {
      request: { api_version: 1, request_id: requestId, identity, command },
    });
    response =
      timeoutMs > 0
        ? await Promise.race([
            request,
            new Promise<never>((_, reject) => {
              const timer = window.setTimeout(
                () => reject(new Error('本地服务响应超时，请重试。')),
                timeoutMs,
              );
              request.finally(() => window.clearTimeout(timer)).catch(() => undefined);
            }),
          ])
        : await request;
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && 'message' in error)
      throw new ServiceError(error as ApiError);
    throw new Error('无法连接本地服务，请重试。');
  }
  if (response.request_id !== requestId || response.data.kind !== expected)
    throw new Error('服务响应不匹配，已停止更新界面。');
  if (
    identity &&
    ['metadata_commit', 'tag_management_commit', 'note_commit', 'collection_commit'].includes(
      response.data.kind,
    )
  )
    announceTagChange(TAG_LOCAL_CHANGED_EVENT, identity);
  return ('payload' in response.data ? response.data.payload : undefined) as never;
}
export async function choose(
  purpose: GrantPurpose,
  identity: WorkspaceIdentity | null,
): Promise<ResourceGrant | null> {
  try {
    return await invoke('choose_resource', { purpose, identity });
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error)
      throw new ServiceError(error as ApiError);
    throw new Error('无法打开文件选择器，请重试。');
  }
}
export function imageUrl(
  identity: WorkspaceIdentity,
  resource: { revision: string; hash: string; thumbnail?: boolean } | { token: string },
): string {
  const path = [
    identity.local_instance_id,
    identity.workspace_id,
    ...('token' in resource
      ? ['preview', resource.token]
      : [resource.thumbnail ? 'thumbnail' : 'revision', resource.revision, resource.hash]),
  ].join('/');
  // convertFileSrc encodes '/' inside a filesystem path. This protocol uses
  // validated identity segments, so only derive its platform-specific origin.
  return convertFileSrc('', 'pkms-image') + path.split('/').map(encodeURIComponent).join('/');
}

export async function openExternalUrl(href: string): Promise<void> {
  if (!isTauri()) throw new Error('请通过 spanseek 桌面应用打开外部链接。');
  try {
    await invoke('open_external_url', { href });
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && 'message' in error)
      throw new ServiceError(error as ApiError);
    throw new Error('无法打开链接，请检查地址和系统默认浏览器后重试。');
  }
}

/** The native adapter resolves a completed job's original authorized directory. */
export async function revealOutputJob(identity: WorkspaceIdentity, jobId: string): Promise<void> {
  if (!isTauri()) throw new Error('请通过 spanseek 桌面应用查看导出文件。');
  try {
    await invoke('reveal_output_job', { identity, jobId });
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && 'message' in error)
      throw new ServiceError(error as ApiError);
    throw new Error('无法打开导出位置；文件可能已移动，请重新导出后再试。');
  }
}

/**
 * Asks the kernel to run application startup again.
 *
 * A startup failure is remembered on the native handle, so the retry buttons on
 * the welcome screen must clear it and start over; re-reading state alone would
 * only return the same failure.
 */
export async function retryStartup(): Promise<void> {
  if (!isTauri()) return;
  await invoke('retry_startup');
}
async function nativeDocumentCall<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('请通过 spanseek 桌面应用打开本地资料。');
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && 'message' in error)
      throw new ServiceError(error as ApiError);
    throw new Error('无法打开资料，请重试。');
  }
}

export function chooseOpenDocuments(pdfOnly: boolean): Promise<LocalDocuments> {
  return nativeDocumentCall('choose_open_documents', { pdfOnly });
}
export function takeOpenDocuments(): Promise<LocalDocuments> {
  return nativeDocumentCall('take_open_documents');
}
export function releaseOpenDocument(token: string): Promise<void> {
  return nativeDocumentCall('release_open_document', { token });
}
export function grantOpenDocuments(
  identity: WorkspaceIdentity,
  tokens: string[],
): Promise<ResourceGrant[]> {
  return nativeDocumentCall('grant_open_documents', { identity, tokens });
}
