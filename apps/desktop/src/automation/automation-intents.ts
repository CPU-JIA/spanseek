import type {
  AuditUndoCommit,
  ChangeItemCommit,
  ChangeSetCommit,
  DesktopCommand,
  ResearchCommit,
  ReviewSettingsCommit,
  WorkspaceIdentity,
} from '@pkms/contracts';

export type AutomationReplies = {
  automation_create_change_set: ChangeSetCommit;
  automation_apply_item: ChangeItemCommit;
  automation_skip_item: ChangeItemCommit;
  automation_reject_change_set: ChangeSetCommit;
  research_create: ResearchCommit;
  research_advance: ResearchCommit;
  research_run_external: ResearchCommit;
  research_cancel: ResearchCommit;
  audit_undo: AuditUndoCommit;
  review_insight_settings_set: ReviewSettingsCommit;
};
export type AutomationCommand = Extract<DesktopCommand, { kind: keyof AutomationReplies }>;
export type AutomationResult = AutomationReplies[keyof AutomationReplies];
export type AutomationIntent = {
  schema: 1;
  key: string;
  identity: WorkspaceIdentity;
  createdAt: string;
  command: AutomationCommand;
};
export type AutomationOutcome = { intent: AutomationIntent; result: AutomationResult };
export interface AutomationIntentStorage {
  read(key: string): Promise<AutomationIntent | undefined>;
  /** Atomically claim an empty slot or confirm the same original request. */
  claim(intent: AutomationIntent): Promise<void>;
  /** Remove only this exact request; never remove a replacement. */
  confirm(intent: AutomationIntent): Promise<void>;
}
type Snapshot = {
  busy: boolean;
  pending: AutomationIntent | null;
  error: string | null;
  code: string | null;
  generation: number;
};
const commandKinds = new Set<AutomationCommand['kind']>([
  'automation_create_change_set',
  'automation_apply_item',
  'automation_skip_item',
  'automation_reject_change_set',
  'research_create',
  'research_advance',
  'research_run_external',
  'research_cancel',
  'audit_undo',
  'review_insight_settings_set',
]);
const definitive = new Set([
  'metadata_conflict',
  'revision_conflict',
  'invalid_input',
  'object_trashed',
  'not_found',
]);
const codeOf = (error: unknown) =>
  error && typeof error === 'object' && 'details' in error
    ? ((error.details as { code?: string } | undefined)?.code ?? null)
    : null;
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const object = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value);
const uint32 = (value: unknown) =>
  typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 4_294_967_295;
const positiveVersion = (value: unknown) => uint32(value) && Number(value) > 0;
const uuid = (value: unknown) =>
  typeof value === 'string' &&
  /^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i.test(value);
const nullableUuid = (value: unknown) => value === null || uuid(value);
const nullableUint32 = (value: unknown) => value === null || uint32(value);
const fields = (value: Record<string, unknown>, names: readonly string[]) =>
  Object.keys(value).length === names.length && names.every((name) => Object.hasOwn(value, name));
const requestFields = {
  automation_create_change_set: ['kind', 'collection_id', 'limit', 'stale_after_days'],
  automation_apply_item: ['change_id', 'item_id', 'confirm'],
  automation_skip_item: ['change_id', 'item_id', 'confirm'],
  automation_reject_change_set: ['change_id'],
  research_create: [
    'goal',
    'collection_id',
    'queries',
    'budget',
    'stop',
    'external_allowed',
    'external_search_endpoint',
  ],
  research_advance: ['task_id', 'base_row_version'],
  research_cancel: ['task_id', 'base_row_version', 'reason'],
  audit_undo: ['audit_id', 'confirm'],
  review_insight_settings_set: [
    'base_row_version',
    'enabled',
    'window_days',
    'include_completed',
    'include_stale',
  ],
} as const;

export const automationIntentKey = (identity: WorkspaceIdentity) =>
  JSON.stringify([
    'spanseek-automation-intent-v1',
    identity.local_instance_id,
    identity.workspace_id,
  ]);

export function validateAutomationIntent(
  value: unknown,
  identity: WorkspaceIdentity,
): asserts value is AutomationIntent {
  if (
    !object(value) ||
    !fields(value, ['schema', 'key', 'identity', 'createdAt', 'command']) ||
    value.schema !== 1 ||
    value.key !== automationIntentKey(identity) ||
    !object(value.identity) ||
    !fields(value.identity, ['local_instance_id', 'workspace_id']) ||
    value.identity.local_instance_id !== identity.local_instance_id ||
    value.identity.workspace_id !== identity.workspace_id ||
    typeof value.createdAt !== 'string' ||
    !Number.isFinite(Date.parse(value.createdAt)) ||
    !object(value.command) ||
    !fields(value.command, ['kind', 'payload']) ||
    !commandKinds.has(value.command.kind as AutomationCommand['kind']) ||
    !object(value.command.payload) ||
    !uuid(value.command.payload.command_id)
  ) {
    throw new Error(
      '自动化待确认记录不匹配，原记录已保留 / Pending automation request does not match; original record retained',
    );
  }
  const command = value.command;
  const payload = command.payload as Record<string, unknown>;
  const request = payload.request;
  let valid = false;
  if (command.kind === 'research_run_external') {
    valid =
      fields(payload, ['command_id', 'task_id', 'base_row_version']) &&
      uuid(payload.task_id) &&
      positiveVersion(payload.base_row_version);
  } else if (object(request)) {
    const names = requestFields[command.kind as keyof typeof requestFields];
    if (!names || !fields(payload, ['command_id', 'request']) || !fields(request, names))
      throw new Error(
        '自动化请求字段不匹配，原记录已保留 / Automation request fields do not match; original record retained',
      );
    switch (command.kind) {
      case 'automation_create_change_set':
        valid =
          [
            'tag_suggestions',
            'duplicate_candidates',
            'relation_suggestions',
            'stale_sources',
          ].includes(String(request.kind)) &&
          uint32(request.limit) &&
          nullableUuid(request.collection_id) &&
          nullableUint32(request.stale_after_days);
        break;
      case 'automation_apply_item':
      case 'automation_skip_item':
        valid =
          uuid(request.change_id) && uuid(request.item_id) && typeof request.confirm === 'boolean';
        break;
      case 'automation_reject_change_set':
        valid = uuid(request.change_id);
        break;
      case 'research_create':
        valid =
          typeof request.goal === 'string' &&
          nullableUuid(request.collection_id) &&
          Array.isArray(request.queries) &&
          request.queries.every((item) => typeof item === 'string') &&
          object(request.budget) &&
          fields(request.budget, ['max_steps', 'max_sources']) &&
          uint32(request.budget.max_steps) &&
          uint32(request.budget.max_sources) &&
          object(request.stop) &&
          fields(request.stop, ['deadline_seconds', 'missing_after_step']) &&
          nullableUint32(request.stop.deadline_seconds) &&
          nullableUint32(request.stop.missing_after_step) &&
          typeof request.external_allowed === 'boolean' &&
          (request.external_search_endpoint === null ||
            typeof request.external_search_endpoint === 'string');
        break;
      case 'research_advance':
        valid = uuid(request.task_id) && positiveVersion(request.base_row_version);
        break;
      case 'research_cancel':
        valid =
          uuid(request.task_id) &&
          positiveVersion(request.base_row_version) &&
          typeof request.reason === 'string';
        break;
      case 'audit_undo':
        valid = uuid(request.audit_id) && request.confirm === true;
        break;
      case 'review_insight_settings_set':
        valid =
          positiveVersion(request.base_row_version) &&
          typeof request.enabled === 'boolean' &&
          uint32(request.window_days) &&
          typeof request.include_completed === 'boolean' &&
          typeof request.include_stale === 'boolean';
        break;
    }
  }
  if (!valid || JSON.stringify(value).length > 1_048_576)
    throw new Error(
      '自动化待确认载荷无效，原记录已保留 / Invalid pending automation payload; original record retained',
    );
}

function verifyResult(command: AutomationCommand, result: AutomationResult) {
  if (!result?.receipt || result.receipt.command_id !== command.payload.command_id)
    throw new Error(
      '自动化回执不匹配，原请求已保留 / Automation receipt mismatch; original request retained',
    );
  if (
    (command.kind.startsWith('automation_') && !('change' in result)) ||
    (command.kind.startsWith('research_') && !('task' in result)) ||
    (command.kind === 'review_insight_settings_set' && !('settings' in result))
  )
    throw new Error('自动化回执类型不匹配 / Automation receipt type mismatch');
  if (
    (command.kind === 'automation_apply_item' || command.kind === 'automation_skip_item') &&
    (!('item' in result) || result.item.item_id !== command.payload.request.item_id)
  )
    throw new Error('提案条目回执不匹配 / Proposal item receipt mismatch');
  if (
    command.kind === 'automation_apply_item' ||
    command.kind === 'automation_skip_item' ||
    command.kind === 'automation_reject_change_set'
  ) {
    if (!('change' in result) || result.change.id !== command.payload.request.change_id)
      throw new Error('提案回执对象不匹配 / Proposal receipt object mismatch');
  } else if (
    command.kind === 'research_advance' ||
    command.kind === 'research_cancel' ||
    command.kind === 'research_run_external'
  ) {
    const id =
      command.kind === 'research_run_external'
        ? command.payload.task_id
        : command.payload.request.task_id;
    if (!('task' in result) || result.task.id !== id)
      throw new Error('研究回执对象不匹配 / Research receipt object mismatch');
  } else if (
    command.kind === 'audit_undo' &&
    (!('reversed' in result) || !uuid(result.reversed.id) || result.reversed.kind !== 'audit.undo')
  ) {
    // `reversed` is the new compensating audit event, not the original event.
    throw new Error('撤销回执事件不匹配 / Undo receipt event mismatch');
  }
}

/** One durable, exact mutation per local workspace. Reads do not use this queue. */
export class AutomationActions {
  private state: Snapshot = { busy: false, pending: null, error: null, code: null, generation: 0 };
  private listeners = new Set<() => void>();
  private identity: WorkspaceIdentity;
  private storage: AutomationIntentStorage;
  private send: (command: AutomationCommand) => Promise<AutomationResult>;
  private constructor(
    identity: WorkspaceIdentity,
    storage: AutomationIntentStorage,
    send: (command: AutomationCommand) => Promise<AutomationResult>,
  ) {
    this.identity = identity;
    this.storage = storage;
    this.send = send;
  }
  static async open(
    identity: WorkspaceIdentity,
    storage: AutomationIntentStorage,
    send: (command: AutomationCommand) => Promise<AutomationResult>,
  ) {
    const value = new AutomationActions(structuredClone(identity), storage, send);
    const pending = await storage.read(automationIntentKey(identity));
    if (pending) validateAutomationIntent(pending, identity);
    value.state = { ...value.state, pending: pending ? structuredClone(pending) : null };
    return value;
  }
  getSnapshot = () => this.state;
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
  async run(
    command?: AutomationCommand,
    consume?: (outcome: AutomationOutcome) => boolean,
  ): Promise<AutomationOutcome | null> {
    if (this.state.busy || (command && this.state.pending) || (!command && !this.state.pending))
      return null;
    this.emit({ busy: true, error: null, code: null });
    let sent = false;
    try {
      if (!this.state.pending) {
        const intent: AutomationIntent = {
          schema: 1,
          key: automationIntentKey(this.identity),
          identity: structuredClone(this.identity),
          createdAt: new Date().toISOString(),
          command: structuredClone(command!),
        };
        validateAutomationIntent(intent, this.identity);
        this.emit({ pending: intent });
      }
      const intent = this.state.pending!;
      validateAutomationIntent(intent, this.identity);
      await this.storage.claim(intent);
      sent = true;
      const result = await this.send(structuredClone(intent.command));
      verifyResult(intent.command, result);
      const outcome = { intent: structuredClone(intent), result };
      // Navigation may have left this scope while Core committed. Keep the
      // original request until an active screen has consumed its outcome.
      if (consume && !consume(outcome)) return null;
      await this.storage.confirm(intent);
      this.emit({ pending: null, generation: this.state.generation + 1 });
      return outcome;
    } catch (error) {
      const code = codeOf(error);
      if (!sent) {
        // Another window may already own this workspace slot. Adopt its exact
        // request instead of continually trying to replace it with this one.
        try {
          const stored = await this.storage.read(automationIntentKey(this.identity));
          if (stored) {
            validateAutomationIntent(stored, this.identity);
            this.emit({ pending: structuredClone(stored) });
          }
        } catch {
          /* Preserve the in-memory request and report the first storage error. */
        }
      }
      if (sent && code && definitive.has(code) && this.state.pending) {
        try {
          await this.storage.confirm(this.state.pending);
          this.emit({ pending: null });
        } catch (storageError) {
          this.emit({ error: `${errorText(error)} · ${errorText(storageError)}`, code });
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
