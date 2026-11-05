import type { CanvasNode } from '@pkms/contracts';

export type CanvasViewport = {
  x: number;
  y: number;
  scale: number;
  width: number;
  height: number;
};

/**
 * The canvas keeps every node in the saved layout, but only mounts nodes that
 * can intersect the visible world rectangle (plus a small prefetch margin).
 * This is the boundary that keeps a 1,000-node board responsive.
 */
export function visibleCanvasNodes(
  nodes: CanvasNode[],
  viewport: CanvasViewport,
  margin = 240,
): CanvasNode[] {
  if (!Number.isFinite(viewport.scale) || viewport.scale <= 0) return nodes;
  const left = -viewport.x / viewport.scale - margin;
  const top = -viewport.y / viewport.scale - margin;
  const right = left + viewport.width / viewport.scale + margin * 2;
  const bottom = top + viewport.height / viewport.scale + margin * 2;
  return nodes.filter(
    (node) =>
      node.x + node.width >= left &&
      node.x <= right &&
      node.y + node.height >= top &&
      node.y <= bottom,
  );
}

export function canvasBounds(nodes: CanvasNode[]) {
  if (!nodes.length) return { x: 0, y: 0, width: 900, height: 620 };
  const left = Math.min(...nodes.map((node) => node.x));
  const top = Math.min(...nodes.map((node) => node.y));
  const right = Math.max(...nodes.map((node) => node.x + node.width));
  const bottom = Math.max(...nodes.map((node) => node.y + node.height));
  return {
    x: left,
    y: top,
    width: Math.max(1, right - left),
    height: Math.max(1, bottom - top),
  };
}

export function fitCanvasViewport(
  nodes: CanvasNode[],
  width: number,
  height: number,
  padding = 80,
): { x: number; y: number; scale: number } {
  const bounds = canvasBounds(nodes);
  const scale = Math.min(
    1.5,
    Math.max(
      0.25,
      Math.min((width - padding * 2) / bounds.width, (height - padding * 2) / bounds.height),
    ),
  );
  return {
    x: (width - bounds.width * scale) / 2 - bounds.x * scale,
    y: (height - bounds.height * scale) / 2 - bounds.y * scale,
    scale,
  };
}

/**
 * Small canvases open at a readable scale instead of shrinking every node
 * into an overview that makes 10–13px labels unreadable. Fit remains one
 * click away, and large canvases still open fitted for orientation.
 */
export function initialCanvasViewport(
  nodes: CanvasNode[],
  width: number,
  height: number,
  padding = 80,
): { x: number; y: number; scale: number } {
  const fitted = fitCanvasViewport(nodes, width, height, padding);
  if (!nodes.length || nodes.length > 60) return fitted;
  const scale = Math.min(1, Math.max(fitted.scale, 0.8));
  if (scale <= fitted.scale + 0.001) return fitted;
  const bounds = canvasBounds(nodes);
  return {
    x: 32 - bounds.x * scale,
    y: 32 - bounds.y * scale,
    scale,
  };
}
