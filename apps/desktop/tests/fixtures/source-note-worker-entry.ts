// Build fixture exercises Vite's actual nested worker pipeline and dependency conditions.
export function start() {
  return new Worker(new URL('../../src/sources/source-note-worker.ts', import.meta.url), {
    type: 'module',
  });
}
