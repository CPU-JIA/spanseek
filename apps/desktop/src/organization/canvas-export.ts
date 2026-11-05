import type { Canvas, CanvasNode, CanvasNote, KnowledgeGraphEdge } from '@pkms/contracts';
import { canvasRelationLines } from './canvas-relations.ts';

type Labels = {
  missing: string;
  pending: string;
  trashed: string;
  partial: string;
  untitled: string;
};

export function canvasSvgBounds(nodes: readonly CanvasNode[]) {
  const left = Math.min(0, ...nodes.map((node) => node.x));
  const top = Math.min(0, ...nodes.map((node) => node.y));
  const right = Math.max(900, ...nodes.map((node) => node.x + node.width));
  const bottom = Math.max(620, ...nodes.map((node) => node.y + node.height));
  return { left, top, width: right - left + 80, height: bottom - top + 80 };
}

function xml(value: string) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

/** Bound previews to their card; the complete title remains in the SVG title element. */
function wrap(value: string, width: number, fontSize: number, maxLines: number): string[] {
  if (maxLines <= 0) return [];
  const characters = Array.from(value.replace(/\s+/g, ' ').trim());
  const lines: string[] = [];
  let position = 0;
  while (position < characters.length && lines.length < maxLines) {
    let line = '';
    let measured = 0;
    while (position < characters.length) {
      const character = characters[position];
      const nextWidth = fontSize * (character.codePointAt(0)! > 255 ? 1 : 0.62);
      if (line && measured + nextWidth > width - fontSize) break;
      line += character;
      measured += nextWidth;
      position++;
    }
    if (position < characters.length && lines.length === maxLines - 1) line += '…';
    lines.push(line);
  }
  return lines;
}

export function canvasSvg(
  canvas: Canvas,
  notes: ReadonlyMap<string, CanvasNote>,
  edges: readonly KnowledgeGraphEdge[],
  options: { labels: Labels; missingIds?: ReadonlySet<string>; relationsComplete: boolean },
): string {
  const bounds = canvasSvgBounds(canvas.nodes);
  const shiftX = 40 - bounds.left;
  const shiftY = 40 - bounds.top;
  const lines = canvasRelationLines(canvas.nodes, edges)
    .map(
      ({ edge, x1, y1, x2, y2 }) =>
        `<line data-source-id="${xml(edge.source_id)}" data-target-id="${xml(edge.target_id)}" data-target-block-id="${xml(edge.target_block_id ?? '')}" x1="${x1 + shiftX}" y1="${y1 + shiftY}" x2="${x2 + shiftX}" y2="${y2 + shiftY}" stroke="#59677d" stroke-width="1.8"${edge.kind === 'relation' ? ' stroke-dasharray="5 4"' : ''} marker-end="url(#relation-arrow)"><title>${xml(edge.context)}</title></line>`,
    )
    .join('');
  const cards = [...canvas.nodes]
    .sort((a, b) => a.z_index - b.z_index)
    .map((node, index) => {
      const note = notes.get(node.note_id);
      const title = note
        ? note.note.title || options.labels.untitled
        : options.missingIds?.has(node.note_id)
          ? options.labels.missing
          : options.labels.pending;
      const preview = note?.metadata.trashed
        ? options.labels.trashed
        : Array.from(note?.note.preview ?? '')
            .slice(0, 90)
            .join('');
      const titleLines = wrap(title, node.width - 28, 14, 2);
      const previewY = 26 + titleLines.length * 18 + 8;
      const previewLines = wrap(
        preview,
        node.width - 28,
        11,
        Math.max(0, Math.floor((node.height - 12 - previewY) / 15) + 1),
      );
      const titles = titleLines
        .map((line, i) => `<tspan x="14" y="${26 + i * 18}">${xml(line)}</tspan>`)
        .join('');
      const previews = previewLines
        .map((line, i) => `<tspan x="14" y="${previewY + i * 15}">${xml(line)}</tspan>`)
        .join('');
      return `<g data-note-id="${xml(node.note_id)}" transform="translate(${node.x + shiftX} ${node.y + shiftY})"><title>${xml(title)}</title><defs><clipPath id="card-text-${index}"><rect x="12" y="12" width="${node.width - 24}" height="${node.height - 24}"/></clipPath></defs><rect width="${node.width}" height="${node.height}" rx="10" fill="#fff" stroke="#c9c9c9"/><g clip-path="url(#card-text-${index})" font-family="Segoe UI, sans-serif"><text font-size="14" font-weight="600" fill="#222">${titles}</text><text font-size="11" fill="#666">${previews}</text></g></g>`;
    })
    .join('');
  const heading = wrap(canvas.name, bounds.width - 80, 18, 1)[0] ?? '';
  const partial = options.relationsComplete
    ? ''
    : `<text x="40" y="${bounds.height - 14}" font-family="Segoe UI, sans-serif" font-size="12" fill="#666">${xml(options.labels.partial)}</text>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${bounds.width}" height="${bounds.height}" viewBox="0 0 ${bounds.width} ${bounds.height}" role="img"><title>${xml(canvas.name)}</title><defs><marker id="relation-arrow" markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto"><path d="M0 0 L7 3.5 L0 7 Z" fill="#59677d"/></marker></defs><rect width="100%" height="100%" fill="#fafafa"/><text x="40" y="32" font-family="Segoe UI, sans-serif" font-size="18" font-weight="600" fill="#222">${xml(heading)}</text>${lines}${cards}${partial}</svg>`;
}
