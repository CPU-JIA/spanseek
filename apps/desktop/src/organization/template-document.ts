import type { DocumentNode, NoteDocument } from '@pkms/contracts';
import { markdownDocument, type PreservedSyntax } from '../inference/markdown-document.ts';
import {
  createObjectTypeDocument,
  objectTypeCatalog,
  objectTypeDefaultTitle,
  setFrontmatterValue,
  type ObjectTypeDefinition,
} from './object-types.ts';
import { expandNoteTemplateText, type NoteTemplateDefinition } from './template-registry.ts';
function resolveType(
  typeId: string | null,
  customTypes: readonly ObjectTypeDefinition[],
): ObjectTypeDefinition | null {
  if (!typeId) return null;
  const normalized = typeId.trim().toLocaleLowerCase('en-US');
  const match = objectTypeCatalog(customTypes).find(
    (type) =>
      type.id.toLocaleLowerCase('en-US') === normalized ||
      type.aliases.some((alias) => alias.toLocaleLowerCase('en-US') === normalized),
  );
  if (!match) throw new Error(`Template object type "${typeId}" is unavailable.`);
  return match;
}

function blankParagraph(): DocumentNode {
  return { type: 'paragraph', attrs: { id: crypto.randomUUID() } };
}

export function createTemplatedNoteDocument(
  template: NoteTemplateDefinition,
  t: (zh: string, en: string) => string,
  language: 'zh-CN' | 'en',
  customTypes: readonly ObjectTypeDefinition[] = [],
  now = new Date(),
): { title: string; document: NoteDocument; preserved: PreservedSyntax[] } {
  const type = resolveType(template.typeId, customTypes);
  const typeLabel = type ? t(type.labelZh, type.labelEn) : t('笔记', 'Note');
  const base = type
    ? createObjectTypeDocument(type.id, t, customTypes)
    : { type: 'doc' as const, content: [blankParagraph()] };
  const frontmatter = base.content.find((node) => node.type === 'frontmatter');
  const bodySource =
    language === 'en' ? template.bodyEn || template.bodyZh : template.bodyZh || template.bodyEn;
  const expandedBody = expandNoteTemplateText(bodySource, { typeLabel, now });
  const parsed = expandedBody.trim()
    ? markdownDocument(expandedBody, { allowFrontmatter: false })
    : {
        document: {
          type: 'doc' as const,
          content: base.content.filter((node) => node.type !== 'frontmatter'),
        },
        preserved: [] as PreservedSyntax[],
      };
  const body = parsed.document.content.length ? parsed.document.content : [blankParagraph()];
  const document = setFrontmatterValue(
    {
      type: 'doc',
      content: frontmatter ? [frontmatter, ...body] : body,
    },
    'template',
    template.id,
  );
  const rawTitle =
    language === 'en' ? template.titleEn || template.titleZh : template.titleZh || template.titleEn;
  const title =
    expandNoteTemplateText(rawTitle, { typeLabel, now }) ||
    (type ? objectTypeDefaultTitle(type.id, t, customTypes) : t('未命名笔记', 'Untitled note'));
  return { title, document, preserved: parsed.preserved };
}
