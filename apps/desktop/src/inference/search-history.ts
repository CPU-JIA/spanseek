import type { SearchMode, SearchQuery, SearchScope, WorkspaceIdentity } from '@pkms/contracts';
import type { SearchResultOrder } from './search-rerank';

export const SEARCH_HISTORY_LIMIT = 20;
export const SEARCH_HISTORY_VERSION = 1;

export type SearchHistoryEntry = {
  id: string;
  query: string;
  mode: SearchMode;
  scope: SearchScope;
  limit: number;
  minimum_cosine: number | null;
  order: SearchResultOrder;
  result_count: number;
  searched_at: string;
};

function emptyScope(): SearchScope {
  return {
    object_ids: [],
    kinds: [],
    tag_ids: [],
    collection_id: null,
    inbox_state: null,
    reading_status: null,
    source_format: null,
    archived: null,
  };
}

function textArray(value: unknown, max: number): string[] {
  return Array.isArray(value)
    ? value
        .filter((item): item is string => typeof item === 'string' && item.length <= 128)
        .slice(0, max)
    : [];
}

function nullableString(value: unknown): string | null {
  return typeof value === 'string' && value.length <= 128 ? value : null;
}

function scope(value: unknown): SearchScope | null {
  if (!value || typeof value !== 'object') return null;
  const source = value as Record<string, unknown>;
  const kinds = textArray(source.kinds, 2).filter(
    (item): item is 'note' | 'source' => item === 'note' || item === 'source',
  );
  const inbox = source.inbox_state;
  const reading = source.reading_status;
  const format = source.source_format;
  return {
    object_ids: textArray(source.object_ids, 100),
    kinds,
    tag_ids: textArray(source.tag_ids, 100),
    collection_id: nullableString(source.collection_id),
    inbox_state: inbox === 'pending' || inbox === 'snoozed' || inbox === 'processed' ? inbox : null,
    reading_status:
      reading === 'unread' || reading === 'reading' || reading === 'finished' ? reading : null,
    source_format:
      format === 'pdf' ||
      format === 'html' ||
      format === 'text' ||
      format === 'markdown' ||
      format === 'docx' ||
      format === 'doc' ||
      format === 'epub' ||
      format === 'mobi' ||
      format === 'csv' ||
      format === 'xlsx' ||
      format === 'xls' ||
      format === 'pptx'
        ? format
        : null,
    archived: typeof source.archived === 'boolean' ? source.archived : null,
  };
}

function mode(value: unknown): SearchMode | null {
  return value === 'lexical' || value === 'semantic' || value === 'hybrid' ? value : null;
}

function order(value: unknown): SearchResultOrder | null {
  return value === 'fusion' || value === 'keyword' || value === 'semantic' ? value : null;
}

function validTime(value: unknown): string | null {
  if (typeof value !== 'string' || !Number.isFinite(Date.parse(value))) return null;
  return new Date(value).toISOString();
}

function fallbackId(searchedAt: string, query: string): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `${searchedAt}:${query}`;
}

function normalizeEntry(value: unknown): SearchHistoryEntry | null {
  if (!value || typeof value !== 'object') return null;
  const source = value as Record<string, unknown>;
  const query = typeof source.query === 'string' ? source.query.trim().slice(0, 4096) : '';
  const parsedMode = mode(source.mode);
  const parsedScope = scope(source.scope);
  const parsedOrder = order(source.order);
  const searchedAt = validTime(source.searched_at);
  const limit = Number(source.limit);
  const minimumCosine = source.minimum_cosine;
  const resultCount = Number(source.result_count);
  if (
    !query ||
    !parsedMode ||
    !parsedScope ||
    !parsedOrder ||
    !searchedAt ||
    !Number.isInteger(limit) ||
    limit < 1 ||
    limit > 100 ||
    !Number.isInteger(resultCount) ||
    resultCount < 0 ||
    resultCount > 1000 ||
    (minimumCosine !== null &&
      (typeof minimumCosine !== 'number' ||
        !Number.isFinite(minimumCosine) ||
        minimumCosine < -1 ||
        minimumCosine > 1))
  ) {
    return null;
  }
  return {
    id:
      typeof source.id === 'string' && source.id.length > 0 && source.id.length <= 128
        ? source.id
        : fallbackId(searchedAt, query),
    query,
    mode: parsedMode,
    scope: parsedScope,
    limit,
    minimum_cosine: minimumCosine as number | null,
    order: parsedOrder,
    result_count: resultCount,
    searched_at: searchedAt,
  };
}

function historyKey(entry: SearchHistoryEntry): string {
  return JSON.stringify([
    entry.query.toLocaleLowerCase(),
    entry.mode,
    entry.scope,
    entry.limit,
    entry.minimum_cosine,
  ]);
}

export function parseSearchHistory(raw: string | null): SearchHistoryEntry[] {
  if (!raw) return [];
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return [];
  }
  const entries = Array.isArray(value)
    ? value
    : value &&
        typeof value === 'object' &&
        (value as Record<string, unknown>).version === SEARCH_HISTORY_VERSION &&
        Array.isArray((value as Record<string, unknown>).entries)
      ? ((value as Record<string, unknown>).entries as unknown[])
      : [];
  const seen = new Set<string>();
  const output: SearchHistoryEntry[] = [];
  for (const item of entries) {
    const entry = normalizeEntry(item);
    if (!entry) continue;
    const key = historyKey(entry);
    if (seen.has(key)) continue;
    seen.add(key);
    output.push(entry);
    if (output.length >= SEARCH_HISTORY_LIMIT) break;
  }
  return output;
}

export function serializeSearchHistory(entries: readonly SearchHistoryEntry[]): string {
  return JSON.stringify({ version: SEARCH_HISTORY_VERSION, entries });
}

export function searchHistoryStorageKey(identity: WorkspaceIdentity): string {
  return JSON.stringify([
    'spanseek',
    identity.local_instance_id,
    identity.workspace_id,
    'search-history',
    SEARCH_HISTORY_VERSION,
  ]);
}

export function searchHistoryEntry(input: {
  id?: string;
  query: SearchQuery;
  order: SearchResultOrder;
  resultCount: number;
  searchedAt?: string;
}): SearchHistoryEntry | null {
  return normalizeEntry({
    id: input.id,
    query: input.query.query,
    mode: input.query.mode,
    scope: input.query.scope,
    limit: input.query.limit,
    minimum_cosine: input.query.minimum_cosine,
    order: input.order,
    result_count: input.resultCount,
    searched_at: input.searchedAt ?? new Date().toISOString(),
  });
}

export function recordSearchHistory(
  entries: readonly SearchHistoryEntry[],
  next: SearchHistoryEntry | null,
): SearchHistoryEntry[] {
  const normalized = parseSearchHistory(serializeSearchHistory(entries));
  if (!next) return normalized;
  const key = historyKey(next);
  return [next, ...normalized.filter((entry) => historyKey(entry) !== key)].slice(
    0,
    SEARCH_HISTORY_LIMIT,
  );
}

export function searchQueryFromHistory(entry: SearchHistoryEntry): SearchQuery {
  return {
    query: entry.query,
    mode: entry.mode,
    scope: structuredClone(entry.scope),
    limit: entry.limit,
    minimum_cosine: entry.minimum_cosine,
  };
}
