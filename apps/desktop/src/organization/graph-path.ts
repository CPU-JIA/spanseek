export type GraphPathNode = { id: string };
export type GraphPathEdge = { source_id: string; target_id: string };

/**
 * Returns one deterministic shortest undirected path inside the already
 * filtered graph. Graph edges carry direction for provenance, but navigating a
 * knowledge graph is bidirectional: either object can lead back to the other.
 */
export function shortestGraphPath(
  nodes: readonly GraphPathNode[],
  edges: readonly GraphPathEdge[],
  sourceId: string | null,
  targetId: string | null,
): string[] | null {
  if (!sourceId || !targetId) return null;
  const known = new Set(nodes.map((node) => node.id));
  if (!known.has(sourceId) || !known.has(targetId)) return null;
  if (sourceId === targetId) return [sourceId];

  const adjacency = new Map<string, string[]>();
  for (const edge of edges) {
    if (
      edge.source_id === edge.target_id ||
      !known.has(edge.source_id) ||
      !known.has(edge.target_id)
    ) {
      continue;
    }
    const source = adjacency.get(edge.source_id);
    if (source) source.push(edge.target_id);
    else adjacency.set(edge.source_id, [edge.target_id]);
    const target = adjacency.get(edge.target_id);
    if (target) target.push(edge.source_id);
    else adjacency.set(edge.target_id, [edge.source_id]);
  }
  for (const neighbours of adjacency.values()) neighbours.sort();

  const previous = new Map<string, string | null>([[sourceId, null]]);
  const queue = [sourceId];
  for (let head = 0; head < queue.length; head += 1) {
    const current = queue[head]!;
    for (const next of adjacency.get(current) ?? []) {
      if (previous.has(next)) continue;
      previous.set(next, current);
      if (next === targetId) {
        const path = [next];
        let cursor: string | null = current;
        while (cursor) {
          path.push(cursor);
          cursor = previous.get(cursor) ?? null;
        }
        return path.reverse();
      }
      queue.push(next);
    }
  }
  return null;
}

export function graphPathSegments(path: readonly string[]): string[] {
  return path.slice(0, -1).map((id, index) => {
    const next = path[index + 1]!;
    return id < next ? `${id}\u0000${next}` : `${next}\u0000${id}`;
  });
}
