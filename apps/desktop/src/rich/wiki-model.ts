import type {
  DocumentNode,
  NodeAttributes,
  Note,
  NoteDocument,
  NotePage,
  ResolveLink,
  ResolvedLink,
  ResolvedSourceTarget,
  SourcePage,
  SourceSummary,
  SourceVersion,
} from '@pkms/contracts';

export const MAX_EMBED_DEPTH = 3;
export const MAX_WIKI_PAGES = 20;
export type WikiPort = {
  notes: (query: string, cursor: string | null) => Promise<NotePage>;
  sources: (query: string, cursor: string | null) => Promise<SourcePage>;
  note: (id: string, revision: string | null) => Promise<Note>;
  source: (id: string) => Promise<SourceSummary>;
  version: (id: string) => Promise<SourceVersion>;
  /** resh asks the backend again instead of accepting a remembered answer. */
  resolve: (target: ResolveLink, options?: { fresh?: boolean }) => Promise<ResolvedLink>;
};
export type WikiResult =
  | { state: 'available'; kind: 'note'; note: Note; target: ResolveLink; blocks: DocumentNode[] }
  | {
      state: 'available';
      kind: 'source';
      source: SourceSummary;
      version: SourceVersion;
      target: ResolveLink;
      sourceTarget: ResolvedSourceTarget;
    }
  | { state: 'available'; kind: 'image'; note: Note; hash: string; alt: string }
  | {
      state: 'missing' | 'ambiguous' | 'invalid' | 'anchor_missing' | 'trashed' | 'limit';
      message: string;
    };

function titleKey(value: string): string {
  return value.normalize('NFC').replace(/\.md$/i, '').toLowerCase();
}
export function wikiParts(value: string, currentTitle = ''): { path: string; fragment: string } {
  if (!value || value.length > 4096) throw new Error('引用目标为空或过长。');
  let decoded: string;
  try {
    decoded = decodeURIComponent(value.trim());
  } catch {
    throw new Error('引用目标的编码无效。');
  }
  if (/[\u0000-\u001f\u007f]|^[a-z][\w+.-]*:|^[\\/]/i.test(decoded))
    throw new Error('只能引用当前知识库中的笔记或资料。');
  const hash = decoded.indexOf('#');
  const name = (hash < 0 ? decoded : decoded.slice(0, hash)).replaceAll('\\', '/');
  const fragment = hash < 0 ? '' : decoded.slice(hash + 1);
  const segments =
    name.startsWith('./') || name.startsWith('../') ? currentTitle.split('/').slice(0, -1) : [];
  for (const part of name.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') {
      if (!segments.length) throw new Error('引用路径不能越出当前知识库。');
      segments.pop();
    } else segments.push(part);
  }
  return { path: segments.join('/'), fragment };
}

/** Comments do not become headings, summaries or searchable anchor text. */
export function richText(node: DocumentNode): string {
  if (node.type === 'comment' || node.type === 'commentBlock') return '';
  if (node.type === 'text') return node.text ?? '';
  if (node.type === 'hardBreak') return '\n';
  if (node.type === 'noteReference' || node.type === 'wikiLink')
    return node.attrs?.label ?? node.attrs?.wiki_target ?? '';
  if (node.type === 'inlineMath') return node.attrs?.source ?? '';
  return (node.content ?? []).map(richText).join('');
}

export function noteAnchor(
  document: NoteDocument,
  fragment: string,
  boundBlock?: string | null,
): { block: string | null; blocks: DocumentNode[] } | null {
  if (!fragment && !boundBlock) return { block: null, blocks: document.content };
  const matches: Array<{ node: DocumentNode; siblings: DocumentNode[]; index: number }> = [];
  const headings: string[] = [];
  const wantedHeadings = fragment.split('#').map((part) => part.trim());
  const visit = (nodes: DocumentNode[]) => {
    nodes.forEach((node, index) => {
      if (node.type === 'heading') {
        const level = node.attrs?.level ?? 1;
        headings.length = level - 1;
        headings[level - 1] = richText(node).trim();
      }
      const match = boundBlock
        ? node.attrs?.id === boundBlock
        : fragment.startsWith('^')
          ? node.attrs?.block_id === fragment.slice(1)
          : node.type === 'heading' &&
            (wantedHeadings.length === 1
              ? richText(node).trim() === wantedHeadings[0]
              : headings.filter(Boolean).slice(-wantedHeadings.length).join('#') ===
                wantedHeadings.join('#'));
      if (match && node.attrs?.id) matches.push({ node, siblings: nodes, index });
      if (node.content) visit(node.content);
    });
  };
  visit(document.content);
  if (matches.length !== 1) return null;
  const { node, siblings, index } = matches[0];
  if (node.type !== 'heading') return { block: node.attrs!.id!, blocks: [node] };
  let end = index + 1;
  while (
    end < siblings.length &&
    !(
      siblings[end].type === 'heading' &&
      (siblings[end].attrs?.level ?? 1) <= (node.attrs?.level ?? 1)
    )
  )
    ++end;
  return { block: node.attrs!.id!, blocks: siblings.slice(index, end) };
}

async function exactMatches<T extends { id: string }>(
  load: (cursor: string | null) => Promise<{ items: T[]; next_cursor: string | null }>,
  matches: (item: T) => boolean,
): Promise<{ values: T[]; complete: boolean }> {
  const values = new Map<string, T>();
  const seen = new Set<string>();
  let cursor: string | null = null;
  for (let page = 0; page < MAX_WIKI_PAGES; ++page) {
    const result = await load(cursor);
    for (const item of result.items) if (matches(item)) values.set(item.id, item);
    if (values.size > 1 || !result.next_cursor)
      return { values: [...values.values()], complete: true };
    if (seen.has(result.next_cursor)) break;
    cursor = result.next_cursor;
    seen.add(cursor);
  }
  return { values: [...values.values()], complete: false };
}

/** All I/O is through the existing scoped note/source commands. No filesystem or URL reads. */
export async function resolveWiki(
  attrs: NodeAttributes,
  current: Note | null,
  port: WikiPort,
  options: { fresh?: boolean } = {},
): Promise<WikiResult> {
  let parsed: ReturnType<typeof wikiParts>;
  try {
    parsed = wikiParts(attrs.wiki_target ?? '', current?.title);
  } catch (error) {
    return { state: 'invalid', message: error instanceof Error ? error.message : '引用目标无效。' };
  }
  let id = attrs.target_id ?? (parsed.path === '' ? current?.id : undefined);
  const uuid = /^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i;
  if (!id && uuid.test(parsed.path)) id = parsed.path;
  if (!id && current && !parsed.fragment) {
    const images = new Map<string, { hash: string; alt: string }>();
    const visit = (nodes: DocumentNode[]) =>
      nodes.forEach((node) => {
        if (
          node.type === 'image' &&
          node.attrs?.original_name === parsed.path &&
          /^[\da-f]{64}$/.test(node.attrs?.blob_hash ?? '')
        )
          images.set(node.attrs!.blob_hash!, {
            hash: node.attrs!.blob_hash!,
            alt: node.attrs?.alt ?? parsed.path,
          });
        if (node.content) visit(node.content);
      });
    visit(current.document.content);
    if (images.size > 1)
      return { state: 'ambiguous', message: '存在多张同名图片，请使用不同名称。' };
    if (images.size === 1)
      return { state: 'available', kind: 'image', note: current, ...[...images.values()][0] };
  }
  if (!id && parsed.path) {
    const noteLike = !/\.[^/.]+$/.test(parsed.path) || /\.md$/i.test(parsed.path);
    if (noteLike) {
      const found = await exactMatches(
        (cursor) => port.notes(parsed.path.replace(/\.md$/i, ''), cursor),
        (note) => titleKey(note.title) === titleKey(parsed.path),
      );
      if (!found.complete)
        return { state: 'limit', message: '同名检查超出查询限额，请改用笔记 UUID。' };
      if (found.values.length > 1)
        return { state: 'ambiguous', message: '存在多篇同名笔记，请改用笔记 UUID。' };
      id = found.values[0]?.id;
    }
    if (!id) {
      const found = await exactMatches(
        (cursor) => port.sources(parsed.path, cursor),
        (source) =>
          source.display_title.normalize('NFC').toLowerCase() ===
          parsed.path.normalize('NFC').toLowerCase(),
      );
      if (!found.complete)
        return { state: 'limit', message: '资料查询超出限额，请改用资料 UUID。' };
      if (found.values.length > 1)
        return { state: 'ambiguous', message: '存在多份同名资料，请改用资料 UUID。' };
      id = found.values[0]?.id;
    }
  }
  if (!id) return { state: 'missing', message: '当前知识库中找不到引用目标。' };
  const base: ResolveLink = {
    target_id: id,
    target_block_id: attrs.target_block_id ?? null,
    target_revision_id: attrs.target_revision_id ?? null,
  };
  const resolved = await port.resolve(base, options);
  if (resolved.state !== 'available')
    return {
      state:
        resolved.state === 'target_trashed'
          ? 'trashed'
          : resolved.state === 'block_missing'
            ? 'anchor_missing'
            : 'missing',
      message:
        resolved.state === 'target_trashed'
          ? '目标已移入回收站。'
          : resolved.state === 'block_missing'
            ? '引用的段落已不存在。'
            : '引用目标或固定版本已不存在。',
    };
  if (resolved.source_target) {
    if (resolved.source_target.locator_status === 'unavailable')
      return { state: 'anchor_missing', message: '此资料中的固定位置已无法定位，请检查原始摘录。' };
    if (parsed.fragment && !attrs.target_block_id)
      return {
        state: 'anchor_missing',
        message: '此资料锚点无法精确解析，请使用资料中的稳定摘录引用。',
      };
    const [source, version] = await Promise.all([
      port.source(resolved.source_target.source_id),
      port.version(resolved.source_target.source_version_id),
    ]);
    return {
      state: 'available',
      kind: 'source',
      source,
      version,
      target: base,
      sourceTarget: resolved.source_target,
    };
  }
  const note =
    current?.id === id && !attrs.target_revision_id
      ? current
      : await port.note(id, attrs.target_revision_id ?? null);
  if (note.trashed) return { state: 'trashed', message: '目标已移入回收站。' };
  const anchor = noteAnchor(note.document, parsed.fragment, attrs.target_block_id);
  if (!anchor) return { state: 'anchor_missing', message: '此版本中找不到唯一的标题或块锚。' };
  return {
    state: 'available',
    kind: 'note',
    note,
    target: { ...base, target_block_id: anchor.block },
    blocks: anchor.blocks,
  };
}

/** Embeds have independent rendering budgets, never traverse the rest of a large tree. */
export function boundedEmbed(nodes: DocumentNode[]): {
  content: DocumentNode[];
  truncated: boolean;
} {
  let budget = 600,
    characters = 24000,
    truncated = false;
  const visit = (items: DocumentNode[], depth: number): DocumentNode[] => {
    const result: DocumentNode[] = [];
    for (let index = 0; index < items.length; ++index) {
      if (budget <= 0 || characters <= 0 || depth > 16) {
        truncated = true;
        break;
      }
      const item = items[index];
      if ((item.attrs?.source?.length ?? 0) > characters) {
        truncated = true;
        break;
      }
      --budget;
      characters -= item.attrs?.source?.length ?? 0;
      const clone = { ...item };
      if (item.text) {
        let end = Math.min(item.text.length, characters);
        if (end < item.text.length && end > 0 && /[\uD800-\uDBFF]/.test(item.text[end - 1])) --end;
        clone.text = item.text.slice(0, end);
        characters -= clone.text.length;
        if (clone.text.length < item.text.length) truncated = true;
      }
      if (item.content) clone.content = visit(item.content, depth + 1);
      result.push(clone);
    }
    return result;
  };
  const content = visit(nodes, 0);
  return { content, truncated };
}
