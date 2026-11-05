import type { AnswerRecord, NoteDocument, SearchEvidence } from '@pkms/contracts';
import type { MarkdownDocument } from './markdown-document.ts';

export type MarkdownRequest =
  | { kind: 'task-preview'; source: string; materials: readonly SearchEvidence[] }
  | {
      kind: 'task-proposal';
      source: string;
      base: NoteDocument;
      materials: readonly SearchEvidence[];
      format: 'rich' | 'source';
    }
  | { kind: 'parse'; source: string; allowFrontmatter: boolean }
  | {
      kind: 'chat-proposal';
      source: string;
      base: NoteDocument;
      format: 'rich' | 'source';
      allowFrontmatter: boolean;
    }
  | { kind: 'answer-proposal'; base: NoteDocument; answer: AnswerRecord };

type MarkdownWorker = Pick<
  Worker,
  'onmessage' | 'onerror' | 'onmessageerror' | 'postMessage' | 'terminate'
>;
type Job = {
  request: MarkdownRequest;
  signal?: AbortSignal;
  resolve: (value: MarkdownDocument) => void;
  reject: (reason: Error) => void;
  worker: MarkdownWorker | null;
  timer: ReturnType<typeof setTimeout> | null;
  abort: () => void;
  settled: boolean;
};

/** Bound historical-message parsing, and terminate an active parse when its owner leaves. */
export function createMarkdownQueue(makeWorker: () => MarkdownWorker, concurrency = 2) {
  const waiting: Job[] = [];
  const running = new Set<Job>();

  function finish(job: Job, value: MarkdownDocument | null, error?: Error) {
    if (job.settled) return;
    job.settled = true;
    job.signal?.removeEventListener('abort', job.abort);
    if (job.timer !== null) clearTimeout(job.timer);
    if (job.worker) {
      job.worker.onmessage = null;
      job.worker.onerror = null;
      job.worker.onmessageerror = null;
      job.worker.terminate();
    }
    running.delete(job);
    const index = waiting.indexOf(job);
    if (index >= 0) waiting.splice(index, 1);
    if (value) job.resolve(value);
    else job.reject(error ?? new Error('Markdown conversion failed.'));
    queueMicrotask(pump);
  }

  function pump() {
    while (running.size < concurrency && waiting.length) {
      const job = waiting.shift()!;
      if (job.settled) continue;
      running.add(job);
      try {
        const worker = makeWorker();
        job.worker = worker;
        worker.onmessage = (event: MessageEvent<{ value?: MarkdownDocument; error?: string }>) => {
          if (event.data.value) finish(job, event.data.value);
          else finish(job, null, new Error(event.data.error ?? 'Markdown conversion failed.'));
        };
        worker.onerror = (event) => {
          event.preventDefault();
          finish(job, null, new Error(event.message || 'Markdown worker failed.'));
        };
        worker.onmessageerror = () =>
          finish(job, null, new Error('Markdown result could not be read.'));
        job.timer = setTimeout(
          () =>
            finish(
              job,
              null,
              new Error('Markdown formatting timed out. The original text is retained.'),
            ),
          30_000,
        );
        worker.postMessage(job.request);
      } catch (error) {
        finish(job, null, error instanceof Error ? error : new Error(String(error)));
      }
    }
  }

  return (request: MarkdownRequest, signal?: AbortSignal): Promise<MarkdownDocument> =>
    new Promise((resolve, reject) => {
      const job: Job = {
        request,
        signal,
        resolve,
        reject,
        worker: null,
        timer: null,
        settled: false,
        abort: () =>
          finish(job, null, new DOMException('Markdown preparation canceled.', 'AbortError')),
      };
      if (signal?.aborted) {
        job.abort();
        return;
      }
      signal?.addEventListener('abort', job.abort, { once: true });
      waiting.push(job);
      pump();
    });
}

export const requestMarkdown = createMarkdownQueue(
  () => new Worker(new URL('./markdown-worker.ts', import.meta.url), { type: 'module' }),
);
