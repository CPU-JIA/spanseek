/**
 * The size of one document original, in the unit that keeps the number readable.
 *
 * The reader used to divide by 1024 and always print "KB", so an 8 MiB PDF read
 * as "8192.0 KB" and a 200 MiB scan as "204800.0 KB": past a few hundred
 * kilobytes the number stopped being something a reader could size up, and the
 * unit no longer matched the size limit the import dialog states in MiB.
 * Imported originals are capped at 256 MiB, so GB is the practical ceiling.
 */

/** `8.0 MB` / `1.6 KB` / `512 B`; unknown or negative sizes read as an em dash. */
export function formatSourceSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}
