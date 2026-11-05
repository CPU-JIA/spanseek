import type { CanvasNode, KnowledgeGraphEdge, KnowledgeGraphNode } from '@pkms/contracts';

export type CanvasRelationLine = {
  edge: KnowledgeGraphEdge;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
};

export type UnplacedRelationTarget = {
  id: string;
  title: string;
};

/** Project block endpoints onto their owning note without discarding the block locator. */
export function canvasNoteEdges(
  edges: readonly KnowledgeGraphEdge[],
  graphNodes: readonly KnowledgeGraphNode[],
): KnowledgeGraphEdge[] {
  const byId = new Map(graphNodes.map((node) => [node.id, node]));
  const noteId = (id: string) => {
    const node = byId.get(id);
    if (node?.kind === 'note') return node.id;
    if (node?.kind === 'block' && node.object_id && byId.get(node.object_id)?.kind === 'note')
      return node.object_id;
    return null;
  };
  return edges.flatMap((edge) => {
    if (edge.kind === 'canvas') return [];
    const source_id = noteId(edge.source_id);
    const target_id = noteId(edge.target_id);
    return source_id && target_id ? [{ ...edge, source_id, target_id }] : [];
  });
}

export function canvasRelationKey(edge: KnowledgeGraphEdge): string {
  return JSON.stringify([edge.source_id, edge.target_id, edge.kind, edge.target_block_id]);
}

/**
 * Only draw edges whose two endpoints are actual nodes on this canvas. The
 * graph is the source of truth; a canvas never invents a second relation store.
 */
export function canvasRelationLines(
  nodes: readonly CanvasNode[],
  edges: readonly KnowledgeGraphEdge[],
): CanvasRelationLine[] {
  const byId = new Map(nodes.map((node) => [node.note_id, node]));
  return edges.flatMap((edge) => {
    if (edge.kind === 'canvas') return [];
    const source = byId.get(edge.source_id);
    const target = byId.get(edge.target_id);
    if (!source || !target) return [];
    return [
      {
        edge,
        x1: source.x + source.width / 2,
        y1: source.y + source.height / 2,
        x2: target.x + target.width / 2,
        y2: target.y + target.height / 2,
      },
    ];
  });
}

/**
 * Notes with an established semantic edge to a note already on the canvas,
 * but which do not yet have their own canvas node. Block endpoints must already
 * be projected through canvasNoteEdges; non-note targets never enter this list.
 */
export function unplacedRelationTargets(
  nodes: readonly CanvasNode[],
  edges: readonly KnowledgeGraphEdge[],
  graphNodes: readonly KnowledgeGraphNode[],
): UnplacedRelationTarget[] {
  const onCanvas = new Set(nodes.map((node) => node.note_id));
  const graphById = new Map(graphNodes.map((node) => [node.id, node]));
  const targets = new Map<string, UnplacedRelationTarget>();
  for (const edge of edges) {
    if (edge.kind === 'canvas') continue;
    const sourceOnCanvas = onCanvas.has(edge.source_id);
    const targetOnCanvas = onCanvas.has(edge.target_id);
    if (sourceOnCanvas === targetOnCanvas) continue;
    const otherId = sourceOnCanvas ? edge.target_id : edge.source_id;
    const node = graphById.get(otherId);
    if (!node || node.kind !== 'note' || onCanvas.has(otherId)) continue;
    targets.set(otherId, { id: otherId, title: node.title });
  }
  return [...targets.values()];
}

export function relationKindLabel(
  kind: KnowledgeGraphEdge['kind'],
  t: (zh: string, en: string) => string = (_, en) => en,
): string {
  return kind === 'relation'
    ? t('属性关系', 'property relation')
    : kind === 'block_reference'
      ? t('块引用', 'block reference')
      : kind === 'canvas'
        ? t('画布成员', 'canvas membership')
        : t('正文引用', 'body reference');
}
