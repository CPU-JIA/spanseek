export type GraphPopoverAnchor = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

export type GraphPopoverViewport = {
  width: number;
  height: number;
};

export type GraphPopoverPlacement = {
  left: number;
  width: number;
  maxHeight: number;
  top: number | null;
  bottom: number | null;
  side: 'above' | 'below';
};

/** Keeps the graph tag picker inside the viewport even when the toolbar scrolls. */
export function placeGraphPopover(
  anchor: GraphPopoverAnchor,
  viewport: GraphPopoverViewport,
  { margin = 12, gap = 6, preferredWidth = 440, preferredHeight = 280 } = {},
): GraphPopoverPlacement | null {
  if (
    anchor.bottom < 0 ||
    anchor.top > viewport.height ||
    anchor.right < 0 ||
    anchor.left > viewport.width
  ) {
    return null;
  }
  const width = Math.max(0, Math.min(preferredWidth, viewport.width - margin * 2));
  const left = Math.min(
    Math.max(margin, anchor.left),
    Math.max(margin, viewport.width - width - margin),
  );
  const below = viewport.height - anchor.bottom - margin - gap;
  const above = anchor.top - margin - gap;
  const openAbove = below < 120 && above > below;
  const maxHeight = Math.max(
    24,
    Math.min(preferredHeight, openAbove ? above : below, viewport.height - margin * 2),
  );
  if (openAbove) {
    return {
      left: Math.round(left),
      width: Math.round(width),
      maxHeight: Math.round(maxHeight),
      top: null,
      bottom: Math.round(viewport.height - anchor.top + gap),
      side: 'above',
    };
  }
  return {
    left: Math.round(left),
    width: Math.round(width),
    maxHeight: Math.round(maxHeight),
    top: Math.round(anchor.bottom + gap),
    bottom: null,
    side: 'below',
  };
}
