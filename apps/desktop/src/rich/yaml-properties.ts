import { isAlias, isMap, isScalar, isSeq, parseDocument, stringify, type Node } from 'yaml';

export type PropertyScalar = string | number | boolean | null;
export type PropertyField = {
  key: string;
  path: (string | number)[];
  kind: 'text' | 'number' | 'boolean' | 'date' | 'datetime' | 'null' | 'list' | 'nested';
  value?: PropertyScalar;
  children?: PropertyField[];
};
const MAX_SOURCE = 64000;
function documentOf(source: string) {
  if (source.length > MAX_SOURCE) throw new Error('属性源码过长，请在源码中编辑。');
  const document = parseDocument(source, {
    keepSourceTokens: true,
    uniqueKeys: true,
    schema: 'core',
    strict: true,
  });
  if (document.errors.length || document.warnings.length)
    throw new Error('YAML 含语法、重复属性或不支持的标签，请检查保留的源码。');
  if (!isMap(document.contents)) throw new Error('属性需要是 YAML 键值表；原始内容已保留。');
  return document;
}
function fieldFor(key: string, path: (string | number)[], node: unknown): PropertyField {
  if (isSeq(node))
    return {
      key,
      path,
      kind: 'list',
      children: node.items
        .slice(0, 100)
        .map((item, index) =>
          isSeq(item) || isMap(item) || isAlias(item)
            ? { key: String(index + 1), path: [...path, index], kind: 'nested' }
            : fieldFor(String(index + 1), [...path, index], item),
        ),
    };
  if (!isScalar(node) || node.anchor || node.tag) return { key, path, kind: 'nested' };
  const value: unknown = node.value;
  if (value === null) return { key, path, kind: 'null', value: null };
  if (typeof value === 'boolean') return { key, path, kind: 'boolean', value };
  if (typeof value === 'number' && Number.isFinite(value))
    return { key, path, kind: 'number', value };
  if (typeof value !== 'string') return { key, path, kind: 'nested' };
  const kind = /^\d{4}-\d{2}-\d{2}$/.test(value)
    ? 'date'
    : /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?$/.test(value)
      ? 'datetime'
      : 'text';
  return { key, path, kind, value };
}
export function objectTypeValue(source: string): string | null | undefined {
  const parsed = yamlFields(source);
  if (parsed.error) return undefined;
  const field = parsed.fields.find((item) => item.key === 'type' || item.key === 'object_type');
  if (!field) return null;
  return typeof field.value === 'string' ? field.value : '';
}

/** Preserve legacy unknown values, but never let raw YAML introduce a new free-form type. */
export function objectTypeChangeAllowed(
  current: string,
  next: string,
  allowed: readonly string[],
): boolean {
  const before = objectTypeValue(current);
  const after = objectTypeValue(next);
  if (after === undefined || after === null || after === before || !after) return true;
  return allowed.includes(after);
}
export function yamlFields(source: string): {
  fields: PropertyField[];
  error: string | null;
  truncated: boolean;
} {
  try {
    const document = documentOf(source);
    const mapping = document.contents;
    if (!isMap(mapping)) throw new Error('属性不是键值表。');
    const fields = mapping.items.slice(0, 100).map(({ key, value }) => {
      if (!isScalar(key) || typeof key.value !== 'string')
        throw new Error('属性名称需要是文字，请在源码中编辑。');
      return fieldFor(key.value, [key.value], value);
    });
    return {
      fields,
      error: null,
      truncated:
        mapping.items.length > 100 ||
        fields.some(
          (field) =>
            field.kind === 'list' &&
            (document.getIn(field.path, true) as { items: unknown[] }).items.length > 100,
        ),
    };
  } catch (error) {
    return {
      fields: [],
      error: error instanceof Error ? error.message : '属性无法解析，源码仍保留。',
      truncated: false,
    };
  }
}

/** Replace one existing scalar token only. Other properties, nested YAML and comments stay byte-identical. */
export function replaceYamlScalar(
  source: string,
  path: (string | number)[],
  value: PropertyScalar,
): string {
  if (typeof value === 'number' && !Number.isFinite(value)) throw new Error('请输入有限的数字。');
  const document = documentOf(source);
  const node = document.getIn(path, true) as Node | undefined;
  if (!isScalar(node) || node.anchor || node.tag || !node.range)
    throw new Error('此属性包含引用或嵌套结构，请编辑保留的源码。');
  // Quoted strings avoid introducing new lines, tags, anchors or a different scalar type.
  let replacement = typeof value === 'string' ? JSON.stringify(value) : stringify(value).trimEnd();
  if (node.type === 'BLOCK_LITERAL' || node.type === 'BLOCK_FOLDED') {
    const previous = source.slice(node.range[0], node.range[1]);
    replacement += previous.split(/\r?\n/, 1)[0].match(/\s+#.*$/)?.[0] ?? '';
    replacement += previous.match(/\r?\n$/)?.[0] ?? '';
  }
  const next = source.slice(0, node.range[0]) + replacement + source.slice(node.range[1]);
  documentOf(next);
  return next;
}
