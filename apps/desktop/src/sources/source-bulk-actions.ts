import type {
  CollectionDetail,
  DesktopCommand,
  ObjectMetadata,
  WorkspaceIdentity,
} from '@pkms/contracts';

export type SelectedSource = { id: string; title: string };
export type SourceBulkAction =
  | { kind: 'tag'; name: string }
  | { kind: 'collection'; id: string; name: string }
  | { kind: 'search' | 'trash'; value: boolean };
type BulkCommand = Extract<
  DesktopCommand,
  {
    kind:
      | 'tags_assign'
      | 'collections_add_members'
      | 'objects_set_search_enabled'
      | 'sources_set_trashed';
  }
>;
export type SourceBulkStep = {
  ids: string[];
  command: BulkCommand;
  expected: 'metadata_commit' | 'collection_commit' | 'source_mutation';
  status: 'ready' | 'pending' | 'done' | 'failed';
  error?: string;
};
export type SourceBulkRecord = {
  schema: 1;
  key: string;
  id: string;
  identity: WorkspaceIdentity;
  selected: SelectedSource[];
  action: SourceBulkAction;
  steps: SourceBulkStep[];
};
export interface SourceBulkStorage {
  read(key: string): Promise<SourceBulkRecord | undefined>;
  write(record: SourceBulkRecord): Promise<void>;
  remove(record: SourceBulkRecord): Promise<void>;
}
export const sourceBulkKey = (identity: WorkspaceIdentity) =>
  JSON.stringify([identity.local_instance_id, identity.workspace_id, 'source-bulk']);
const text = (error: unknown) => (error instanceof Error ? error.message : String(error));
const terminal = new Set(['invalid_input', 'metadata_conflict', 'object_trashed', 'not_found']);

export async function prepareSourceBulk(
  selected: SelectedSource[],
  action: SourceBulkAction,
  readMetadata: (id: string) => Promise<ObjectMetadata>,
  readCollection: (id: string) => Promise<CollectionDetail>,
): Promise<SourceBulkStep[]> {
  if (
    !selected.length ||
    selected.length > 100 ||
    new Set(selected.map((item) => item.id)).size !== selected.length
  )
    throw new Error('请选择 1–100 份不同资料 / Select 1–100 distinct documents');
  const ids = selected.map((item) => item.id);
  if (action.kind === 'collection') {
    const collection = await readCollection(action.id);
    if (collection.id !== action.id || collection.metadata.trashed)
      throw new Error('目标集合不可用，请重新选择 / Choose an available collection');
    return [
      {
        ids,
        status: 'ready',
        expected: 'collection_commit',
        command: {
          kind: 'collections_add_members',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              collection_id: action.id,
              base_row_version: collection.metadata.row_version,
              member_ids: ids,
            },
          },
        },
      },
    ];
  }
  const metadata = await Promise.all(ids.map(readMetadata));
  if (metadata.some((item, index) => item.id !== ids[index] || item.kind !== 'source'))
    throw new Error('资料信息不匹配，请刷新列表 / Document metadata changed; refresh the list');
  if (action.kind === 'tag') {
    if (!action.name.trim()) throw new Error('请输入标签名称 / Enter a tag name');
    return [
      {
        ids,
        status: 'ready',
        expected: 'metadata_commit',
        command: {
          kind: 'tags_assign',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              targets: metadata.map((item) => ({
                object_id: item.id,
                base_row_version: item.row_version,
              })),
              tag: { kind: 'name', name: action.name.trim() },
            },
          },
        },
      },
    ];
  }
  return metadata.map((item) => ({
    ids: [item.id],
    status: 'ready',
    expected: action.kind === 'trash' ? 'source_mutation' : 'metadata_commit',
    command: {
      kind: action.kind === 'trash' ? 'sources_set_trashed' : 'objects_set_search_enabled',
      payload: {
        command_id: crypto.randomUUID(),
        request: {
          object_id: item.id,
          base_row_version: item.row_version,
          value: action.value,
        },
      },
    },
  }));
}

export function validateSourceBulk(record: SourceBulkRecord, identity: WorkspaceIdentity) {
  const invalid = () => {
    throw new Error(
      '资料整理记录校验失败，已保留供恢复 / Invalid document batch; retained for recovery',
    );
  };
  if (
    !record ||
    record.schema !== 1 ||
    record.key !== sourceBulkKey(identity) ||
    !record.id ||
    !record.identity ||
    sourceBulkKey(record.identity) !== record.key ||
    !Array.isArray(record.selected) ||
    !record.selected.length ||
    record.selected.length > 100 ||
    record.selected.some(
      (item) => !item || typeof item.id !== 'string' || !item.id || typeof item.title !== 'string',
    ) ||
    !Array.isArray(record.steps) ||
    !record.steps.length ||
    record.steps.length > 100 ||
    !record.action
  )
    return invalid();
  const ids = record.selected.map((item) => item.id);
  if (new Set(ids).size !== ids.length) return invalid();
  const action = record.action;
  if (
    !['tag', 'collection', 'search', 'trash'].includes(action.kind) ||
    (action.kind === 'tag' && (typeof action.name !== 'string' || !action.name.trim())) ||
    (action.kind === 'collection' &&
      (typeof action.id !== 'string' || !action.id || typeof action.name !== 'string')) ||
    ((action.kind === 'search' || action.kind === 'trash') && typeof action.value !== 'boolean')
  )
    return invalid();
  const covered: string[] = [];
  const commandIds = new Set<string>();
  for (const step of record.steps) {
    if (
      !step ||
      !Array.isArray(step.ids) ||
      !step.ids.length ||
      !['ready', 'pending', 'done', 'failed'].includes(step.status) ||
      typeof step.command?.payload?.command_id !== 'string' ||
      !step.command.payload.command_id ||
      commandIds.has(step.command.payload.command_id)
    )
      return invalid();
    commandIds.add(step.command.payload.command_id);
    covered.push(...step.ids);
    const command = step.command;
    if (action.kind === 'tag') {
      if (
        record.steps.length !== 1 ||
        command.kind !== 'tags_assign' ||
        step.expected !== 'metadata_commit' ||
        command.payload.request?.tag?.kind !== 'name' ||
        command.payload.request.tag.name !== action.name.trim() ||
        JSON.stringify(command.payload.request.targets?.map((item) => item.object_id)) !==
          JSON.stringify(step.ids) ||
        command.payload.request.targets.some((item) => !Number.isSafeInteger(item.base_row_version))
      )
        return invalid();
    } else if (action.kind === 'collection') {
      if (
        record.steps.length !== 1 ||
        command.kind !== 'collections_add_members' ||
        step.expected !== 'collection_commit' ||
        command.payload.request?.collection_id !== action.id ||
        !Number.isSafeInteger(command.payload.request.base_row_version) ||
        JSON.stringify(command.payload.request.member_ids) !== JSON.stringify(step.ids)
      )
        return invalid();
    } else {
      if (
        step.ids.length !== 1 ||
        command.kind !==
          (action.kind === 'trash' ? 'sources_set_trashed' : 'objects_set_search_enabled') ||
        step.expected !== (action.kind === 'trash' ? 'source_mutation' : 'metadata_commit') ||
        !('object_id' in command.payload.request) ||
        command.payload.request.object_id !== step.ids[0] ||
        !Number.isSafeInteger(command.payload.request.base_row_version) ||
        command.payload.request.value !== action.value
      )
        return invalid();
    }
  }
  if (JSON.stringify(covered) !== JSON.stringify(ids)) return invalid();
}

/** Freeze the complete batch before dispatch. Unknown replies only replay the original command. */
export class SourceBulkActions {
  private state: {
    ready: boolean;
    busy: boolean;
    record: SourceBulkRecord | null;
    error: string | null;
    generation: number;
  } = {
    ready: false,
    busy: false,
    record: null,
    error: null,
    generation: 0,
  };
  private listeners = new Set<() => void>();
  private loading: Promise<void> | null = null;
  private identity: WorkspaceIdentity;
  private storage: SourceBulkStorage;
  private prepare: (
    selected: SelectedSource[],
    action: SourceBulkAction,
  ) => Promise<SourceBulkStep[]>;
  private send: (step: SourceBulkStep) => Promise<unknown>;
  constructor(
    identity: WorkspaceIdentity,
    storage: SourceBulkStorage,
    prepare: SourceBulkActions['prepare'],
    send: SourceBulkActions['send'],
  ) {
    this.identity = structuredClone(identity);
    this.storage = storage;
    this.prepare = prepare;
    this.send = send;
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private emit(patch: Partial<typeof this.state>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((listener) => listener());
  }
  load = async () => {
    if (this.state.ready) return;
    if (this.loading) return this.loading;
    this.loading = this.storage
      .read(sourceBulkKey(this.identity))
      .then((record) => {
        if (record) validateSourceBulk(record, this.identity);
        this.emit({ record: record ?? null, ready: true, error: null });
      })
      .catch((error) => this.emit({ error: text(error) }))
      .finally(() => {
        this.loading = null;
      });
    return this.loading;
  };
  private async save(record: SourceBulkRecord) {
    await this.storage.write(structuredClone(record));
    this.emit({ record: structuredClone(record) });
  }
  start = async (selected: SelectedSource[], action: SourceBulkAction) => {
    if (!this.state.ready || this.state.busy || this.state.record) return;
    this.emit({ busy: true, error: null });
    try {
      const frozen = structuredClone({ selected, action });
      const steps = await this.prepare(frozen.selected, frozen.action);
      const record: SourceBulkRecord = {
        schema: 1,
        key: sourceBulkKey(this.identity),
        id: crypto.randomUUID(),
        identity: this.identity,
        ...frozen,
        steps,
      };
      validateSourceBulk(record, this.identity);
      await this.save(record);
    } catch (error) {
      this.emit({ error: text(error) });
    } finally {
      this.emit({ busy: false });
    }
    if (this.state.record) await this.resume();
  };
  resume = async () => {
    if (!this.state.ready || this.state.busy || !this.state.record) return;
    this.emit({ busy: true, error: null });
    const record = structuredClone(this.state.record);
    try {
      for (const step of record.steps) {
        if (step.status === 'done' || step.status === 'failed') continue;
        step.status = 'pending';
        delete step.error;
        await this.save(record);
        let committed = false;
        try {
          await this.send(structuredClone(step));
          step.status = 'done';
          committed = true;
        } catch (error) {
          const code = (error as { details?: { code?: string } })?.details?.code;
          step.status = code && terminal.has(code) ? 'failed' : 'pending';
          step.error = text(error);
        }
        // Persisting the acknowledgement is outside the transport catch: a disk failure is
        // never a definitive rejection and must leave the original command replayable.
        await this.save(record);
        if (committed) this.emit({ generation: this.state.generation + 1 });
        if (step.status === 'pending') break;
      }
    } catch (error) {
      this.emit({ error: text(error) });
    } finally {
      this.emit({ busy: false });
    }
  };
  dismiss = async () => {
    const record = this.state.record;
    if (
      !record ||
      this.state.busy ||
      record.steps.some((step) => step.status === 'ready' || step.status === 'pending')
    )
      return false;
    this.emit({ busy: true, error: null });
    try {
      await this.storage.remove(record);
      this.emit({ record: null });
      return true;
    } catch (error) {
      this.emit({ error: text(error) });
      return false;
    } finally {
      this.emit({ busy: false });
    }
  };
}
