import { useEffect, useState, type RefObject } from 'react';

/**
 * The organization columns switch to a stacked layout from a container query,
 * not from the viewport. Keep reveal behavior tied to the same width so zoom
 * and a resized sidebar cannot leave the selected detail off screen.
 */
export function useStackedColumns(ref: RefObject<HTMLElement | null>, maxWidth = 720): boolean {
  const [stacked, setStacked] = useState(false);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const update = (width: number) => setStacked(width <= maxWidth);
    update(node.getBoundingClientRect().width);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) update(entry.contentRect.width);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [maxWidth, ref]);
  return stacked;
}

/** Align a stacked detail with the page scroller rather than relying on the
 * browser's nearest-scroll heuristic, which can leave a partly visible pane in
 * place after the list changes height. */
export function revealOrganizationDetail(detail: HTMLElement | null, inset = 8): void {
  if (!detail) return;
  const scroller = detail.closest<HTMLElement>('.org-page');
  if (!scroller) {
    detail.scrollIntoView({ block: 'start' });
    return;
  }
  const detailBox = detail.getBoundingClientRect();
  const scrollerBox = scroller.getBoundingClientRect();
  scroller.scrollTo({
    top: Math.max(0, scroller.scrollTop + detailBox.top - scrollerBox.top - inset),
    behavior: 'auto',
  });
}
