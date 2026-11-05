export type GraphLayoutNode = {
  id: string;
  x: number;
  y: number;
  radius: number;
};

export type GraphLayoutEdge = {
  source: string;
  target: string;
  weight: number;
};

/** Fit the current positions, including manually moved nodes, into the viewport. */
export function fitGraphViewport(
  nodes: readonly GraphLayoutNode[],
  width: number,
  height: number,
): { x: number; y: number; k: number } {
  if (!nodes.length || width <= 0 || height <= 0) return { x: 0, y: 0, k: 1 };
  let left = Infinity;
  let top = Infinity;
  let right = -Infinity;
  let bottom = -Infinity;
  for (const node of nodes) {
    left = Math.min(left, node.x - node.radius);
    top = Math.min(top, node.y - node.radius);
    right = Math.max(right, node.x + node.radius);
    bottom = Math.max(bottom, node.y + node.radius);
  }
  const padding = Math.min(40, width * 0.1, height * 0.1);
  const k = Math.min(
    1,
    (width - padding * 2) / Math.max(1, right - left),
    (height - padding * 2) / Math.max(1, bottom - top),
  );
  return { x: width / 2 - ((left + right) / 2) * k, y: height / 2 - ((top + bottom) / 2) * k, k };
}

type ForceNode = {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
  degree: number;
};

function idHash(value: string) {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function desiredIterations(count: number) {
  if (count <= 180) return 110;
  if (count <= 700) return 72;
  if (count <= 2200) return 44;
  if (count <= 4000) return 24;
  return 12;
}

/**
 * Bounded deterministic graph layout. The canvas only receives coordinates;
 * the repository stays the source of truth and re-running the layout never
 * depends on Math.random, wall-clock time, or browser-specific neighbour order.
 */
export function layoutKnowledgeGraph(
  nodes: readonly { id: string; degree: number }[],
  edges: readonly GraphLayoutEdge[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  if (!nodes.length || width <= 0 || height <= 0) return [];
  const nodeIds = new Set(nodes.map((node) => node.id));
  const validEdges = edges.filter(
    (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target) && edge.source !== edge.target,
  );
  const byId = new Map<string, ForceNode>();
  const forceNodes = nodes.map((node, index) => {
    const hash = idHash(node.id);
    const angle = index * 2.399963229728653 + (hash % 997) / 997;
    const radius = Math.sqrt(index + 1) * 18 + (hash % 31);
    const force: ForceNode = {
      id: node.id,
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
      vx: 0,
      vy: 0,
      radius: 4.5 + Math.min(9, Math.sqrt(Math.max(0, node.degree)) * 2.4),
      degree: node.degree,
    };
    byId.set(node.id, force);
    return force;
  });
  const ideal = Math.max(46, Math.min(92, 64 + Math.sqrt(forceNodes.length) * 1.4));
  const iterations = desiredIterations(forceNodes.length);
  const cellSize = ideal * 1.7;
  const maxNeighbours = forceNodes.length > 8000 ? 12 : forceNodes.length > 1600 ? 24 : 40;
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const cells = new Map<string, number[]>();
    forceNodes.forEach((node, index) => {
      const key = `${Math.floor(node.x / cellSize)}:${Math.floor(node.y / cellSize)}`;
      const bucket = cells.get(key);
      if (bucket) bucket.push(index);
      else cells.set(key, [index]);
    });
    const cooling = 1 - iteration / (iterations + 1);
    forceNodes.forEach((node, index) => {
      const cellX = Math.floor(node.x / cellSize);
      const cellY = Math.floor(node.y / cellSize);
      let visited = 0;
      for (let offsetX = -1; offsetX <= 1 && visited < maxNeighbours; offsetX += 1) {
        for (let offsetY = -1; offsetY <= 1 && visited < maxNeighbours; offsetY += 1) {
          const bucket = cells.get(`${cellX + offsetX}:${cellY + offsetY}`);
          if (!bucket) continue;
          const stride =
            bucket.length > maxNeighbours
              ? Math.max(1, Math.floor(bucket.length / maxNeighbours))
              : 1;
          for (
            let bucketIndex = 0;
            bucketIndex < bucket.length && visited < maxNeighbours;
            bucketIndex += stride
          ) {
            const otherIndex = bucket[bucketIndex];
            if (otherIndex === index) continue;
            const other = forceNodes[otherIndex];
            let dx = node.x - other.x;
            let dy = node.y - other.y;
            let distance = Math.hypot(dx, dy);
            if (distance < 0.01) {
              dx = ((idHash(node.id) % 17) - 8) * 0.02 || 0.02;
              dy = ((idHash(other.id) % 19) - 9) * 0.02 || 0.02;
              distance = Math.hypot(dx, dy);
            }
            const minimum = ideal * 0.78;
            if (distance >= minimum * 1.35) continue;
            const strength = Math.min(4.8, (minimum * minimum) / (distance * distance)) * cooling;
            node.vx += (dx / distance) * strength;
            node.vy += (dy / distance) * strength;
            visited += 1;
          }
        }
      }
    });
    for (const edge of validEdges) {
      const source = byId.get(edge.source);
      const target = byId.get(edge.target);
      if (!source || !target) continue;
      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const distance = Math.max(0.01, Math.hypot(dx, dy));
      const targetDistance = ideal * (1 + Math.min(0.35, Math.log2(edge.weight + 1) * 0.08));
      const strength = ((distance - targetDistance) / distance) * 0.018 * cooling;
      const forceX = dx * strength;
      const forceY = dy * strength;
      source.vx += forceX / Math.max(1, source.degree);
      source.vy += forceY / Math.max(1, source.degree);
      target.vx -= forceX / Math.max(1, target.degree);
      target.vy -= forceY / Math.max(1, target.degree);
    }
    for (const node of forceNodes) {
      node.vx += -node.x * 0.0009;
      node.vy += -node.y * 0.0009;
      node.vx *= 0.76;
      node.vy *= 0.76;
      const limit = ideal * 0.42;
      const speed = Math.hypot(node.vx, node.vy);
      if (speed > limit) {
        node.vx = (node.vx / speed) * limit;
        node.vy = (node.vy / speed) * limit;
      }
      node.x += node.vx;
      node.y += node.vy;
    }
  }
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const node of forceNodes) {
    minX = Math.min(minX, node.x - node.radius);
    minY = Math.min(minY, node.y - node.radius);
    maxX = Math.max(maxX, node.x + node.radius);
    maxY = Math.max(maxY, node.y + node.radius);
  }
  const padding = Math.min(70, Math.max(28, Math.min(width, height) * 0.09));
  const usableWidth = Math.max(1, width - padding * 2);
  const usableHeight = Math.max(1, height - padding * 2);
  const scale = Math.min(
    usableWidth / Math.max(1, maxX - minX),
    usableHeight / Math.max(1, maxY - minY),
  );
  const centerX = (minX + maxX) / 2;
  const centerY = (minY + maxY) / 2;
  return forceNodes.map((node) => ({
    id: node.id,
    x: width / 2 + (node.x - centerX) * scale,
    y: height / 2 + (node.y - centerY) * scale,
    radius: node.radius,
  }));
}
