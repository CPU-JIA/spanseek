import { invoke } from '@tauri-apps/api/core';
import type { SourceResourceTicket, WorkspaceIdentity } from '@pkms/contracts';

/** The native resource endpoint accepts at most 16 MiB in one response. */
export const SOURCE_RESOURCE_CHUNK_BYTES = 8 * 1024 * 1024;
/** Keep one malformed package from exhausting the WebView heap. */
export const MAX_DOCX_PREVIEW_BYTES = 64 * 1024 * 1024;

export type ByteRange = { start: number; end: number };

/** Deterministic, non-overlapping range plan; the last range is inclusive. */
export function sourceByteRanges(size: number, chunk = SOURCE_RESOURCE_CHUNK_BYTES): ByteRange[] {
  if (!Number.isSafeInteger(size) || size < 0) throw new Error('source_resource_size');
  if (!Number.isSafeInteger(chunk) || chunk <= 0) throw new Error('source_resource_chunk');
  const ranges: ByteRange[] = [];
  for (let start = 0; start < size; start += chunk) {
    const end = Math.min(size - 1, start + chunk - 1);
    ranges.push({ start, end });
  }
  return ranges;
}

export function isSafeDocxHref(value: string): boolean {
  const href = value.trim();
  if (!href) return false;
  if (href.startsWith('#')) return true;
  try {
    const url = new URL(href);
    return url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:';
  } catch {
    return false;
  }
}

function isSafeEmbeddedSource(value: string): boolean {
  const source = value.trim();
  return (
    source.startsWith('blob:') ||
    /^data:image\/(?:png|jpe?g|gif|webp|bmp);base64,/i.test(source) ||
    source.startsWith('http://pkms-image.localhost/') ||
    source.startsWith('pkms-image:')
  );
}

/**
 * `docx-preview` turns package parts into DOM rather than inserting arbitrary
 * HTML. Defensive cleanup still removes executable elements, inline handlers,
 * unsafe links and non-image resources, because the DOCX is user-controlled.
 */
export function sanitizeDocxPreview(root: ParentNode): void {
  for (const element of root.querySelectorAll(
    'script,iframe,frame,object,embed,link,meta,form,base',
  )) {
    element.remove();
  }
  for (const element of root.querySelectorAll('*')) {
    for (const attribute of [...element.attributes]) {
      const name = attribute.name.toLowerCase();
      if (name.startsWith('on')) {
        element.removeAttribute(attribute.name);
        continue;
      }
      if (name === 'href' && !isSafeDocxHref(attribute.value)) {
        element.removeAttribute(attribute.name);
        continue;
      }
      if ((name === 'src' || name === 'poster') && !isSafeEmbeddedSource(attribute.value)) {
        element.removeAttribute(attribute.name);
        continue;
      }
      if (name === 'style' && /(?:javascript\s*:|expression\s*\(|@import)/i.test(attribute.value)) {
        element.removeAttribute(attribute.name);
      }
    }
    if (element.localName === 'a') {
      element.setAttribute('rel', 'noreferrer noopener');
    }
  }
}

/** Blob URLs created by docx-preview are not garbage-collected until revoked. */
export function docxPreviewObjectUrls(roots: readonly ParentNode[]): string[] {
  const urls = new Set<string>();
  for (const root of roots) {
    for (const element of root.querySelectorAll('[src]')) {
      const source = element.getAttribute('src') ?? '';
      if (source.startsWith('blob:')) urls.add(source);
    }
    for (const style of root.querySelectorAll('style')) {
      const text = style.textContent ?? '';
      for (const match of text.matchAll(/url\((['"]?)(blob:[^)'"\s]+)\1\)/g)) urls.add(match[2]);
    }
  }
  return [...urls];
}

/**
 * Reads an immutable original through the same short-lived, version-scoped
 * resource ticket PDF already uses. The resource endpoint caps a single
 * response at 16 MiB, so large DOCX files are assembled from bounded ranges.
 */
export async function readSourceResourceBytes(
  identity: WorkspaceIdentity,
  sourceVersionId: string,
  maximum = MAX_DOCX_PREVIEW_BYTES,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  const ticket = await invoke<SourceResourceTicket>('issue_source_resource', {
    identity,
    sourceVersionId,
  });
  try {
    if (!Number.isSafeInteger(ticket.size) || ticket.size <= 0)
      throw new Error('source_resource_size');
    if (ticket.size > maximum) throw new Error('source_resource_too_large');
    const bytes = new Uint8Array(ticket.size);
    for (const { start, end } of sourceByteRanges(ticket.size)) {
      const response = await fetch(ticket.url, {
        headers: { Range: `bytes=${start}-${end}` },
        signal,
      });
      if (response.status !== 206) throw new Error('source_resource_range');
      if (response.headers.get('Content-Range') !== `bytes ${start}-${end}/${ticket.size}`)
        throw new Error('source_resource_range');
      const chunk = new Uint8Array(await response.arrayBuffer());
      if (chunk.length !== end - start + 1) throw new Error('source_resource_range');
      bytes.set(chunk, start);
    }
    return bytes;
  } finally {
    await invoke('release_source_resource', { resourceUrl: ticket.url }).catch(() => {});
  }
}
