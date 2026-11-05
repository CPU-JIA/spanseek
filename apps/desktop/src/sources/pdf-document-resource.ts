import { invoke } from '@tauri-apps/api/core';
import { getDocument, PDFDataRangeTransport } from 'pdfjs-dist';
import type { SourceResourceTicket, WorkspaceIdentity } from '@pkms/contracts';
import { PdfOriginalReadError } from './pdf-reader-errors';

export type PdfReaderInput = { url: string } | { data: Uint8Array };

/** The native source path keeps its scoped range grant; temporary input uses the same renderer. */
export function openPdfResource(
  versionId: string,
  identity: WorkspaceIdentity | undefined,
  input: PdfReaderInput | undefined,
  onFailure: (error: unknown) => void,
) {
  const abort = new AbortController();
  let disposed = false;
  let loading: ReturnType<typeof getDocument> | undefined;
  let resourceUrl: string | undefined;
  let failureReported = false;
  const report = (error: unknown) => {
    if (!disposed && !failureReported) {
      failureReported = true;
      onFailure(error);
    }
  };
  const options = {
    rangeChunkSize: 256 * 1024,
    disableAutoFetch: true,
    disableStream: true,
    cMapUrl: '/pdfjs/cmaps/',
    cMapPacked: true,
    standardFontDataUrl: '/pdfjs/standard_fonts/',
    wasmUrl: '/pdfjs/wasm/',
    enableXfa: false,
  };
  const promise = (async () => {
    if (input && 'data' in input) {
      // PDF.js transfers data to its worker. Keep the caller's reusable bytes intact.
      loading = getDocument({ ...options, data: input.data.slice() });
    } else {
      let ticket: SourceResourceTicket;
      if (input) {
        // The native endpoint intentionally refuses an unbounded GET above 16 MiB.
        // PDF.js's URL loader probes with GET first, so temporary files also use
        // an explicit bounded transport, starting with a metadata-only HEAD.
        const response = await fetch(input.url, { method: 'HEAD', signal: abort.signal });
        const size = Number(response.headers.get('Content-Length'));
        if (!response.ok || !Number.isSafeInteger(size) || size <= 0)
          throw new PdfOriginalReadError('pdf_resource_unavailable');
        ticket = {
          url: input.url,
          size,
          media_type: 'application/pdf',
          expires_at: '9999-12-31T23:59:59.999Z',
        };
      } else {
        if (!identity) throw new PdfOriginalReadError('pdf_resource_unavailable');
        ticket = await invoke<SourceResourceTicket>('issue_source_resource', {
          identity,
          sourceVersionId: versionId,
        });
        resourceUrl = ticket.url;
      }
      if (disposed) {
        if (resourceUrl) await invoke('release_source_resource', { resourceUrl }).catch(() => {});
        throw new DOMException('Reader closed', 'AbortError');
      }
      let renewing: Promise<void> | null = null;
      class LocalRanges extends PDFDataRangeTransport {
        override requestDataRange(begin: number, end: number) {
          void (async () => {
            if (!input && Date.parse(ticket.expires_at) <= Date.now() + 5000) {
              renewing ??= (async () => {
                const renewed = await invoke<SourceResourceTicket>('issue_source_resource', {
                  identity,
                  sourceVersionId: versionId,
                });
                if (disposed) {
                  await invoke('release_source_resource', { resourceUrl: renewed.url }).catch(
                    () => {},
                  );
                  throw new DOMException('Reader closed', 'AbortError');
                }
                const oldUrl = ticket.url;
                ticket = renewed;
                resourceUrl = renewed.url;
                await invoke('release_source_resource', { resourceUrl: oldUrl }).catch(() => {});
              })().finally(() => {
                renewing = null;
              });
              await renewing;
            }
            const activeTicket = ticket;
            const output = new Uint8Array(end - begin);
            for (let offset = begin; offset < end; offset += 4 * 1024 * 1024) {
              const last = Math.min(end, offset + 4 * 1024 * 1024);
              const response = await fetch(activeTicket.url, {
                headers: { Range: `bytes=${offset}-${last - 1}` },
                signal: abort.signal,
              });
              if (response.status === 403 || response.status === 404)
                throw new PdfOriginalReadError('pdf_resource_unavailable');
              if (
                response.status !== 206 ||
                response.headers.get('Content-Range') !==
                  `bytes ${offset}-${last - 1}/${activeTicket.size}`
              )
                throw new PdfOriginalReadError('pdf_range_mismatch');
              const bytes = new Uint8Array(await response.arrayBuffer());
              if (bytes.length !== last - offset)
                throw new PdfOriginalReadError('pdf_range_incomplete');
              output.set(bytes, offset - begin);
            }
            if (!disposed) this.onDataRange(begin, output);
          })().catch((error) => {
            report(error);
            void loading?.destroy();
          });
        }
        override abort() {
          abort.abort();
        }
      }
      loading = getDocument({ ...options, range: new LocalRanges(ticket.size, null, true) });
    }
    return await loading.promise;
  })();
  return {
    promise,
    dispose() {
      disposed = true;
      abort.abort();
      void loading?.destroy();
      if (resourceUrl) void invoke('release_source_resource', { resourceUrl }).catch(() => {});
    },
  };
}
