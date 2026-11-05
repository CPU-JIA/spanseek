import type {
  DesktopCommand,
  GenerationModels,
  GenerationProfile,
  GenerationSettings,
} from '@pkms/contracts';

const defaults = (): GenerationProfile => ({
  provider: 'openai_chat',
  endpoint: '',
  model: '',
  model_digest: '',
  config_revision: 1,
  temperature: 0,
  context_tokens: 65536,
});
type Snapshot = {
  settings: GenerationSettings | null;
  draft: GenerationProfile;
  loaded: boolean;
  loading: boolean;
  checking: boolean;
  saving: boolean;
  verified: string | null;
  error: unknown;
  catalog: GenerationModels | null;
  listing: boolean;
  catalogError: unknown;
  credentialConfigured: boolean | null;
  credentialBusy: boolean;
  credentialError: unknown;
};

/** One workspace's draft: async results belong to the exact load/edit that started them. */
export class ModelDraft {
  private active = true;
  private epoch = 0;
  private verification = 0;
  private catalogSequence = 0;
  private credentialSequence = 0;
  private listeners = new Set<() => void>();
  private state: Snapshot = {
    settings: null,
    draft: defaults(),
    loaded: false,
    loading: true,
    checking: false,
    saving: false,
    verified: null,
    error: null,
    catalog: null,
    listing: false,
    catalogError: null,
    credentialConfigured: null,
    credentialBusy: false,
    credentialError: null,
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
  get locked() {
    return !this.active || !this.state.loaded || this.state.loading || this.state.saving;
  }
  get canSave() {
    return (
      !this.locked &&
      !this.state.checking &&
      !this.state.credentialBusy &&
      (this.state.draft.provider === 'ollama' || this.state.credentialConfigured === true) &&
      this.state.verified === JSON.stringify(this.state.draft)
    );
  }
  invalidate() {
    this.active = false;
    ++this.epoch;
    ++this.verification;
  }
  /** A deliberate channel action may update settings while a legacy form is open. */
  acceptChannelSettings(settings: GenerationSettings) {
    if (
      !this.active ||
      this.state.saving ||
      (this.state.settings && settings.row_version < this.state.settings.row_version)
    )
      return false;
    const unchanged =
      JSON.stringify(this.state.draft) ===
      JSON.stringify(this.state.settings?.profile ?? defaults());
    ++this.epoch;
    ++this.verification;
    ++this.catalogSequence;
    ++this.credentialSequence;
    this.emit({
      settings: structuredClone(settings),
      draft: unchanged ? structuredClone(settings.profile ?? defaults()) : this.state.draft,
      loaded: true,
      loading: false,
      checking: false,
      verified: null,
      error: null,
      catalog: null,
      listing: false,
      catalogError: null,
      credentialConfigured: null,
      credentialBusy: false,
      credentialError: null,
    });
    return true;
  }
  beginLegacyConfiguration() {
    if (this.locked) return;
    this.change({ ...defaults(), channel_id: undefined, capabilities: undefined });
  }
  async reloadAfterConfirmation(
    kind: DesktopCommand['kind'] | undefined,
    read: () => Promise<GenerationSettings>,
  ) {
    return kind === 'generation_settings_set' ? this.load(read) : false;
  }
  async load(read: () => Promise<GenerationSettings>) {
    if (this.state.saving) return false;
    this.active = true;
    const epoch = ++this.epoch;
    ++this.verification;
    this.emit({
      loading: true,
      loaded: false,
      checking: false,
      verified: null,
      error: null,
      catalog: null,
      listing: false,
      catalogError: null,
      credentialConfigured: null,
      credentialBusy: false,
      credentialError: null,
    });
    try {
      const settings = await read();
      if (epoch !== this.epoch) return false;
      this.emit({ settings, draft: structuredClone(settings.profile ?? defaults()), loaded: true });
      return true;
    } catch (error) {
      if (epoch === this.epoch) this.emit({ error });
      return false;
    } finally {
      if (epoch === this.epoch) this.emit({ loading: false });
    }
  }
  change(patch: Partial<GenerationProfile>) {
    if (this.locked) return;
    ++this.verification;
    const endpointChanged =
      (patch.endpoint !== undefined && patch.endpoint !== this.state.draft.endpoint) ||
      (patch.provider !== undefined && patch.provider !== this.state.draft.provider);
    const credentialChanged =
      endpointChanged || (patch.model !== undefined && patch.model !== this.state.draft.model);
    if (endpointChanged) ++this.catalogSequence;
    if (credentialChanged) ++this.credentialSequence;
    const draft = { ...this.state.draft, ...patch };
    if (draft.provider !== 'openai_chat' || draft.chat_token_limit == null) {
      delete draft.chat_token_limit;
    }
    this.emit({
      draft,
      verified: null,
      checking: false,
      error: null,
      ...(endpointChanged ? { catalog: null, listing: false, catalogError: null } : {}),
      ...(credentialChanged
        ? { credentialConfigured: null, credentialBusy: false, credentialError: null }
        : {}),
    });
  }
  async loadModels(read: (endpoint: string) => Promise<GenerationModels>) {
    if (this.locked || this.state.listing || this.state.draft.provider !== 'ollama') return;
    const endpoint = this.state.draft.endpoint;
    const epoch = this.epoch;
    const sequence = ++this.catalogSequence;
    const current = () =>
      epoch === this.epoch &&
      sequence === this.catalogSequence &&
      endpoint === this.state.draft.endpoint;
    ++this.verification;
    this.emit({
      listing: true,
      catalog: null,
      catalogError: null,
      error: null,
      checking: false,
      verified: null,
    });
    try {
      const catalog = await read(endpoint);
      if (current()) this.emit({ catalog });
    } catch (catalogError) {
      if (current()) this.emit({ catalogError });
    } finally {
      if (current()) this.emit({ listing: false });
    }
  }
  get selectedModel() {
    if (this.state.draft.provider !== 'ollama') return null;
    const digest = this.state.draft.model_digest.replace(/^sha256:/, '');
    return (
      this.state.catalog?.models.find(
        (item) =>
          item.name === this.state.draft.model && item.digest.replace(/^sha256:/, '') === digest,
      ) ?? null
    );
  }
  selectModel(name: string, digest: string) {
    if (this.locked || this.state.listing || this.state.draft.provider !== 'ollama') return false;
    const item = this.state.catalog?.models.find(
      (value) => value.name === name && value.digest === digest,
    );
    if (!item?.supported || !item.profile) return false;
    this.change({
      ...item.profile,
      endpoint: this.state.draft.endpoint,
      config_revision: this.state.draft.config_revision,
    });
    return true;
  }
  async verify(check: (profile: GenerationProfile) => Promise<unknown>) {
    if (
      this.locked ||
      this.state.checking ||
      this.state.listing ||
      !this.state.draft.model ||
      !this.state.draft.endpoint ||
      this.state.credentialBusy ||
      (this.state.draft.provider === 'ollama'
        ? !this.state.draft.model_digest
        : this.state.credentialConfigured !== true)
    )
      return;
    const epoch = this.epoch;
    const verification = ++this.verification;
    const profile = structuredClone(this.state.draft);
    const current = () => epoch === this.epoch && verification === this.verification;
    this.emit({ checking: true, verified: null, error: null });
    try {
      await check(profile);
      if (current()) this.emit({ verified: JSON.stringify(profile) });
    } catch (error) {
      if (current()) this.emit({ error });
    } finally {
      if (current()) this.emit({ checking: false });
    }
  }
  /** Callbacks own secrets; neither snapshots nor durable settings ever contain a key. */
  async credentialOperation(operation: (profile: GenerationProfile) => Promise<boolean>) {
    if (this.locked || this.state.draft.provider === 'ollama') return;
    const epoch = this.epoch;
    const sequence = ++this.credentialSequence;
    const profile = structuredClone(this.state.draft);
    const current = () =>
      this.active && epoch === this.epoch && sequence === this.credentialSequence;
    ++this.verification;
    this.emit({
      credentialBusy: true,
      credentialError: null,
      credentialConfigured: null,
      verified: null,
      checking: false,
    });
    try {
      const configured = await operation(profile);
      if (current()) this.emit({ credentialConfigured: configured });
    } catch (error) {
      // Never retain arbitrary transport errors which could contain request credentials.
      const reason = (error as { details?: { details?: { reason?: string } } })?.details?.details
        ?.reason;
      const safeReason =
        reason &&
        [
          'model_key_missing',
          'model_invalid_key',
          'model_credential_unavailable',
          'model_credential_unsupported',
          'model_invalid_endpoint',
          'invalid_model_endpoint',
        ].includes(reason)
          ? reason
          : 'credential_operation_failed';
      if (current()) this.emit({ credentialError: safeReason });
    } finally {
      if (current()) this.emit({ credentialBusy: false });
    }
  }
  async save(
    profile: GenerationProfile | null,
    write: (request: {
      base_row_version: number;
      profile: GenerationProfile | null;
    }) => Promise<GenerationSettings | null>,
  ) {
    if (
      this.locked ||
      !this.state.settings ||
      (profile !== null && (!this.canSave || JSON.stringify(profile) !== this.state.verified))
    )
      return;
    const epoch = ++this.epoch;
    ++this.verification;
    const request = {
      base_row_version: this.state.settings.row_version,
      profile: structuredClone(profile),
    };
    this.emit({ saving: true, checking: false, verified: null, error: null, listing: false });
    try {
      const settings = await write(request);
      if (epoch === this.epoch && settings) {
        const draft = structuredClone(settings.profile ?? defaults());
        this.emit({
          settings,
          draft,
          catalog: draft.endpoint === this.state.draft.endpoint ? this.state.catalog : null,
        });
      }
    } catch (error) {
      if (epoch === this.epoch) this.emit({ error });
    } finally {
      if (epoch === this.epoch) this.emit({ saving: false });
    }
  }
}
