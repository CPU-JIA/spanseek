import type {
  DeleteGenerationChannel,
  GenerationChannel,
  GenerationChannelModel,
  GenerationChannelMutation,
  GenerationChannelSpec,
  GenerationChannelsState,
  GenerationProfile,
  GenerationSettings,
  GenerationSettingsMutation,
  RemoteGenerationModels,
  SaveGenerationChannel,
  SelectGenerationChannelModel,
} from '@pkms/contracts';
import {
  cloudflareEndpoint,
  validateChannelSpec,
  type ChannelTemplate,
} from './generation-channel-presets.ts';

export type ChannelDraftEntry = {
  id: string;
  baseRowVersion: number;
  spec: GenerationChannelSpec;
  dirty: boolean;
  conflict: boolean;
  template?: ChannelTemplate;
  accountInput?: string;
};
type Snapshot = {
  server: GenerationChannelsState | null;
  drafts: Record<string, ChannelDraftEntry>;
  selectedId: string | null;
  loaded: boolean;
  loading: boolean;
  writing: 'save' | 'select' | 'delete' | null;
  error: string | null;
  connectionEpoch: number;
  catalog: RemoteGenerationModels | null;
  listing: boolean;
  catalogError: string | null;
  credentialConfigured: boolean | null;
  credentialBusy: boolean;
  credentialError: string | null;
  checkingModel: string | null;
  verifiedModels: string[];
  verificationError: string | null;
};

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object')
    return Object.fromEntries(
      Object.entries(value)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, item]) => [key, canonical(item)]),
    );
  return value;
}
const equal = (a: unknown, b: unknown) =>
  JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const entry = (channel: GenerationChannel): ChannelDraftEntry => ({
  id: channel.id,
  baseRowVersion: channel.row_version,
  spec: structuredClone(channel.spec),
  dirty: false,
  conflict: false,
});
const safeReasons = new Set([
  'model_key_missing',
  'model_invalid_key',
  'model_credential_unavailable',
  'model_credential_unsupported',
  'model_authentication_failed',
  'model_rate_limited',
  'model_api_error',
  'model_unavailable',
  'model_invalid_response',
  'model_invalid_endpoint',
  'model_response_too_large',
  'model_profile_unsupported',
  'model_discovery_unsupported',
  'model_response_invalid',
  'model_response_limit',
  'model_unsupported_output',
  'model_incomplete',
  'model_first_token_timeout',
  'model_total_timeout',
  'model_idle_timeout',
]);
/** Keep only known error codes. Transport exceptions can contain credentials or remote bodies. */
export function safeChannelError(error: unknown, fallback: string): string {
  const api = (error as { details?: { code?: string; details?: { reason?: string } } })?.details;
  if (api?.code === 'metadata_conflict' || api?.code === 'revision_conflict')
    return 'channel_conflict';
  if (api?.code === 'not_found') return 'channel_missing';
  const reason = api?.details?.reason;
  return reason && safeReasons.has(reason) ? reason : fallback;
}

/** Workspace-owned drafts. Refreshes merge saved state without replacing edited channel specs. */
export class GenerationChannelDraft {
  private active = true;
  private lifetime = 0;
  private readSequence = 0;
  private connectionSequence = 0;
  private catalogSequence = 0;
  private credentialSequence = 0;
  private verificationSequence = 0;
  private listeners = new Set<() => void>();
  private state: Snapshot = {
    server: null,
    drafts: {},
    selectedId: null,
    loaded: false,
    loading: false,
    writing: null,
    error: null,
    connectionEpoch: 0,
    catalog: null,
    listing: false,
    catalogError: null,
    credentialConfigured: null,
    credentialBusy: false,
    credentialError: null,
    checkingModel: null,
    verifiedModels: [],
    verificationError: null,
  };
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
  get selected() {
    return this.state.selectedId ? (this.state.drafts[this.state.selectedId] ?? null) : null;
  }
  get isActive() {
    return this.active;
  }
  get locked() {
    return !this.active || !this.state.loaded || this.state.loading || this.state.writing !== null;
  }
  get savedChannel(): GenerationChannel | null {
    const current = this.selected;
    if (!current || current.dirty || current.conflict) return null;
    return (
      this.state.server?.channels.find(
        (channel) =>
          channel.id === current.id &&
          !channel.deleted &&
          channel.row_version === current.baseRowVersion &&
          equal(channel.spec, current.spec),
      ) ?? null
    );
  }
  get canSave() {
    return (
      !this.locked &&
      !!this.selected?.dirty &&
      !this.selected.conflict &&
      !validateChannelSpec(this.selected.spec)
    );
  }
  invalidate() {
    this.active = false;
    ++this.lifetime;
    ++this.readSequence;
    this.resetConnection();
  }
  private resetConnection(preserveCatalog = false) {
    ++this.connectionSequence;
    ++this.catalogSequence;
    ++this.credentialSequence;
    ++this.verificationSequence;
    this.emit({
      connectionEpoch: this.connectionSequence,
      catalog: preserveCatalog ? this.state.catalog : null,
      listing: false,
      catalogError: null,
      credentialConfigured: null,
      credentialBusy: false,
      credentialError: null,
      checkingModel: null,
      verifiedModels: [],
      verificationError: null,
    });
  }
  async load(read: () => Promise<GenerationChannelsState>) {
    if (this.state.writing) return false;
    this.active = true;
    const lifetime = this.lifetime,
      sequence = ++this.readSequence;
    this.emit({ loading: true, error: null });
    try {
      const incoming = await read();
      if (!this.active || lifetime !== this.lifetime || sequence !== this.readSequence)
        return false;
      const drafts = { ...this.state.drafts };
      const channels = incoming.channels.filter((channel) => !channel.deleted);
      for (const channel of channels) {
        const old = drafts[channel.id];
        drafts[channel.id] = old?.dirty
          ? {
              ...old,
              conflict:
                old.conflict ||
                (old.baseRowVersion !== 0 && old.baseRowVersion !== channel.row_version),
            }
          : { ...entry(channel), template: old?.template };
      }
      for (const [id, draft] of Object.entries(drafts)) {
        if (draft.baseRowVersion > 0 && !channels.some((channel) => channel.id === id)) {
          if (draft.dirty) drafts[id] = { ...draft, conflict: true };
          else delete drafts[id];
        }
      }
      const selectedId =
        this.state.selectedId && drafts[this.state.selectedId]
          ? this.state.selectedId
          : incoming.settings.profile?.channel_id && drafts[incoming.settings.profile.channel_id]
            ? incoming.settings.profile.channel_id
            : (Object.keys(drafts)[0] ?? null);
      this.emit({ server: structuredClone(incoming), drafts, selectedId, loaded: true });
      this.resetConnection();
      return true;
    } catch (error) {
      if (this.active && lifetime === this.lifetime && sequence === this.readSequence)
        this.emit({ error: safeChannelError(error, 'channel_read_failed') });
      return false;
    } finally {
      if (this.active && lifetime === this.lifetime && sequence === this.readSequence)
        this.emit({ loading: false });
    }
  }
  viewChannel(id: string) {
    if (!this.active || !this.state.drafts[id] || id === this.state.selectedId) return;
    this.emit({ selectedId: id, error: null });
    this.resetConnection();
  }
  create(spec: GenerationChannelSpec, template?: ChannelTemplate, id = crypto.randomUUID()) {
    if (this.locked || this.state.drafts[id]) return null;
    this.emit({
      drafts: {
        ...this.state.drafts,
        [id]: {
          id,
          baseRowVersion: 0,
          spec: structuredClone(spec),
          dirty: true,
          conflict: false,
          template,
        },
      },
      selectedId: id,
      error: null,
    });
    this.resetConnection();
    return id;
  }
  change(patch: Partial<GenerationChannelSpec>) {
    const current = this.selected;
    if (!this.active || !this.state.loaded || !current) return;
    const spec = { ...current.spec, ...structuredClone(patch) };
    const sameConnection =
      spec.provider === current.spec.provider &&
      spec.endpoint === current.spec.endpoint &&
      spec.preset === current.spec.preset &&
      equal(spec.capabilities, current.spec.capabilities);
    const saved = this.state.server?.channels.find(
      (channel) => channel.id === current.id && !channel.deleted,
    );
    const dirty = !saved || !equal(spec, saved.spec);
    this.emit({
      drafts: { ...this.state.drafts, [current.id]: { ...current, spec, dirty } },
      error: null,
    });
    this.resetConnection(sameConnection);
  }
  changeModel(index: number, patch: Partial<GenerationChannelModel>) {
    const current = this.selected;
    if (!current?.spec.models[index]) return;
    this.change({
      models: current.spec.models.map((model, item) =>
        item === index ? { ...model, ...patch } : model,
      ),
    });
  }
  changeCloudflareAccount(accountInput: string) {
    if (!this.active || !this.selected) return;
    this.change({ endpoint: cloudflareEndpoint(accountInput) ?? '' });
    const current = this.selected;
    this.emit({ drafts: { ...this.state.drafts, [current.id]: { ...current, accountInput } } });
  }
  discardSelected() {
    if (this.locked || !this.selected) return;
    const id = this.selected.id,
      drafts = { ...this.state.drafts };
    const saved = this.state.server?.channels.find(
      (channel) => channel.id === id && !channel.deleted,
    );
    if (saved) drafts[id] = entry(saved);
    else delete drafts[id];
    this.emit({ drafts, selectedId: saved ? id : (Object.keys(drafts)[0] ?? null), error: null });
    this.resetConnection();
  }
  copySelected(name: string) {
    return this.selected
      ? this.create({ ...structuredClone(this.selected.spec), name }, this.selected.template)
      : null;
  }
  private acceptSettings(settings: GenerationSettings) {
    if (this.state.server && settings.row_version >= this.state.server.settings.row_version)
      this.emit({ server: { ...this.state.server, settings: structuredClone(settings) } });
  }
  confirmSettings(settings: GenerationSettings) {
    if (this.active) this.acceptSettings(settings);
  }
  confirmMutation(result: GenerationChannelMutation, submitted?: GenerationChannelSpec) {
    if (this.active) this.acceptMutation(result, submitted);
  }
  private acceptMutation(result: GenerationChannelMutation, submitted?: GenerationChannelSpec) {
    if (!this.state.server) return;
    const channel = result.channel;
    const known = this.state.server.channels.find((item) => item.id === channel.id);
    if (!known || channel.row_version >= known.row_version) {
      const channels = this.state.server.channels.filter((item) => item.id !== channel.id);
      if (!channel.deleted) channels.push(structuredClone(channel));
      const drafts = { ...this.state.drafts },
        old = drafts[channel.id];
      if (channel.deleted) delete drafts[channel.id];
      else if (old && submitted && !equal(old.spec, submitted))
        drafts[channel.id] = {
          ...old,
          baseRowVersion: channel.row_version,
          dirty: !equal(old.spec, channel.spec),
          conflict: false,
        };
      else drafts[channel.id] = { ...entry(channel), template: old?.template };
      this.emit({
        server: { ...this.state.server, channels },
        drafts,
        selectedId:
          this.state.selectedId && drafts[this.state.selectedId]
            ? this.state.selectedId
            : (Object.keys(drafts)[0] ?? null),
      });
    }
    this.acceptSettings(result.settings);
    if (this.state.selectedId === channel.id || channel.deleted) this.resetConnection();
  }
  private async write<T>(
    kind: NonNullable<Snapshot['writing']>,
    operation: () => Promise<T | null>,
    accept: (result: T) => void,
  ): Promise<T | null> {
    if (this.locked) return null;
    const lifetime = this.lifetime;
    ++this.readSequence;
    this.emit({ writing: kind, error: null });
    try {
      const result = await operation();
      if (!this.active || lifetime !== this.lifetime) return null;
      if (result) accept(result);
      return result;
    } catch (error) {
      if (this.active && lifetime === this.lifetime)
        this.emit({ error: safeChannelError(error, 'channel_write_failed') });
      return null;
    } finally {
      if (this.active && lifetime === this.lifetime) this.emit({ writing: null });
    }
  }
  async save(write: (request: SaveGenerationChannel) => Promise<GenerationChannelMutation | null>) {
    if (!this.canSave || !this.selected || !this.state.server) return null;
    const draft = this.selected,
      spec = { ...structuredClone(draft.spec), name: draft.spec.name.trim() };
    const request: SaveGenerationChannel = {
      channel_id: draft.id,
      base_row_version: draft.baseRowVersion,
      base_generation_row_version: this.state.server.settings.row_version,
      spec,
    };
    return this.write(
      'save',
      () => write(request),
      (result) => this.acceptMutation(result, draft.spec),
    );
  }
  async remove(
    clearActive: boolean,
    write: (request: DeleteGenerationChannel) => Promise<GenerationChannelMutation | null>,
  ) {
    const channel = this.savedChannel;
    if (!channel || !this.state.server) return null;
    if (this.state.server.settings.profile?.channel_id === channel.id && !clearActive) {
      this.emit({ error: 'channel_active_delete' });
      return null;
    }
    const request: DeleteGenerationChannel = {
      channel_id: channel.id,
      base_row_version: channel.row_version,
      base_generation_row_version: this.state.server.settings.row_version,
      clear_active: clearActive,
    };
    return this.write(
      'delete',
      () => write(request),
      (result) => this.acceptMutation(result),
    );
  }
  async selectModel(
    model: string,
    write: (request: SelectGenerationChannelModel) => Promise<GenerationSettingsMutation | null>,
  ) {
    const channel = this.savedChannel;
    if (!channel?.spec.models.some((item) => item.model === model) || !this.state.server)
      return null;
    const request: SelectGenerationChannelModel = {
      channel_id: channel.id,
      model,
      base_row_version: channel.row_version,
      base_generation_row_version: this.state.server.settings.row_version,
    };
    return this.write(
      'select',
      () => write(request),
      (result) => this.acceptSettings(result.settings),
    );
  }
  async disable(write: (baseRowVersion: number) => Promise<GenerationSettingsMutation | null>) {
    if (!this.state.server?.settings.profile) return null;
    const version = this.state.server.settings.row_version;
    return this.write(
      'select',
      () => write(version),
      (result) => this.acceptSettings(result.settings),
    );
  }
  private connectionCurrent(channel: GenerationChannel, sequence: number, lifetime: number) {
    return (
      this.active &&
      this.lifetime === lifetime &&
      sequence === this.connectionSequence &&
      this.savedChannel?.id === channel.id &&
      this.savedChannel.row_version === channel.row_version
    );
  }
  async loadModels(read: (channel: GenerationChannel) => Promise<RemoteGenerationModels>) {
    const channel = this.savedChannel;
    if (this.locked || !channel || this.state.listing || this.state.credentialConfigured !== true)
      return;
    const sequence = this.connectionSequence,
      lifetime = this.lifetime,
      ticket = ++this.catalogSequence;
    const current = () =>
      ticket === this.catalogSequence && this.connectionCurrent(channel, sequence, lifetime);
    this.emit({ listing: true, catalog: null, catalogError: null });
    try {
      const catalog = await read(structuredClone(channel));
      if (!current()) return;
      if (
        catalog.channel_id !== channel.id ||
        catalog.row_version !== channel.row_version ||
        catalog.provider !== channel.spec.provider ||
        catalog.endpoint !== channel.spec.endpoint
      ) {
        this.emit({ catalogError: 'channel_catalog_mismatch' });
        return;
      }
      this.emit({ catalog: structuredClone(catalog) });
    } catch (error) {
      if (current()) this.emit({ catalogError: safeChannelError(error, 'channel_catalog_failed') });
    } finally {
      if (current()) this.emit({ listing: false });
    }
  }
  async credentialOperation(operation: (channel: GenerationChannel) => Promise<boolean>) {
    const channel = this.savedChannel;
    if (this.locked || !channel || this.state.credentialBusy) return;
    const sequence = this.connectionSequence,
      lifetime = this.lifetime,
      ticket = ++this.credentialSequence;
    const current = () =>
      ticket === this.credentialSequence && this.connectionCurrent(channel, sequence, lifetime);
    ++this.verificationSequence;
    this.emit({
      credentialBusy: true,
      credentialConfigured: null,
      credentialError: null,
      verifiedModels: [],
      checkingModel: null,
    });
    try {
      const configured = await operation(structuredClone(channel));
      if (current()) this.emit({ credentialConfigured: configured });
    } catch (error) {
      if (current())
        this.emit({ credentialError: safeChannelError(error, 'channel_credential_failed') });
    } finally {
      if (current()) this.emit({ credentialBusy: false });
    }
  }
  async verify(model: string, check: (profile: GenerationProfile) => Promise<unknown>) {
    const channel = this.savedChannel,
      selected = channel?.spec.models.find((item) => item.model === model);
    if (
      this.locked ||
      !channel ||
      !selected ||
      this.state.checkingModel ||
      this.state.credentialConfigured !== true
    )
      return;
    const sequence = this.connectionSequence,
      lifetime = this.lifetime,
      ticket = ++this.verificationSequence;
    const current = () =>
      ticket === this.verificationSequence && this.connectionCurrent(channel, sequence, lifetime);
    const profile: GenerationProfile = {
      ...selected,
      provider: channel.spec.provider,
      endpoint: channel.spec.endpoint,
      model_digest: '',
      config_revision: Math.max(1, this.state.server?.settings.row_version ?? 1),
      channel_id: channel.id,
      capabilities: structuredClone(channel.spec.capabilities),
    };
    this.emit({ checkingModel: model, verificationError: null });
    try {
      await check(profile);
      if (current())
        this.emit({ verifiedModels: [...new Set([...this.state.verifiedModels, model])] });
    } catch (error) {
      if (current())
        this.emit({ verificationError: safeChannelError(error, 'channel_verification_failed') });
    } finally {
      if (current()) this.emit({ checkingModel: null });
    }
  }
}
