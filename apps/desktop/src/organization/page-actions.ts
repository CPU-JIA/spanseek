import type {
  CollectionCommit,
  CanvasCommit,
  DesktopCommand,
  MetadataCommit,
  NoteCommit,
  ViewCommit,
  WorkspaceIdentity,
} from '@pkms/contracts';

export type OrganizationArea = 'collections' | 'views' | 'canvas' | 'note-trash';
export type OrganizationCommand = Extract<
  DesktopCommand,
  {
    kind:
      | 'collections_create'
      | 'collections_update'
      | 'collections_add_members'
      | 'collections_remove_members'
      | 'collections_reorder'
      | 'objects_set_trashed'
      | 'objects_set_archived'
      | 'views_save'
      | 'views_remove'
      | 'canvases_save'
      | 'canvases_remove'
      | 'notes_save';
  }
>;
export type OrganizationResult =
  CollectionCommit | MetadataCommit | ViewCommit | CanvasCommit | NoteCommit;
export type PageIntent = {
  schema: 1;
  key: string;
  identity: WorkspaceIdentity;
  area: OrganizationArea;
  command: OrganizationCommand;
};
export interface PageIntentStorage {
  read(key: string): Promise<PageIntent | undefined>;
  write(intent: PageIntent): Promise<void>;
  remove(key: string): Promise<void>;
}
export const pageKey = (identity: WorkspaceIdentity, area: OrganizationArea) =>
  `${identity.local_instance_id}/${identity.workspace_id}/${area}`;
export const actionErrorCode = (error: unknown): string | undefined =>
  error && typeof error === 'object' && 'details' in error
    ? (error.details as { code?: string } | undefined)?.code
    : undefined;
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
type Snapshot = {
  busy: boolean;
  pending: PageIntent | null;
  error: string | null;
  code: string | null;
  generation: number;
  result: OrganizationResult | null;
};
const terminal = new Set([
  'metadata_conflict',
  'revision_conflict',
  'invalid_input',
  'object_trashed',
  'not_found',
]);
const kinds = new Set([
  'collections_create',
  'collections_update',
  'collections_add_members',
  'collections_remove_members',
  'collections_reorder',
  'objects_set_trashed',
  'objects_set_archived',
  'views_save',
  'views_remove',
  'canvases_save',
  'canvases_remove',
  'notes_save',
]);
function belongsToArea(area: OrganizationArea, kind: string) {
  if (area === 'views') return kind.startsWith('views_');
  if (area === 'canvas') return kind.startsWith('canvases_') || kind === 'notes_save';
  if (area === 'note-trash') return kind === 'objects_set_trashed';
  return (
    kind.startsWith('collections_') ||
    kind === 'objects_set_trashed' ||
    kind === 'objects_set_archived'
  );
}

/** One exact durable command per workspace/page. It outlives React mounts. */
export class PageActions {
  private state: Snapshot = {
    busy: false,
    pending: null,
    error: null,
    code: null,
    generation: 0,
    result: null,
  };
  private listeners = new Set<() => void>();
  private identity: WorkspaceIdentity;
  private area: OrganizationArea;
  private storage: PageIntentStorage;
  private send: (command: OrganizationCommand) => Promise<OrganizationResult>;
  private constructor(
    identity: WorkspaceIdentity,
    area: OrganizationArea,
    storage: PageIntentStorage,
    send: (command: OrganizationCommand) => Promise<OrganizationResult>,
  ) {
    this.identity = identity;
    this.area = area;
    this.storage = storage;
    this.send = send;
  }
  static async open(
    identity: WorkspaceIdentity,
    area: OrganizationArea,
    storage: PageIntentStorage,
    send: (command: OrganizationCommand) => Promise<OrganizationResult>,
  ) {
    const controller = new PageActions(structuredClone(identity), area, storage, send);
    const key = pageKey(identity, area);
    const intent = await storage.read(key);
    if (
      intent &&
      (intent.schema !== 1 ||
        intent.key !== key ||
        !intent.identity ||
        pageKey(intent.identity, intent.area) !== key ||
        !intent.command ||
        !kinds.has(intent.command.kind) ||
        !intent.command.payload?.command_id ||
        !belongsToArea(area, intent.command.kind))
    ) {
      throw new Error('Invalid pending organization operation / 整理操作记录校验失败');
    }
    controller.state = { ...controller.state, pending: intent ?? null };
    return controller;
  }
  getSnapshot = () => this.state;
  clearError = () => {
    if (!this.state.busy && !this.state.pending) this.emit({ error: null, code: null });
  };
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private emit(patch: Partial<Snapshot>) {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
  async run(command?: OrganizationCommand): Promise<OrganizationResult | null> {
    if (this.state.busy || (command && this.state.pending)) return null;
    if (!command && !this.state.pending) return null;
    if (command && (!kinds.has(command.kind) || !belongsToArea(this.area, command.kind))) {
      this.emit({
        error: 'Operation does not belong to this page / 操作与当前页面不匹配',
        code: 'invalid_input',
      });
      return null;
    }
    this.emit({ busy: true, error: null, code: null });
    let sent = false;
    try {
      if (!this.state.pending)
        this.emit({
          pending: {
            schema: 1,
            key: pageKey(this.identity, this.area),
            identity: structuredClone(this.identity),
            area: this.area,
            command: structuredClone(command!),
          },
        });
      const intent = this.state.pending!;
      // Retry persistence too: a failed local write must never permit a send.
      await this.storage.write(intent);
      sent = true;
      const result = await this.send(structuredClone(intent.command));
      await this.storage.remove(intent.key);
      this.emit({ pending: null, generation: this.state.generation + 1, result });
      return result;
    } catch (error) {
      const code = actionErrorCode(error) ?? null;
      if (sent && code && terminal.has(code) && this.state.pending) {
        try {
          await this.storage.remove(this.state.pending.key);
          this.emit({ pending: null });
        } catch (storageError) {
          this.emit({ error: `${errorText(error)}; ${errorText(storageError)}`, code });
          return null;
        }
      }
      this.emit({ error: errorText(error), code });
      return null;
    } finally {
      this.emit({ busy: false });
    }
  }
}
