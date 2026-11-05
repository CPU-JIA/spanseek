export const NOTE_TEMPLATE_SCHEMA = 1;
const TEMPLATE_ID = /^[a-z][a-z0-9_-]{1,47}$/;
const TEMPLATE_TYPE_ID = /^[A-Za-z0-9][A-Za-z0-9:_-]{0,63}$/;
const MAX_BODY_CHARS = 20_000;

export interface NoteTemplateDefinition {
  id: string;
  nameZh: string;
  nameEn: string;
  descriptionZh: string;
  descriptionEn: string;
  typeId: string | null;
  titleZh: string;
  titleEn: string;
  bodyZh: string;
  bodyEn: string;
}

export interface NoteTemplateRegistryStore {
  schema: 1;
  workspaces: Record<string, unknown>;
}

export interface NoteTemplateRegistryRead {
  store: NoteTemplateRegistryStore;
  templates: NoteTemplateDefinition[];
  error: string | null;
}

function emptyStore(): NoteTemplateRegistryStore {
  return { schema: NOTE_TEMPLATE_SCHEMA, workspaces: {} };
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function requiredText(value: unknown, field: string, max: number): string {
  if (typeof value !== 'string' || !value.trim()) throw new Error(`${field} is required.`);
  const text = value.trim();
  if (text.length > max) throw new Error(`${field} exceeds ${max} characters.`);
  return text;
}

function optionalText(value: unknown, field: string, max: number): string {
  if (value === undefined || value === null) return '';
  if (typeof value !== 'string') throw new Error(`${field} must be text.`);
  const text = value.trim();
  if (text.length > max) throw new Error(`${field} exceeds ${max} characters.`);
  return text;
}

function validateTemplate(value: unknown, index: number): NoteTemplateDefinition {
  const raw = recordValue(value);
  if (!raw) throw new Error(`Template ${index + 1} must be an object.`);
  const id = requiredText(raw.id, `Template ${index + 1} id`, 48);
  if (!TEMPLATE_ID.test(id))
    throw new Error(`Template id "${id}" must use 2-48 lowercase letters, numbers, _ or -.`);
  const typeId = optionalText(raw.typeId, `Template "${id}" object type`, 64) || null;
  if (typeId && !TEMPLATE_TYPE_ID.test(typeId))
    throw new Error(`Template "${id}" references an invalid object type id.`);
  return {
    id,
    nameZh: requiredText(raw.nameZh, `Template "${id}" Chinese name`, 100),
    nameEn: requiredText(raw.nameEn, `Template "${id}" English name`, 100),
    descriptionZh: requiredText(raw.descriptionZh, `Template "${id}" Chinese description`, 400),
    descriptionEn: requiredText(raw.descriptionEn, `Template "${id}" English description`, 400),
    typeId,
    titleZh: optionalText(raw.titleZh, `Template "${id}" Chinese title`, 200),
    titleEn: optionalText(raw.titleEn, `Template "${id}" English title`, 200),
    bodyZh: optionalText(raw.bodyZh, `Template "${id}" Chinese body`, MAX_BODY_CHARS),
    bodyEn: optionalText(raw.bodyEn, `Template "${id}" English body`, MAX_BODY_CHARS),
  };
}

function validateTemplates(value: unknown): NoteTemplateDefinition[] {
  if (!Array.isArray(value) || value.length > 32)
    throw new Error('Workspace templates must be an array of at most 32 items.');
  const seen = new Set<string>();
  return value.map((candidate, index) => {
    const template = validateTemplate(candidate, index);
    if (seen.has(template.id)) throw new Error(`Duplicate template id "${template.id}".`);
    seen.add(template.id);
    return template;
  });
}

function registrySize(json: string): number {
  return new TextEncoder().encode(json).length;
}

export function readNoteTemplateRegistry(
  registryJson: string | null | undefined,
  workspaceId: string | null | undefined,
): NoteTemplateRegistryRead {
  const store = emptyStore();
  if (!registryJson?.trim()) return { store, templates: [], error: null };
  try {
    const parsed = recordValue(JSON.parse(registryJson));
    if (!parsed) throw new Error('Note template registry must be a JSON object.');
    if (parsed.schema !== NOTE_TEMPLATE_SCHEMA)
      throw new Error('Note template registry schema is unsupported.');
    const workspaces = recordValue(parsed.workspaces);
    if (!workspaces) throw new Error('Note template registry workspaces must be an object.');
    store.workspaces = { ...workspaces };
    if (!workspaceId) return { store, templates: [], error: null };
    const rawTemplates = workspaces[workspaceId];
    if (rawTemplates === undefined) return { store, templates: [], error: null };
    return { store, templates: validateTemplates(rawTemplates), error: null };
  } catch (error) {
    return {
      store,
      templates: [],
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

export function writeNoteTemplateRegistry(
  registryJson: string | null | undefined,
  workspaceId: string,
  templates: NoteTemplateDefinition[],
): string {
  if (!workspaceId.trim()) throw new Error('A workspace is required for note templates.');
  const current = readNoteTemplateRegistry(registryJson, workspaceId);
  if (current.error) throw new Error(current.error);
  const candidate: NoteTemplateRegistryStore = {
    schema: NOTE_TEMPLATE_SCHEMA,
    workspaces: { ...current.store.workspaces, [workspaceId]: templates },
  };
  const next = JSON.stringify(candidate);
  if (registrySize(next) > 65_536)
    throw new Error('Note templates exceed the 65,536-byte registry limit.');
  const checked = readNoteTemplateRegistry(next, workspaceId);
  if (checked.error) throw new Error(checked.error);
  return JSON.stringify({
    schema: NOTE_TEMPLATE_SCHEMA,
    workspaces: { ...current.store.workspaces, [workspaceId]: checked.templates },
  } satisfies NoteTemplateRegistryStore);
}

function localDate(now: Date): string {
  return `${now.getFullYear().toString().padStart(4, '0')}-${(now.getMonth() + 1)
    .toString()
    .padStart(2, '0')}-${now.getDate().toString().padStart(2, '0')}`;
}

function localTime(now: Date): string {
  return `${now.getHours().toString().padStart(2, '0')}:${now
    .getMinutes()
    .toString()
    .padStart(2, '0')}`;
}

export function expandNoteTemplateText(
  source: string,
  context: { typeLabel: string; now: Date },
): string {
  return source.replace(/\{\{\s*(date|time|type)\s*\}\}/gi, (match, token: string) => {
    switch (token.toLocaleLowerCase('en-US')) {
      case 'date':
        return localDate(context.now);
      case 'time':
        return localTime(context.now);
      case 'type':
        return context.typeLabel;
      default:
        return match;
    }
  });
}
