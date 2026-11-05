import type { DocumentNode, NoteDocument } from '@pkms/contracts';
import { isMap, parseDocument, stringify } from 'yaml';

export type ObjectTypeIconName =
  | 'note'
  | 'character'
  | 'person'
  | 'book'
  | 'film'
  | 'place'
  | 'event'
  | 'project'
  | 'idea'
  | 'organization'
  | 'meeting'
  | 'custom';

export type ObjectFieldKind = 'text' | 'date' | 'number' | 'select' | 'relation';

export interface ObjectFieldOption {
  value: string;
  zh: string;
  en: string;
}

export interface ObjectFieldDefinition {
  key: string;
  labelZh: string;
  labelEn: string;
  kind: ObjectFieldKind;
  placeholderZh?: string;
  placeholderEn?: string;
  defaultValue?: string;
  options?: ObjectFieldOption[];
  /** Canonical object type IDs. Empty or absent allows every note. */
  allowedTypeIds?: string[];
}

export interface ObjectRelationValue {
  kind: 'note';
  id: string;
  title: string;
}

export interface ObjectRelationState {
  values: ObjectRelationValue[];
  error: string | null;
}

export interface ObjectTypeDefinition {
  id: string;
  labelZh: string;
  labelEn: string;
  pluralZh: string;
  pluralEn: string;
  descriptionZh: string;
  descriptionEn: string;
  icon: ObjectTypeIconName;
  tone: 'neutral' | 'amber' | 'blue' | 'rose' | 'violet' | 'teal' | 'green';
  aliases: string[];
  fields: ObjectFieldDefinition[];
  sections: Array<{ zh: string; en: string }>;
}

const summary: ObjectFieldDefinition = {
  key: 'summary',
  labelZh: '摘要',
  labelEn: 'Summary',
  kind: 'text',
  placeholderZh: '用一句话说明它是什么',
  placeholderEn: 'Describe this object in one line',
};

const status = (options: ObjectFieldOption[]): ObjectFieldDefinition => ({
  key: 'status',
  labelZh: '状态',
  labelEn: 'Status',
  kind: 'select',
  options,
});

const planned: ObjectFieldOption[] = [
  { value: 'idea', zh: '初步想法', en: 'Idea' },
  { value: 'planned', zh: '已计划', en: 'Planned' },
  { value: 'active', zh: '进行中', en: 'In progress' },
  { value: 'complete', zh: '已完成', en: 'Complete' },
];

const reading: ObjectFieldOption[] = [
  { value: 'want', zh: '想读', en: 'Want to read' },
  { value: 'reading', zh: '在读', en: 'Reading' },
  { value: 'finished', zh: '已读完', en: 'Finished' },
  { value: 'paused', zh: '搁置', en: 'Paused' },
];

const watching: ObjectFieldOption[] = [
  { value: 'want', zh: '想看', en: 'Want to watch' },
  { value: 'watching', zh: '在看', en: 'Watching' },
  { value: 'finished', zh: '已看完', en: 'Finished' },
  { value: 'paused', zh: '搁置', en: 'Paused' },
];

export const OBJECT_TYPES: ObjectTypeDefinition[] = [
  {
    id: 'note',
    labelZh: '笔记',
    labelEn: 'Note',
    pluralZh: '笔记',
    pluralEn: 'Notes',
    descriptionZh: '自由记录想法、资料和过程。',
    descriptionEn: 'Capture thoughts, references, and working notes.',
    icon: 'note',
    tone: 'neutral',
    aliases: ['note', 'notes', '笔记'],
    fields: [summary],
    sections: [],
  },
  {
    id: 'character',
    labelZh: '角色',
    labelEn: 'Character',
    pluralZh: '角色',
    pluralEn: 'Characters',
    descriptionZh: '整理人物设定、关系与故事线索。',
    descriptionEn: 'Organize characters, relationships, and story threads.',
    icon: 'character',
    tone: 'rose',
    aliases: ['character', 'characters', '角色', '人物角色'],
    fields: [
      summary,
      {
        key: 'universe',
        labelZh: '所属世界',
        labelEn: 'Universe',
        kind: 'text',
        placeholderZh: '故事、作品或世界观',
        placeholderEn: 'Story, work, or universe',
      },
      {
        key: 'role',
        labelZh: '故事角色',
        labelEn: 'Story role',
        kind: 'text',
        placeholderZh: '主角、对手、导师…',
        placeholderEn: 'Protagonist, antagonist, mentor…',
      },
      status(planned),
    ],
    sections: [
      { zh: '概览', en: 'Overview' },
      { zh: '动机与冲突', en: 'Motivation and conflict' },
      { zh: '关系', en: 'Relationships' },
      { zh: '出场与线索', en: 'Appearances and threads' },
    ],
  },
  {
    id: 'person',
    labelZh: '人物',
    labelEn: 'Person',
    pluralZh: '人物',
    pluralEn: 'People',
    descriptionZh: '记录认识的人、合作者和重要关系。',
    descriptionEn: 'Keep context on people, collaborators, and relationships.',
    icon: 'person',
    tone: 'amber',
    aliases: ['person', 'people', '人物', '联系人'],
    fields: [
      summary,
      {
        key: 'organization',
        labelZh: '组织',
        labelEn: 'Organization',
        kind: 'text',
        placeholderZh: '公司、学校或社群',
        placeholderEn: 'Company, school, or community',
      },
      {
        key: 'relationship',
        labelZh: '关系',
        labelEn: 'Relationship',
        kind: 'select',
        options: [
          { value: 'friend', zh: '朋友', en: 'Friend' },
          { value: 'family', zh: '家人', en: 'Family' },
          { value: 'colleague', zh: '同事', en: 'Colleague' },
          { value: 'mentor', zh: '导师', en: 'Mentor' },
          { value: 'other', zh: '其他', en: 'Other' },
        ],
      },
      { key: 'birthday', labelZh: '生日', labelEn: 'Birthday', kind: 'date' },
    ],
    sections: [
      { zh: '背景', en: 'Context' },
      { zh: '互动记录', en: 'Interactions' },
      { zh: '共同项目', en: 'Shared projects' },
    ],
  },
  {
    id: 'book',
    labelZh: '书籍',
    labelEn: 'Book',
    pluralZh: '书籍',
    pluralEn: 'Books',
    descriptionZh: '建立阅读库，保留作者、评分与阅读笔记。',
    descriptionEn: 'Build a reading library with authors, ratings, and notes.',
    icon: 'book',
    tone: 'violet',
    aliases: ['book', 'books', '书', '书籍', '图书'],
    fields: [
      summary,
      {
        key: 'author',
        labelZh: '作者',
        labelEn: 'Author',
        kind: 'text',
        placeholderZh: '作者姓名',
        placeholderEn: 'Author name',
      },
      { ...status(reading), defaultValue: 'want' },
      { key: 'rating', labelZh: '评分', labelEn: 'Rating', kind: 'number' },
      { key: 'finished_on', labelZh: '读完日期', labelEn: 'Finished on', kind: 'date' },
    ],
    sections: [
      { zh: '摘要', en: 'Summary' },
      { zh: '关键观点', en: 'Key ideas' },
      { zh: '引用', en: 'Quotes' },
      { zh: '笔记', en: 'Notes' },
    ],
  },
  {
    id: 'film',
    labelZh: '影视',
    labelEn: 'Film',
    pluralZh: '影视',
    pluralEn: 'Films',
    descriptionZh: '收藏电影与剧集，记录观看状态和感受。',
    descriptionEn: 'Collect films and series with viewing state and reactions.',
    icon: 'film',
    tone: 'blue',
    aliases: ['film', 'films', 'movie', 'movies', '影视', '电影', '剧集'],
    fields: [
      summary,
      {
        key: 'director',
        labelZh: '导演',
        labelEn: 'Director',
        kind: 'text',
        placeholderZh: '导演或主创',
        placeholderEn: 'Director or creator',
      },
      { ...status(watching), defaultValue: 'want' },
      { key: 'rating', labelZh: '评分', labelEn: 'Rating', kind: 'number' },
      { key: 'released_on', labelZh: '上映日期', labelEn: 'Released on', kind: 'date' },
    ],
    sections: [
      { zh: '简介', en: 'Overview' },
      { zh: '观看笔记', en: 'Viewing notes' },
      { zh: '引用与灵感', en: 'Quotes and inspiration' },
    ],
  },
  {
    id: 'place',
    labelZh: '地点',
    labelEn: 'Place',
    pluralZh: '地点',
    pluralEn: 'Places',
    descriptionZh: '保存城市、场所与旅程中的空间线索。',
    descriptionEn: 'Save cities, venues, and places from your journeys.',
    icon: 'place',
    tone: 'teal',
    aliases: ['place', 'places', 'location', 'locations', '地点', '场所'],
    fields: [
      summary,
      {
        key: 'region',
        labelZh: '地区',
        labelEn: 'Region',
        kind: 'text',
        placeholderZh: '城市、国家或区域',
        placeholderEn: 'City, country, or region',
      },
      {
        key: 'category',
        labelZh: '类别',
        labelEn: 'Category',
        kind: 'text',
        placeholderZh: '咖啡馆、公园、博物馆…',
        placeholderEn: 'Cafe, park, museum…',
      },
      { key: 'visited_on', labelZh: '到访日期', labelEn: 'Visited on', kind: 'date' },
    ],
    sections: [
      { zh: '到达方式', en: 'Getting there' },
      { zh: '现场笔记', en: 'Field notes' },
      { zh: '相关人物与资料', en: 'Related people and sources' },
    ],
  },
  {
    id: 'event',
    labelZh: '事件',
    labelEn: 'Event',
    pluralZh: '事件',
    pluralEn: 'Events',
    descriptionZh: '整理会议、活动、里程碑与时间脉络。',
    descriptionEn: 'Organize meetings, activities, milestones, and timelines.',
    icon: 'event',
    tone: 'amber',
    aliases: ['event', 'events', '事件', '活动'],
    fields: [
      summary,
      { key: 'starts_on', labelZh: '日期', labelEn: 'Date', kind: 'date' },
      {
        key: 'place',
        labelZh: '地点',
        labelEn: 'Place',
        kind: 'text',
        placeholderZh: '线下地点或线上房间',
        placeholderEn: 'Venue or online room',
      },
      status(planned),
    ],
    sections: [
      { zh: '议程', en: 'Agenda' },
      { zh: '参与记录', en: 'Attendance' },
      { zh: '决定与后续', en: 'Decisions and follow-ups' },
    ],
  },
  {
    id: 'project',
    labelZh: '项目',
    labelEn: 'Project',
    pluralZh: '项目',
    pluralEn: 'Projects',
    descriptionZh: '追踪目标、阶段、协作与交付结果。',
    descriptionEn: 'Track goals, stages, collaborators, and outcomes.',
    icon: 'project',
    tone: 'green',
    aliases: ['project', 'projects', '项目', '计划'],
    fields: [
      summary,
      {
        key: 'area',
        labelZh: '领域',
        labelEn: 'Area',
        kind: 'text',
        placeholderZh: '工作、学习、生活…',
        placeholderEn: 'Work, study, life…',
      },
      status(planned),
      { key: 'started_on', labelZh: '开始日期', labelEn: 'Started on', kind: 'date' },
      { key: 'due_on', labelZh: '截止日期', labelEn: 'Due on', kind: 'date' },
    ],
    sections: [
      { zh: '目标', en: 'Goal' },
      { zh: '里程碑', en: 'Milestones' },
      { zh: '协作与资料', en: 'Collaboration and sources' },
      { zh: '复盘', en: 'Retrospective' },
    ],
  },
  {
    id: 'idea',
    labelZh: '想法',
    labelEn: 'Idea',
    pluralZh: '想法',
    pluralEn: 'Ideas',
    descriptionZh: '把灵感、假设和待验证方向收集到一处。',
    descriptionEn: 'Collect sparks, hypotheses, and directions to validate.',
    icon: 'idea',
    tone: 'amber',
    aliases: ['idea', 'ideas', '想法', '灵感'],
    fields: [
      summary,
      status(planned),
      {
        key: 'source',
        labelZh: '来源',
        labelEn: 'Source',
        kind: 'text',
        placeholderZh: '对话、阅读或观察',
        placeholderEn: 'Conversation, reading, or observation',
      },
    ],
    sections: [
      { zh: '为什么重要', en: 'Why it matters' },
      { zh: '下一步', en: 'Next step' },
      { zh: '相关证据', en: 'Related evidence' },
    ],
  },
  {
    id: 'organization',
    labelZh: '组织',
    labelEn: 'Organization',
    pluralZh: '组织',
    pluralEn: 'Organizations',
    descriptionZh: '记录公司、学校、社群与相关人物。',
    descriptionEn: 'Keep context on companies, schools, communities, and people.',
    icon: 'organization',
    tone: 'blue',
    aliases: ['organization', 'organizations', 'organisation', '组织', '机构'],
    fields: [
      summary,
      {
        key: 'category',
        labelZh: '类别',
        labelEn: 'Category',
        kind: 'text',
        placeholderZh: '公司、学校、非营利组织…',
        placeholderEn: 'Company, school, nonprofit…',
      },
      {
        key: 'website',
        labelZh: '网站',
        labelEn: 'Website',
        kind: 'text',
        placeholderZh: 'https://…',
        placeholderEn: 'https://…',
      },
    ],
    sections: [
      { zh: '概况', en: 'Overview' },
      { zh: '相关人物', en: 'People' },
      { zh: '相关项目', en: 'Projects' },
    ],
  },
  {
    id: 'meeting',
    labelZh: '会面',
    labelEn: 'Meeting',
    pluralZh: '会面',
    pluralEn: 'Meetings',
    descriptionZh: '记录一次对话的参与者、议题与行动项。',
    descriptionEn: 'Capture attendees, topics, and actions from a conversation.',
    icon: 'meeting',
    tone: 'rose',
    aliases: ['meeting', 'meetings', '会面', '会议'],
    fields: [
      summary,
      { key: 'starts_on', labelZh: '日期', labelEn: 'Date', kind: 'date' },
      {
        key: 'people',
        labelZh: '参与者',
        labelEn: 'Attendees',
        kind: 'text',
        placeholderZh: '用逗号分隔参与者',
        placeholderEn: 'Comma-separated attendees',
      },
      {
        key: 'place',
        labelZh: '地点',
        labelEn: 'Place',
        kind: 'text',
        placeholderZh: '地点或线上房间',
        placeholderEn: 'Venue or online room',
      },
    ],
    sections: [
      { zh: '议题', en: 'Topics' },
      { zh: '决定', en: 'Decisions' },
      { zh: '行动项', en: 'Action items' },
    ],
  },
];

export const CUSTOM_OBJECT_TYPE_SCHEMA = 1;
const OBJECT_TYPE_ID = /^[a-z][a-z0-9_-]{1,47}$/;
const OBJECT_TYPE_KEY = /^[A-Za-z_][A-Za-z0-9_-]*$/;
const RELATION_NOTE_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const MAX_RELATIONS = 12;
const OBJECT_TYPE_TONES = new Set<ObjectTypeDefinition['tone']>([
  'neutral',
  'amber',
  'blue',
  'rose',
  'violet',
  'teal',
  'green',
]);

export interface ObjectTypeRegistryStore {
  schema: 1;
  workspaces: Record<string, unknown>;
}

export interface ObjectTypeRegistryRead {
  store: ObjectTypeRegistryStore;
  types: ObjectTypeDefinition[];
  error: string | null;
}

function emptyObjectTypeRegistry(): ObjectTypeRegistryStore {
  return { schema: CUSTOM_OBJECT_TYPE_SCHEMA, workspaces: {} };
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

function optionalText(value: unknown, field: string, max: number): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') throw new Error(`${field} must be text.`);
  const text = value.trim();
  if (!text) return undefined;
  if (text.length > max) throw new Error(`${field} exceeds ${max} characters.`);
  return text;
}

function validateObjectField(value: unknown, index: number): ObjectFieldDefinition {
  const raw = recordValue(value);
  if (!raw) throw new Error(`Field ${index + 1} must be an object.`);
  const key = requiredText(raw.key, `Field ${index + 1} key`, 64);
  if (!OBJECT_TYPE_KEY.test(key)) throw new Error(`Field key "${key}" is invalid.`);
  const kind = requiredText(raw.kind, `Field ${index + 1} kind`, 20);
  if (!['text', 'date', 'number', 'select', 'relation'].includes(kind))
    throw new Error(`Field "${key}" has an unsupported kind.`);
  const field: ObjectFieldDefinition = {
    key,
    labelZh: requiredText(raw.labelZh, `Field ${key} Chinese label`, 80),
    labelEn: requiredText(raw.labelEn, `Field ${key} English label`, 80),
    kind: kind as ObjectFieldKind,
  };
  const placeholderZh = optionalText(raw.placeholderZh, `Field ${key} Chinese placeholder`, 160);
  const placeholderEn = optionalText(raw.placeholderEn, `Field ${key} English placeholder`, 160);
  if (placeholderZh) field.placeholderZh = placeholderZh;
  if (placeholderEn) field.placeholderEn = placeholderEn;
  const defaultValue = optionalText(raw.defaultValue, `Field ${key} default`, 160);
  if (kind === 'relation' && defaultValue)
    throw new Error(`Relation field "${key}" cannot have a default value.`);
  if (defaultValue) field.defaultValue = defaultValue;
  if (kind === 'relation' && raw.options !== undefined)
    throw new Error(`Relation field "${key}" cannot have options.`);
  if (kind !== 'relation' && raw.allowedTypeIds !== undefined)
    throw new Error(`Only relation fields can restrict object types.`);
  if (kind === 'relation' && raw.allowedTypeIds !== undefined) {
    if (!Array.isArray(raw.allowedTypeIds) || raw.allowedTypeIds.length > 32)
      throw new Error(
        `Relation field "${key}" allowed types must be an array of at most 32 values.`,
      );
    const seen = new Set<string>();
    field.allowedTypeIds = raw.allowedTypeIds.map((candidate, typeIndex) => {
      const value = requiredText(
        candidate,
        `Relation field "${key}" allowed type ${typeIndex + 1}`,
        48,
      ).toLocaleLowerCase('en-US');
      if (!OBJECT_TYPE_ID.test(value) || seen.has(value))
        throw new Error(`Relation field "${key}" has an invalid or duplicate allowed type.`);
      seen.add(value);
      return value;
    });
    if (field.allowedTypeIds.length === 0) delete field.allowedTypeIds;
  }
  if (kind === 'select') {
    if (!Array.isArray(raw.options) || raw.options.length === 0 || raw.options.length > 50)
      throw new Error(`Select field "${key}" needs 1-50 options.`);
    const seen = new Set<string>();
    field.options = raw.options.map((option, optionIndex) => {
      const item = recordValue(option);
      if (!item) throw new Error(`Option ${optionIndex + 1} for "${key}" must be an object.`);
      const value = requiredText(item.value, `Option ${optionIndex + 1} value`, 80);
      if (seen.has(value)) throw new Error(`Select field "${key}" has duplicate options.`);
      seen.add(value);
      return {
        value,
        zh: requiredText(item.zh, `Option ${value} Chinese label`, 80),
        en: requiredText(item.en, `Option ${value} English label`, 80),
      };
    });
  }
  return field;
}

function normalizedObjectTypeToken(value: string): string {
  return value.trim().toLocaleLowerCase('en-US');
}

function validateCustomObjectType(value: unknown, index: number): ObjectTypeDefinition {
  const raw = recordValue(value);
  if (!raw) throw new Error(`Custom type ${index + 1} must be an object.`);
  const id = requiredText(raw.id, `Custom type ${index + 1} id`, 48);
  if (!OBJECT_TYPE_ID.test(id))
    throw new Error(
      `Custom type id "${id}" is invalid. Use 2-48 lowercase letters, numbers, _ or -.`,
    );
  const tone = requiredText(raw.tone, `Custom type "${id}" tone`, 20);
  if (!OBJECT_TYPE_TONES.has(tone as ObjectTypeDefinition['tone']))
    throw new Error(`Custom type "${id}" has an unsupported tone.`);
  if (raw.icon !== 'custom') throw new Error(`Custom type "${id}" must use the custom icon.`);
  if (!Array.isArray(raw.aliases) || raw.aliases.length > 16)
    throw new Error(`Custom type "${id}" aliases must be an array of at most 16 values.`);
  const aliases = raw.aliases.map((alias, aliasIndex) =>
    requiredText(alias, `Custom type "${id}" alias ${aliasIndex + 1}`, 48),
  );
  if (!Array.isArray(raw.fields) || raw.fields.length > 20)
    throw new Error(`Custom type "${id}" must have at most 20 fields.`);
  const fields = raw.fields.map(validateObjectField);
  if (new Set(fields.map((field) => field.key.toLocaleLowerCase('en-US'))).size !== fields.length)
    throw new Error(`Custom type "${id}" has duplicate field keys.`);
  for (const field of fields) {
    if (
      field.kind === 'select' &&
      field.defaultValue &&
      !field.options?.some((option) => option.value === field.defaultValue)
    )
      throw new Error(`Field "${field.key}" default value is not one of its options.`);
  }
  if (!Array.isArray(raw.sections) || raw.sections.length > 8)
    throw new Error(`Custom type "${id}" sections must be an array of at most 8 items.`);
  const sections = raw.sections.map((section, sectionIndex) => {
    const item = recordValue(section);
    if (!item) throw new Error(`Custom type "${id}" section ${sectionIndex + 1} is invalid.`);
    return {
      zh: requiredText(item.zh, `Custom type "${id}" section Chinese label`, 100),
      en: requiredText(item.en, `Custom type "${id}" section English label`, 100),
    };
  });
  return {
    id,
    labelZh: requiredText(raw.labelZh, `Custom type "${id}" Chinese label`, 80),
    labelEn: requiredText(raw.labelEn, `Custom type "${id}" English label`, 80),
    pluralZh: requiredText(raw.pluralZh, `Custom type "${id}" Chinese plural`, 80),
    pluralEn: requiredText(raw.pluralEn, `Custom type "${id}" English plural`, 80),
    descriptionZh: requiredText(raw.descriptionZh, `Custom type "${id}" Chinese description`, 400),
    descriptionEn: requiredText(raw.descriptionEn, `Custom type "${id}" English description`, 400),
    icon: 'custom',
    tone: tone as ObjectTypeDefinition['tone'],
    aliases,
    fields,
    sections,
  };
}

function validateCustomObjectTypes(value: unknown): ObjectTypeDefinition[] {
  if (!Array.isArray(value) || value.length > 32)
    throw new Error('Workspace object types must be an array of at most 32 items.');
  const reserved = new Set(
    OBJECT_TYPES.flatMap((type) =>
      [type.id, ...type.aliases].map((token) => normalizedObjectTypeToken(token)),
    ),
  );
  const seenIds = new Set<string>();
  const seenAliases = new Set<string>();
  return value.map((candidate, index) => {
    const type = validateCustomObjectType(candidate, index);
    const idToken = normalizedObjectTypeToken(type.id);
    if (reserved.has(idToken) || seenIds.has(idToken) || seenAliases.has(idToken))
      throw new Error(`Custom type id "${type.id}" is already reserved or in use.`);
    const ownAliases = new Set<string>();
    for (const alias of type.aliases) {
      const aliasToken = normalizedObjectTypeToken(alias);
      if (!aliasToken) throw new Error(`Custom type "${type.id}" has an empty alias.`);
      if (ownAliases.has(aliasToken))
        throw new Error(`Custom type "${type.id}" has duplicate alias "${alias}".`);
      if (
        reserved.has(aliasToken) ||
        seenIds.has(aliasToken) ||
        aliasToken === idToken ||
        seenAliases.has(aliasToken)
      )
        throw new Error(`Custom type alias "${alias}" is already reserved or in use.`);
      ownAliases.add(aliasToken);
    }
    seenIds.add(idToken);
    for (const alias of ownAliases) seenAliases.add(alias);
    return type;
  });
}

function registrySize(json: string): number {
  return new TextEncoder().encode(json).length;
}
export function readObjectTypeRegistry(
  registryJson: string | null | undefined,
  workspaceId: string | null | undefined,
): ObjectTypeRegistryRead {
  const store = emptyObjectTypeRegistry();
  if (!registryJson?.trim()) return { store, types: [], error: null };
  try {
    const parsed = recordValue(JSON.parse(registryJson));
    if (!parsed) throw new Error('Object type registry must be a JSON object.');
    if (parsed.schema !== CUSTOM_OBJECT_TYPE_SCHEMA)
      throw new Error('Object type registry schema is unsupported.');
    const workspaces = recordValue(parsed.workspaces);
    if (!workspaces) throw new Error('Object type registry workspaces must be an object.');
    store.workspaces = { ...workspaces };
    if (!workspaceId) return { store, types: [], error: null };
    const rawTypes = workspaces[workspaceId];
    if (rawTypes === undefined) return { store, types: [], error: null };
    const types = validateCustomObjectTypes(rawTypes);
    return { store, types, error: null };
  } catch (error) {
    return {
      store,
      types: [],
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

export function writeObjectTypeRegistry(
  registryJson: string | null | undefined,
  workspaceId: string,
  types: ObjectTypeDefinition[],
): string {
  if (!workspaceId.trim()) throw new Error('A workspace is required for custom object types.');
  const current = readObjectTypeRegistry(registryJson, workspaceId);
  if (current.error) throw new Error(current.error);
  const candidate: ObjectTypeRegistryStore = {
    schema: CUSTOM_OBJECT_TYPE_SCHEMA,
    workspaces: { ...current.store.workspaces, [workspaceId]: types },
  };
  const next = JSON.stringify(candidate);
  if (registrySize(next) > 65_536)
    throw new Error('Custom object types exceed the 65,536-byte registry limit.');
  const checked = readObjectTypeRegistry(next, workspaceId);
  if (checked.error) throw new Error(checked.error);
  const normalized: ObjectTypeRegistryStore = {
    schema: CUSTOM_OBJECT_TYPE_SCHEMA,
    workspaces: { ...current.store.workspaces, [workspaceId]: checked.types },
  };
  return JSON.stringify(normalized);
}

export function objectTypeCatalog(customTypes: readonly ObjectTypeDefinition[] = []) {
  return [...OBJECT_TYPES, ...customTypes];
}

export function objectTypeQueryTokens(definition: ObjectTypeDefinition): string[] {
  const tokens = [
    definition.id,
    ...definition.aliases,
    ...(definition.id === 'note' ? ['__untyped'] : []),
  ].map((value) => value.trim().toLocaleLowerCase('en-US'));
  return [...new Set(tokens)].filter((value) => {
    if (value === '__untyped') return true;
    if (!value || value.length > 48) return false;
    return /^[\p{L}\p{N}][\p{L}\p{N}_-]*$/u.test(value);
  });
}

export function objectTypeDefinition(
  id: string | null | undefined,
  customTypes: readonly ObjectTypeDefinition[] = [],
): ObjectTypeDefinition {
  const catalog = objectTypeCatalog(customTypes);
  const value = id?.trim();
  if (!value) return catalog.find((type) => type.id === 'note')!;
  const normalized = value.toLocaleLowerCase();
  const match = catalog.find(
    (type) =>
      type.id.toLocaleLowerCase() === normalized ||
      type.aliases.some((alias) => alias.toLocaleLowerCase() === normalized),
  );
  if (match) return match;
  return {
    id: 'custom:' + value,
    labelZh: value,
    labelEn: value,
    pluralZh: value,
    pluralEn: value,
    descriptionZh: '自定义对象类型，使用 YAML frontmatter 扩展属性。',
    descriptionEn: 'A custom object type extended through YAML frontmatter.',
    icon: 'custom',
    tone: 'neutral',
    aliases: [],
    fields: [summary],
    sections: [],
  };
}

export interface FrontmatterState {
  source: string;
  values: Record<string, unknown>;
  error: string | null;
  present: boolean;
}

export interface ObjectTypeState {
  definition: ObjectTypeDefinition;
  rawType: string | null;
  custom: boolean;
  error: string | null;
}

function frontmatterNode(document: NoteDocument): DocumentNode | undefined {
  const first = document.content[0];
  return first?.type === 'frontmatter' ? first : undefined;
}

export function readFrontmatter(document: NoteDocument): FrontmatterState {
  const node = frontmatterNode(document);
  if (!node) return { source: '', values: {}, error: null, present: false };
  const source = typeof node.attrs?.source === 'string' ? node.attrs.source : '';
  let parsed;
  try {
    parsed = parseDocument(source);
  } catch (error) {
    return {
      source,
      values: {},
      error: error instanceof Error ? error.message : String(error),
      present: true,
    };
  }
  if (parsed.errors.length) {
    return { source, values: {}, error: parsed.errors[0].message, present: true };
  }
  if (parsed.contents !== null && !isMap(parsed.contents)) {
    return {
      source,
      values: {},
      error: 'YAML frontmatter must contain a mapping of keys to values.',
      present: true,
    };
  }
  const value = parsed.toJS() as unknown;
  return {
    source,
    values:
      value && typeof value === 'object' && !Array.isArray(value)
        ? (value as Record<string, unknown>)
        : {},
    error: null,
    present: true,
  };
}

export function readObjectType(
  document: NoteDocument,
  customTypes: readonly ObjectTypeDefinition[] = [],
): ObjectTypeState {
  const frontmatter = readFrontmatter(document);
  if (frontmatter.error)
    return {
      definition: objectTypeDefinition('note', customTypes),
      rawType: null,
      custom: false,
      error: frontmatter.error,
    };
  const rawValue = frontmatter.values.type ?? frontmatter.values.object_type;
  const rawType =
    rawValue === null || rawValue === undefined ? null : String(rawValue).trim() || null;
  const definition = objectTypeDefinition(rawType, customTypes);
  const custom = !OBJECT_TYPES.some((type) => type.id === definition.id);
  return { definition, rawType, custom, error: null };
}

export function readFrontmatterText(document: NoteDocument, key: string): string {
  const value = readFrontmatter(document).values[key];
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function nodeId(): string {
  return crypto.randomUUID();
}

function setFrontmatterEntry(
  document: NoteDocument,
  key: string,
  value: unknown,
  present: boolean,
): NoteDocument {
  if (!/^[A-Za-z_][A-Za-z0-9_-]*$/.test(key))
    throw new Error('Frontmatter key must use letters, numbers, underscores, or hyphens.');
  const next = structuredClone(document);
  const index = next.content[0]?.type === 'frontmatter' ? 0 : -1;
  const existing =
    index >= 0 && typeof next.content[index].attrs?.source === 'string'
      ? next.content[index].attrs!.source!
      : '';
  const parsed = parseDocument(existing);
  if (parsed.errors.length) throw new Error(parsed.errors[0].message);
  if (parsed.contents !== null && !isMap(parsed.contents))
    throw new Error('YAML frontmatter must contain a mapping of keys to values.');
  if (present) parsed.set(key, value);
  else parsed.delete(key);
  const source = parsed.toString({ lineWidth: 0 }).trimEnd();
  if (index >= 0) {
    next.content[index] = {
      ...next.content[index],
      attrs: {
        ...next.content[index].attrs,
        id: next.content[index].attrs?.id ?? nodeId(),
        source,
      },
    };
  } else if (present) {
    next.content.unshift({ type: 'frontmatter', attrs: { id: nodeId(), source } });
  }
  return next;
}

export function setFrontmatterValue(
  document: NoteDocument,
  key: string,
  value: string,
): NoteDocument {
  const normalized = value.trim();
  return setFrontmatterEntry(document, key, value, Boolean(normalized));
}

export function readObjectRelations(document: NoteDocument, key: string): ObjectRelationState {
  const frontmatter = readFrontmatter(document);
  if (frontmatter.error) return { values: [], error: frontmatter.error };
  const raw = frontmatter.values[key];
  if (raw === undefined || raw === null) return { values: [], error: null };
  if (!Array.isArray(raw)) return { values: [], error: 'Relation value must be a YAML list.' };
  if (raw.length > MAX_RELATIONS)
    return { values: [], error: `A relation field can contain at most ${MAX_RELATIONS} notes.` };
  const seen = new Set<string>();
  const values: ObjectRelationValue[] = [];
  for (const [index, entry] of raw.entries()) {
    const item = recordValue(entry);
    if (!item || item.kind !== 'note')
      return { values: [], error: `Relation ${index + 1} must be a note reference.` };
    if (typeof item.id !== 'string' || !RELATION_NOTE_ID.test(item.id))
      return { values: [], error: `Relation ${index + 1} has an invalid note ID.` };
    if (typeof item.title !== 'string' || !item.title.trim() || item.title.trim().length > 300)
      return {
        values: [],
        error: `Relation ${index + 1} needs a title of at most 300 characters.`,
      };
    if (seen.has(item.id))
      return { values: [], error: `Relation field "${key}" contains a duplicate note.` };
    seen.add(item.id);
    values.push({ kind: 'note', id: item.id, title: item.title.trim() });
  }
  return { values, error: null };
}

export function setFrontmatterRelations(
  document: NoteDocument,
  key: string,
  relations: readonly ObjectRelationValue[],
): NoteDocument {
  if (relations.length > MAX_RELATIONS)
    throw new Error(`A relation field can contain at most ${MAX_RELATIONS} notes.`);
  const seen = new Set<string>();
  const values = relations.map((relation, index) => {
    const title = relation.title.trim();
    if (relation.kind !== 'note' || !RELATION_NOTE_ID.test(relation.id))
      throw new Error(`Relation ${index + 1} has an invalid note ID.`);
    if (!title || title.length > 300)
      throw new Error(`Relation ${index + 1} needs a title of at most 300 characters.`);
    if (seen.has(relation.id)) throw new Error('Relation fields cannot repeat a note.');
    seen.add(relation.id);
    return { kind: 'note' as const, id: relation.id, title };
  });
  return setFrontmatterEntry(document, key, values, values.length > 0);
}

export function relationAllowedTypeTokens(
  field: ObjectFieldDefinition,
  customTypes: readonly ObjectTypeDefinition[],
): string[] {
  if (!field.allowedTypeIds?.length) return [];
  const tokens = field.allowedTypeIds.flatMap((id) => {
    const definition = objectTypeDefinition(id, customTypes);
    if (definition.id.startsWith('custom:')) return [id.trim().toLocaleLowerCase('en-US')];
    return objectTypeQueryTokens(definition);
  });
  return [...new Set(tokens)];
}

export function setObjectType(document: NoteDocument, id: string): NoteDocument {
  return setFrontmatterValue(document, 'type', id);
}

function templateBlock(type: 'heading' | 'paragraph', text: string, level?: number): DocumentNode {
  return {
    type,
    attrs: { id: nodeId(), ...(level ? { level } : {}) },
    ...(text ? { content: [{ type: 'text', text }] } : {}),
  };
}

export function createObjectTypeDocument(
  id: string | null | undefined,
  t: (zh: string, en: string) => string,
  customTypes: readonly ObjectTypeDefinition[] = [],
): NoteDocument {
  const definition = objectTypeDefinition(id, customTypes);
  if (!id || definition.id === 'note')
    return { type: 'doc', content: [templateBlock('paragraph', '')] };
  const values: Record<string, string> = { type: definition.id };
  for (const field of definition.fields)
    if (field.defaultValue) values[field.key] = field.defaultValue;
  const source = stringify(values, { lineWidth: 0 }).trimEnd();
  const content: DocumentNode[] = [
    { type: 'frontmatter', attrs: { id: nodeId(), source } },
    templateBlock('paragraph', ''),
  ];
  for (const section of definition.sections) {
    content.push(templateBlock('heading', t(section.zh, section.en), 2));
    content.push(templateBlock('paragraph', ''));
  }
  return { type: 'doc', content };
}

export function objectTypeDefaultTitle(
  id: string | null | undefined,
  t: (zh: string, en: string) => string,
  customTypes: readonly ObjectTypeDefinition[] = [],
): string {
  const definition = objectTypeDefinition(id, customTypes);
  return t(`未命名${definition.labelZh}`, `Untitled ${definition.labelEn.toLocaleLowerCase()}`);
}
