import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join, relative, sep } from 'node:path';

const CACHE_PREFIX = 'pkms-vite-ssr-';

export async function isolatedSsrCache(context) {
  const tempRoot = await realpath(tmpdir());
  const createdPath = await mkdtemp(join(tempRoot, CACHE_PREFIX));
  const cacheDir = await realpath(createdPath);
  // Each test closes its server in finally before this after hook runs.
  context.after(async () => {
    const actualPath = await realpath(createdPath);
    const child = relative(tempRoot, actualPath);
    if (
      relative(cacheDir, actualPath) !== '' ||
      isAbsolute(child) ||
      child.includes(sep) ||
      !child.startsWith(CACHE_PREFIX)
    ) {
      throw new Error('Refusing to remove an SSR cache outside its created temporary directory');
    }
    // Vite's dependency optimizer can still be writing `deps_temp_*` when a fast
    // test closes its server; on Windows that surfaces as ENOTEMPTY. Retry instead
    // of failing the test for a temporary directory we created ourselves.
    await rm(actualPath, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
  });
  return cacheDir;
}
