import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react';
import {
  AlertCircle,
  FileText,
  Focus,
  FolderClosed,
  Highlighter,
  LayoutDashboard,
  LoaderCircle,
  Maximize2,
  Maximize,
  Minimize,
  Minus,
  Network,
  Pilcrow,
  Plus,
  RefreshCw,
  Route,
  Search,
  SquareArrowOutUpRight,
  X,
} from 'lucide-react';
import type {
  KnowledgeGraph,
  KnowledgeGraphEdge,
  KnowledgeGraphNode,
  ResolveLink,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { PageFrame } from '../layout/PageFrame';
import { Pagination } from '../layout/Pagination.tsx';
import {
  SourceCollectionPicker,
  type SourceCollectionChoice,
} from '../sources/SourceCollectionPicker';
import { useContentFullscreen } from '../ui/useContentFullscreen';
import { fitGraphViewport, layoutKnowledgeGraph, type GraphLayoutNode } from './graph-layout';
import { placeGraphPopover } from './graph-popover';
import { graphPathSegments, shortestGraphPath } from './graph-path';
import './graph-page.css';

type GraphKind = KnowledgeGraphNode['kind'];
type Transform = { x: number; y: number; k: number };

const KIND_ORDER: GraphKind[] = ['note', 'block', 'source', 'annotation', 'canvas'];
const DEFAULT_KINDS: GraphKind[] = [...KIND_ORDER];

type CanvasHandle = {
  fit: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
};

type GraphCanvasProps = {
  viewportKey: string;
  nodes: KnowledgeGraphNode[];
  edges: KnowledgeGraphEdge[];
  selectedId: string | null;
  showAllLabels: boolean;
  pathNodeIds: string[];
  onSelect: (id: string | null) => void;
  onOpen: (node: KnowledgeGraphNode) => void;
};

function clampZoom(value: number) {
  return Math.min(3.6, Math.max(0.18, value));
}
function bounded(value: number, minimum: number, maximum: number) {
  return Math.max(minimum, Math.min(maximum, value));
}

const GraphCanvas = forwardRef<CanvasHandle, GraphCanvasProps>(function GraphCanvas(
  { viewportKey, nodes, edges, selectedId, showAllLabels, pathNodeIds, onSelect, onOpen },
  forwardedRef,
) {
  const { t } = useI18n();
  const host = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const wheelHandler = useRef<(event: WheelEvent) => void>(() => undefined);
  const [savedTransform, setSavedTransform] = usePageNavigationState<Transform>(
    `graph.viewport/${viewportKey}`,
    { x: 0, y: 0, k: 1 },
  );
  const transform = useRef<Transform>({ ...savedTransform });
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [zoomPercent, setZoomPercent] = useState(() => Math.round(savedTransform.k * 100));
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const hoveredRef = useRef<string | null>(null);
  hoveredRef.current = hoveredId;
  const selectedRef = useRef<string | null>(selectedId);
  selectedRef.current = selectedId;
  const pointer = useRef<{
    id: number;
    startX: number;
    startY: number;
    lastX: number;
    lastY: number;
    moved: boolean;
    node: GraphLayoutNode | null;
  } | null>(null);
  const baseLayout = useMemo(
    () =>
      layoutKnowledgeGraph(
        nodes,
        edges.map((edge) => ({
          source: edge.source_id,
          target: edge.target_id,
          weight: edge.weight,
        })),
        size.width,
        size.height,
      ),
    [nodes, edges, size.width, size.height],
  );
  const [manualPositions, setManualPositions] = usePageNavigationState<
    Record<string, { x: number; y: number }>
  >('graph.positions', () => ({}));
  const layout = useMemo(
    () =>
      baseLayout.map((point) => {
        const manual = manualPositions[point.id];
        return manual ? { ...point, x: manual.x, y: manual.y } : point;
      }),
    [baseLayout, manualPositions],
  );
  const layoutRef = useRef<GraphLayoutNode[]>(layout);
  if (!pointer.current) layoutRef.current = layout;
  const nodeById = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes]);

  useEffect(() => {
    if (!host.current) return;
    const observer = new ResizeObserver(([entry]) => {
      const rect = entry.contentRect;
      setSize({
        width: Math.max(1, Math.round(rect.width)),
        height: Math.max(1, Math.round(rect.height)),
      });
    });
    observer.observe(host.current);
    const rect = host.current.getBoundingClientRect();
    setSize({
      width: Math.max(1, Math.round(rect.width)),
      height: Math.max(1, Math.round(rect.height)),
    });
    return () => observer.disconnect();
  }, []);

  const fit = () => {
    transform.current = fitGraphViewport(layoutRef.current, size.width, size.height);
    setSavedTransform({ ...transform.current });
    setZoomPercent(Math.round(transform.current.k * 100));
    draw();
  };
  const zoomBy = (factor: number) => {
    const current = transform.current;
    const k = clampZoom(current.k * factor);
    const centerX = size.width / 2;
    const centerY = size.height / 2;
    const worldX = (centerX - current.x) / current.k;
    const worldY = (centerY - current.y) / current.k;
    transform.current = { k, x: centerX - worldX * k, y: centerY - worldY * k };
    setSavedTransform({ ...transform.current });
    setZoomPercent(Math.round(k * 100));
    draw();
  };
  useImperativeHandle(forwardedRef, () => ({
    fit,
    zoomIn: () => zoomBy(1.24),
    zoomOut: () => zoomBy(1 / 1.24),
  }));

  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseLayout]);

  useEffect(() => {
    draw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, hoveredId, showAllLabels, pathNodeIds, size.width, size.height, layout]);

  function palette() {
    const style = canvas.current ? getComputedStyle(canvas.current) : null;
    const value = (name: string, fallback: string) =>
      style?.getPropertyValue(name).trim() || fallback;
    return {
      text: value('--text', '#1d1d1f'),
      muted: value('--muted', '#6b7280'),
      line: value('--line', '#d7d7dc'),
      surface: value('--surface', '#ffffff'),
      accent: value('--accent', '#3f6ad8'),
      success: value('--success', '#2f855a'),
      warning: value('--warning', '#b7791f'),
    };
  }

  function draw() {
    const element = canvas.current;
    if (!element || size.width <= 0 || size.height <= 0) return;
    const dpr = Math.min(2.5, Math.max(1, window.devicePixelRatio || 1));
    if (
      element.width !== Math.round(size.width * dpr) ||
      element.height !== Math.round(size.height * dpr)
    ) {
      element.width = Math.round(size.width * dpr);
      element.height = Math.round(size.height * dpr);
    }
    const context = element.getContext('2d');
    if (!context) return;
    const colors = palette();
    const pathNodes = new Set(pathNodeIds);
    const pathSegments = new Set(graphPathSegments(pathNodeIds));
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, element.width, element.height);
    const current = transform.current;
    context.setTransform(dpr * current.k, 0, 0, dpr * current.k, dpr * current.x, dpr * current.y);
    const currentLayout = layoutRef.current;
    const pointById = new Map(currentLayout.map((point) => [point.id, point]));
    const selectedNeighbours = new Set<string>();
    if (selectedRef.current) {
      for (const edge of edges) {
        if (edge.source_id === selectedRef.current) selectedNeighbours.add(edge.target_id);
        if (edge.target_id === selectedRef.current) selectedNeighbours.add(edge.source_id);
      }
    }
    const highlighted = (id: string) =>
      pathNodes.has(id) ||
      !selectedRef.current ||
      id === selectedRef.current ||
      selectedNeighbours.has(id);
    context.lineCap = 'round';
    for (const edge of edges) {
      const source = pointById.get(edge.source_id);
      const target = pointById.get(edge.target_id);
      if (!source || !target) continue;
      const active =
        pathSegments.has(
          edge.source_id < edge.target_id
            ? `${edge.source_id}\u0000${edge.target_id}`
            : `${edge.target_id}\u0000${edge.source_id}`,
        ) ||
        !selectedRef.current ||
        edge.source_id === selectedRef.current ||
        edge.target_id === selectedRef.current;
      context.beginPath();
      context.moveTo(source.x, source.y);
      context.lineTo(target.x, target.y);
      const onPath = pathSegments.has(
        edge.source_id < edge.target_id
          ? `${edge.source_id}\u0000${edge.target_id}`
          : `${edge.target_id}\u0000${edge.source_id}`,
      );
      context.strokeStyle = onPath ? colors.accent : active ? colors.muted : colors.line;
      context.globalAlpha = onPath
        ? 0.98
        : active
          ? Math.min(0.62, 0.18 + Math.log2(edge.weight + 1) * 0.09)
          : 0.12;
      context.lineWidth =
        (onPath ? 3.1 : Math.max(0.65, Math.min(3.2, 0.75 + Math.log2(edge.weight + 1) * 0.45))) /
        current.k;
      context.stroke();
    }
    context.globalAlpha = 1;
    const labelThreshold = nodes.length > 140 ? 1.4 : nodes.length > 25 ? 1.2 : 0.7;
    for (const point of currentLayout) {
      const node = nodeById.get(point.id);
      if (!node) continue;
      const active = highlighted(node.id);
      const typeColor =
        node.kind === 'source'
          ? colors.success
          : node.kind === 'annotation'
            ? colors.warning
            : colors.accent;
      context.globalAlpha = active ? 1 : 0.24;
      context.beginPath();
      context.arc(point.x, point.y, point.radius, 0, Math.PI * 2);
      context.fillStyle = typeColor;
      context.fill();
      if (pathNodes.has(node.id) && node.id !== selectedRef.current) {
        context.beginPath();
        context.arc(point.x, point.y, point.radius + 4.5 / current.k, 0, Math.PI * 2);
        context.strokeStyle = colors.accent;
        context.lineWidth = 2.4 / current.k;
        context.stroke();
      }
      if (node.id === selectedRef.current) {
        context.beginPath();
        context.arc(point.x, point.y, point.radius + 5 / current.k, 0, Math.PI * 2);
        context.strokeStyle = colors.accent;
        context.globalAlpha = 1;
        context.lineWidth = 2.4 / current.k;
        context.stroke();
      } else if (node.id === hoveredRef.current) {
        context.beginPath();
        context.arc(point.x, point.y, point.radius + 3.5 / current.k, 0, Math.PI * 2);
        context.strokeStyle = colors.muted;
        context.globalAlpha = 0.72;
        context.lineWidth = 1.4 / current.k;
        context.stroke();
      }
      const labelVisible =
        active &&
        (showAllLabels ||
          pathNodes.has(node.id) ||
          node.id === selectedRef.current ||
          node.id === hoveredRef.current ||
          current.k >= labelThreshold);
      if (labelVisible) {
        context.globalAlpha = active ? 0.94 : 0.22;
        const fontName = getComputedStyle(element).fontFamily || 'sans-serif';
        const fontSize = Math.max(10.5, 11.5 / current.k);
        context.font = `500 ${fontSize}px ${fontName}`;
        context.textAlign = 'center';
        context.textBaseline = 'top';
        const label = node.title.length > 28 ? `${node.title.slice(0, 27)}…` : node.title;
        const width = context.measureText(label).width + 8 / current.k;
        const labelX = bounded(
          point.x,
          (8 - current.x) / current.k + width / 2,
          (size.width - 8 - current.x) / current.k - width / 2,
        );
        const labelY = bounded(
          point.y + point.radius + 6 / current.k,
          (8 - current.y) / current.k,
          (size.height - 8 - current.y) / current.k - 15 / current.k,
        );
        context.fillStyle = colors.surface;
        context.globalAlpha = 0.82;
        context.fillRect(labelX - width / 2, labelY, width, 15 / current.k);
        context.globalAlpha = 0.96;
        context.fillStyle = colors.text;
        context.fillText(label, labelX, labelY + 1.5 / current.k);
      }
    }
    context.globalAlpha = 1;
  }

  function localPoint(event: { clientX: number; clientY: number }) {
    const rect = canvas.current?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function nodeAt(clientX: number, clientY: number) {
    const local = localPoint({ clientX, clientY });
    const current = transform.current;
    const x = (local.x - current.x) / current.k;
    const y = (local.y - current.y) / current.k;
    let found: { node: KnowledgeGraphNode; point: GraphLayoutNode; distance: number } | null = null;
    for (const point of layoutRef.current) {
      const node = nodeById.get(point.id);
      if (!node) continue;
      const distance = Math.hypot(point.x - x, point.y - y);
      const hitRadius = point.radius + 9 / current.k;
      if (distance <= hitRadius && (!found || distance < found.distance))
        found = { node, point, distance };
    }
    return found;
  }

  function commitDraggedNode() {
    const active = pointer.current;
    if (!active?.node || !active.moved) return;
    setManualPositions((current) => ({
      ...current,
      [active.node!.id]: { x: active.node!.x, y: active.node!.y },
    }));
  }

  function pointerDown(event: ReactPointerEvent<HTMLCanvasElement>) {
    if (event.button !== 0) return;
    const hit = nodeAt(event.clientX, event.clientY);
    canvas.current?.setPointerCapture(event.pointerId);
    pointer.current = {
      id: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      lastX: event.clientX,
      lastY: event.clientY,
      moved: false,
      node: hit?.point ?? null,
    };
    if (hit) onSelect(hit.node.id);
    event.currentTarget.focus({ preventScroll: true });
  }

  function pointerMove(event: ReactPointerEvent<HTMLCanvasElement>) {
    const hit = nodeAt(event.clientX, event.clientY);
    setHoveredId(hit?.node.id ?? null);
    const active = pointer.current;
    if (!active || active.id !== event.pointerId) {
      canvas.current!.style.cursor = hit ? 'move' : 'grab';
      return;
    }
    const dx = event.clientX - active.lastX;
    const dy = event.clientY - active.lastY;
    if (Math.abs(event.clientX - active.startX) + Math.abs(event.clientY - active.startY) > 2)
      active.moved = true;
    if (!active.moved) return;
    if (active.node) {
      active.node.x = active.node.x + (event.clientX - active.lastX) / transform.current.k;
      active.node.y = active.node.y + (event.clientY - active.lastY) / transform.current.k;
    } else {
      transform.current = {
        ...transform.current,
        x: transform.current.x + dx,
        y: transform.current.y + dy,
      };
    }
    active.lastX = event.clientX;
    active.lastY = event.clientY;
    canvas.current!.style.cursor = 'grabbing';
    draw();
  }

  function pointerUp(event: ReactPointerEvent<HTMLCanvasElement>) {
    const active = pointer.current;
    commitDraggedNode();
    if (active?.moved && !active.node) setSavedTransform({ ...transform.current });
    pointer.current = null;
    if (active && !active.moved && !nodeAt(event.clientX, event.clientY)) onSelect(null);
    canvas.current?.releasePointerCapture(event.pointerId);
    canvas.current!.style.cursor = nodeAt(event.clientX, event.clientY) ? 'move' : 'grab';
  }

  // React's root wheel listener can be passive in WebView2. Use a native
  // non-passive listener so the graph owns wheel gestures without scrolling
  // the surrounding page.
  wheelHandler.current = (event: WheelEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const local = localPoint(event);
    const current = transform.current;
    const factor = Math.exp(-event.deltaY * 0.0012);
    const k = clampZoom(current.k * factor);
    const worldX = (local.x - current.x) / current.k;
    const worldY = (local.y - current.y) / current.k;
    transform.current = { k, x: local.x - worldX * k, y: local.y - worldY * k };
    setSavedTransform({ ...transform.current });
    setZoomPercent(Math.round(k * 100));
    draw();
  };
  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    const listener = (event: WheelEvent) => wheelHandler.current(event);
    element.addEventListener('wheel', listener, { passive: false });
    return () => element.removeEventListener('wheel', listener);
  }, []);

  function keyDown(event: ReactKeyboardEvent<HTMLCanvasElement>) {
    const selected = selectedRef.current ? nodeById.get(selectedRef.current) : null;
    if (event.key === 'Enter' && selected) {
      event.preventDefault();
      onOpen(selected);
      return;
    }
    if (event.key === '0') {
      event.preventDefault();
      fit();
      return;
    }
    if (event.key === '+' || event.key === '=') {
      event.preventDefault();
      zoomBy(1.24);
      return;
    }
    if (event.key === '-') {
      event.preventDefault();
      zoomBy(1 / 1.24);
      return;
    }
    const amount = 48;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [amount, 0],
      ArrowRight: [-amount, 0],
      ArrowUp: [0, amount],
      ArrowDown: [0, -amount],
    };
    const move = moves[event.key];
    if (move) {
      event.preventDefault();
      transform.current = {
        ...transform.current,
        x: transform.current.x + move[0],
        y: transform.current.y + move[1],
      };
      setSavedTransform({ ...transform.current });
      draw();
    }
  }

  return (
    <div className="graph-canvas-host" ref={host}>
      <canvas
        ref={canvas}
        className="graph-canvas"
        tabIndex={0}
        role="img"
        aria-label={t(
          `知识关系图画布，${nodes.length} 个节点，${edges.length} 条连接；拖动节点调整位置，拖动空白平移，方向键平移，加减缩放，0 重置，Enter 打开选中节点。`,
          `Knowledge graph canvas with ${nodes.length} nodes and ${edges.length} links. Drag a node to move it, drag empty space to pan, arrow keys pan, plus and minus zoom, 0 resets, Enter opens the selected node.`,
        )}
        onPointerDown={pointerDown}
        onPointerMove={pointerMove}
        onPointerUp={pointerUp}
        onPointerCancel={() => {
          commitDraggedNode();
          setSavedTransform({ ...transform.current });
          pointer.current = null;
        }}
        onPointerLeave={() => setHoveredId(null)}
        onKeyDown={keyDown}
        onDoubleClick={(event) => {
          const hit = nodeAt(event.clientX, event.clientY);
          if (hit) onOpen(hit.node);
        }}
      />
      <div className="graph-canvas-hint" aria-hidden="true">
        {t(
          '拖动节点调整位置 · 拖动空白平移 · 滚轮缩放 · 双击打开',
          'Drag a node to move it · drag empty space to pan · wheel to zoom · double-click to open',
        )}
      </div>
      <output className="graph-zoom-readout" aria-live="polite" aria-atomic="true">
        {t(`缩放 ${zoomPercent}%`, `Zoom ${zoomPercent}%`)}
      </output>
    </div>
  );
});

export function GraphPage({
  identity,
  onOpenNote,
  onOpenTarget,
  onOpenCanvas,
}: {
  identity: WorkspaceIdentity;
  onOpenNote: (id: string) => void;
  onOpenTarget?: (target: ResolveLink) => Promise<void>;
  onOpenCanvas?: (id: string) => void;
}) {
  const { t } = useI18n();
  const fullscreen = useContentFullscreen('.graph-page');
  const [query, setQuery] = usePageNavigationState<string>('graph.query', '');
  const [collectionId, setCollectionId] = usePageNavigationState<string | null>(
    'graph.collection',
    null,
  );
  const [kinds, setKinds] = usePageNavigationState<GraphKind[]>('graph.kinds', DEFAULT_KINDS);
  const [tagFilter, setTagFilter] = usePageNavigationState<string[]>('graph.tags', []);
  const [orphans, setOrphans] = usePageNavigationState<boolean>('graph.orphans', false);
  const [labels, setLabels] = usePageNavigationState<boolean>('graph.labels', false);
  const [focus, setFocus] = usePageNavigationState<boolean>('graph.focus', false);
  const [selectedId, setSelectedId] = usePageNavigationState<string | null>('graph.selected', null);
  const [pathSource, setPathSource] = usePageNavigationState<string | null>(
    'graph.pathSource',
    null,
  );
  const [pathTarget, setPathTarget] = usePageNavigationState<string | null>(
    'graph.pathTarget',
    null,
  );
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [collectionChoice, setCollectionChoice] = useState<SourceCollectionChoice | null>(null);
  const [collectionError, setCollectionError] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const canvas = useRef<CanvasHandle>(null);
  const tagFacetDetails = useRef<HTMLDetailsElement>(null);
  const tagOptions = useRef<HTMLDivElement>(null);

  const positionTagOptions = useCallback(() => {
    const details = tagFacetDetails.current;
    const options = tagOptions.current;
    const summary = details?.querySelector('summary');
    if (!details?.open || !(summary instanceof HTMLElement) || !options) return;
    const anchor = summary.getBoundingClientRect();
    const placement = placeGraphPopover(anchor, {
      width: window.innerWidth,
      height: window.innerHeight,
    });
    if (!placement) {
      details.open = false;
      return;
    }
    options.style.left = `${placement.left}px`;
    options.style.width = `${placement.width}px`;
    options.style.maxHeight = `${placement.maxHeight}px`;
    options.style.top = placement.top === null ? 'auto' : `${placement.top}px`;
    options.style.bottom = placement.bottom === null ? 'auto' : `${placement.bottom}px`;
  }, []);

  useEffect(() => {
    const update = () => positionTagOptions();
    const outside = (event: PointerEvent) => {
      const details = tagFacetDetails.current;
      if (details?.open && event.target instanceof Node && !details.contains(event.target)) {
        details.open = false;
      }
    };
    const escape = (event: KeyboardEvent) => {
      const details = tagFacetDetails.current;
      if (event.key !== 'Escape' || !details?.open) return;
      event.preventDefault();
      details.open = false;
      details.querySelector('summary')?.focus();
    };
    window.addEventListener('resize', update);
    document.addEventListener('scroll', update, true);
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('keydown', escape, true);
    return () => {
      window.removeEventListener('resize', update);
      document.removeEventListener('scroll', update, true);
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('keydown', escape, true);
    };
  }, [positionTagOptions]);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setError(null);
    call({ kind: 'knowledge_graph' }, 'knowledge_graph', identity, 20000).then(
      (value) => {
        if (!live) return;
        setGraph(value);
        setLoading(false);
      },
      (failure: unknown) => {
        if (!live) return;
        setError(message(failure));
        setLoading(false);
      },
    );
    return () => {
      live = false;
    };
  }, [identity.local_instance_id, identity.workspace_id, attempt]);

  useEffect(() => {
    setCollectionError('');
    setCollectionChoice(null);
    if (!collectionId) return;
    let live = true;
    void call(
      { kind: 'collections_get', payload: { collection_id: collectionId } },
      'collection',
      identity,
    )
      .then((collection) => {
        if (live) setCollectionChoice({ id: collection.id, name: collection.name });
      })
      .catch((failure) => {
        if (live) setCollectionError(message(failure));
      });
    return () => {
      live = false;
    };
  }, [collectionId, identity.local_instance_id, identity.workspace_id, attempt]);

  const collectionNodes = useMemo(
    () =>
      (graph?.nodes ?? []).filter(
        (node) => !collectionId || (node.collection_ids ?? []).includes(collectionId),
      ),
    [collectionId, graph],
  );
  // A category view is about every object in that category, including objects
  // that currently have no visible relation, so do not hide them as orphans.
  useEffect(() => {
    if (collectionId) setOrphans(true);
  }, [collectionId, setOrphans]);
  useEffect(() => {
    if (!collectionId || !graph) return;
    const available = new Set<string>();
    for (const node of collectionNodes) for (const id of node.tag_ids) available.add(id);
    setTagFilter((current) =>
      current.every((id) => available.has(id))
        ? current
        : current.filter((id) => available.has(id)),
    );
  }, [collectionId, collectionNodes, graph, setTagFilter]);
  useEffect(() => {
    if (!collectionId || !graph) return;
    const inCategory = (id: string | null) => {
      if (!id) return false;
      const node = graph.nodes.find((candidate) => candidate.id === id);
      return Boolean((node?.collection_ids ?? []).includes(collectionId));
    };
    if (selectedId && !inCategory(selectedId)) setSelectedId(null);
    if (pathSource && !inCategory(pathSource)) setPathSource(null);
    if (pathTarget && !inCategory(pathTarget)) setPathTarget(null);
  }, [
    collectionId,
    graph,
    pathSource,
    pathTarget,
    selectedId,
    setPathSource,
    setPathTarget,
    setSelectedId,
  ]);
  const tagFacets = useMemo(() => {
    const counts = new Map<string, number>();
    for (const node of collectionNodes) {
      for (const id of new Set(node.tag_ids)) counts.set(id, (counts.get(id) ?? 0) + 1);
    }
    return (graph?.tags ?? [])
      .map((tag) => ({ ...tag, count: counts.get(tag.id) ?? 0 }))
      .filter((tag) => tag.count > 0)
      .sort((a, b) => b.count - a.count || a.display_name.localeCompare(b.display_name));
  }, [collectionNodes, graph]);
  const filteredNodes = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const allowedKinds = new Set(kinds);
    return collectionNodes.filter(
      (node) =>
        allowedKinds.has(node.kind) &&
        tagFilter.every((tagId) => node.tag_ids.includes(tagId)) &&
        (!needle || node.title.toLocaleLowerCase().includes(needle)),
    );
  }, [collectionNodes, kinds, query, tagFilter]);
  const filteredEdges = useMemo(() => {
    const allowed = new Set(filteredNodes.map((node) => node.id));
    return (graph?.edges ?? []).filter(
      (edge) => allowed.has(edge.source_id) && allowed.has(edge.target_id),
    );
  }, [filteredNodes, graph]);
  const visibleNodes = useMemo(() => {
    const linked = new Set<string>();
    for (const edge of filteredEdges) {
      linked.add(edge.source_id);
      linked.add(edge.target_id);
    }
    const visible = orphans
      ? filteredNodes
      : filteredNodes.filter((node) => node.degree > 0 || linked.has(node.id));
    if (!focus || !selectedId || (pathSource && pathTarget)) return visible;
    const neighbourhood = new Set<string>([selectedId]);
    for (const edge of filteredEdges) {
      if (edge.source_id === selectedId) neighbourhood.add(edge.target_id);
      if (edge.target_id === selectedId) neighbourhood.add(edge.source_id);
    }
    return visible.filter((node) => neighbourhood.has(node.id));
  }, [filteredEdges, filteredNodes, focus, orphans, pathSource, pathTarget, selectedId]);
  const pathNodeIds = useMemo(
    () => shortestGraphPath(filteredNodes, filteredEdges, pathSource, pathTarget) ?? [],
    [filteredEdges, filteredNodes, pathSource, pathTarget],
  );
  const visibleIds = useMemo(() => new Set(visibleNodes.map((node) => node.id)), [visibleNodes]);
  const visibleEdges = useMemo(
    () =>
      filteredEdges.filter(
        (edge) => visibleIds.has(edge.source_id) && visibleIds.has(edge.target_id),
      ),
    [filteredEdges, visibleIds],
  );
  const selected = visibleNodes.find((node) => node.id === selectedId) ?? null;
  const selectedEdges = useMemo(() => {
    if (!selected) return [];
    return visibleEdges.filter(
      (edge) => edge.source_id === selected.id || edge.target_id === selected.id,
    );
  }, [selected, visibleEdges]);
  const neighbours = useMemo(() => {
    if (!selected) return [];
    const ids = new Set<string>();
    for (const edge of visibleEdges) {
      if (edge.source_id === selected.id) ids.add(edge.target_id);
      if (edge.target_id === selected.id) ids.add(edge.source_id);
    }
    return visibleNodes.filter((node) => ids.has(node.id));
  }, [selected, visibleEdges, visibleNodes]);

  function toggleKind(kind: GraphKind) {
    setKinds((current) => {
      const active = current.includes(kind);
      if (active && current.length === 1) return current;
      return active ? current.filter((item) => item !== kind) : [...current, kind];
    });
  }
  function toggleTag(tagId: string) {
    setTagFilter((current) =>
      current.includes(tagId) ? current.filter((item) => item !== tagId) : [...current, tagId],
    );
  }
  function clearPath() {
    setPathSource(null);
    setPathTarget(null);
  }
  function nodeKindLabel(kind: GraphKind) {
    switch (kind) {
      case 'note':
        return t('笔记', 'Note');
      case 'block':
        return t('内容块', 'Block');
      case 'source':
        return t('资料', 'Source');
      case 'annotation':
        return t('批注', 'Annotation');
      case 'canvas':
        return t('画布', 'Canvas');
      default:
        return t('对象', 'Object');
    }
  }
  function edgeKindLabel(kind: KnowledgeGraphEdge['kind']) {
    if (kind === 'relation') return t('关联字段', 'Relation field');
    if (kind === 'block_reference') return t('块引用', 'Block reference');
    if (kind === 'canvas') return t('画布成员', 'Canvas member');
    return t('正文引用', 'Body reference');
  }
  function kindIcon(kind: GraphKind) {
    switch (kind) {
      case 'source':
        return <FileText size={14} />;
      case 'annotation':
        return <Highlighter size={14} />;
      case 'block':
        return <Pilcrow size={14} />;
      case 'canvas':
        return <LayoutDashboard size={14} />;
      default:
        return <Network size={14} />;
    }
  }
  function openNode(node: KnowledgeGraphNode) {
    if (node.kind === 'note' && node.object_id) {
      onOpenNote(node.object_id);
      return;
    }
    if (node.kind === 'canvas' && node.object_id) {
      onOpenCanvas?.(node.object_id);
      return;
    }
    if (onOpenTarget && node.object_id) {
      void onOpenTarget({
        target_id: node.object_id,
        target_block_id: node.kind === 'block' ? node.block_id : null,
        target_revision_id: null,
      });
    }
  }
  const totalLinks = visibleEdges.reduce((sum, edge) => sum + edge.weight, 0);
  const pathStart = graph?.nodes.find((node) => node.id === pathSource) ?? null;
  const pathEnd = graph?.nodes.find((node) => node.id === pathTarget) ?? null;
  const pathMissing = Boolean(pathSource && pathTarget && pathNodeIds.length === 0);
  const pathPreview = pathNodeIds.length > 200 ? pathNodeIds.slice(0, 200) : pathNodeIds;
  const viewportKey = JSON.stringify([
    collectionId,
    query,
    [...kinds].sort(),
    [...tagFilter].sort(),
    orphans,
    focus,
    pathSource,
    pathTarget,
  ]);
  const [edgePage, setEdgePage] = usePageNavigationState(`graph.edge-page/${selectedId}`, 1);
  const [edgeSize, setEdgeSize] = usePageNavigationState('graph.edge-size', 20);
  const [nodePage, setNodePage] = usePageNavigationState(`graph.node-page/${viewportKey}`, 1);
  const [nodeSize, setNodeSize] = usePageNavigationState('graph.node-size', 20);
  const edgeIndex = Math.min(edgePage, Math.max(1, Math.ceil(selectedEdges.length / edgeSize)));
  const nodeIndex = Math.min(nodePage, Math.max(1, Math.ceil(visibleNodes.length / nodeSize)));
  const edgeRows = selectedEdges.slice((edgeIndex - 1) * edgeSize, edgeIndex * edgeSize);
  const nodeRows = visibleNodes.slice((nodeIndex - 1) * nodeSize, nodeIndex * nodeSize);
  const [nodeIndexOpen, setNodeIndexOpen] = usePageNavigationState<boolean | null>(
    'graph.node-index-open',
    null,
  );
  const [nodeScroll, setNodeScroll] = usePageNavigationState(
    `graph.node-scroll/${viewportKey}/${nodeIndex}`,
    0,
  );
  const [edgeScroll, setEdgeScroll] = usePageNavigationState(
    `graph.edge-scroll/${selectedId}/${edgeIndex}`,
    0,
  );
  const nodeList = useRef<HTMLDivElement>(null);
  const edgeList = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (nodeList.current) nodeList.current.scrollTop = nodeScroll;
  }, [nodeScroll, nodeIndex, viewportKey, nodeRows.length]);
  useLayoutEffect(() => {
    if (edgeList.current) edgeList.current.scrollTop = edgeScroll;
  }, [edgeScroll, edgeIndex, selectedId, edgeRows.length]);

  return (
    <PageFrame
      className="org-page graph-page"
      title={t('知识图谱', 'Knowledge graph')}
      description={t(
        '探索知识之间的关系，沿着连接回到原文。',
        'Explore how your knowledge connects and follow links to the original content.',
      )}
      actions={
        <button
          className="secondary-button"
          aria-label={t('重新生成', 'Refresh graph')}
          disabled={loading}
          onClick={() => setAttempt((value) => value + 1)}
        >
          <RefreshCw size={15} className={loading ? 'spin' : undefined} />
          <span className="graph-refresh-label">{t('重新生成', 'Refresh graph')}</span>
        </button>
      }
      toolbar={
        <div className="graph-toolbar" role="toolbar" aria-label={t('图谱筛选', 'Graph filters')}>
          <div className="graph-collection-filter">
            <FolderClosed size={15} aria-hidden="true" />
            <SourceCollectionPicker
              identity={identity}
              label={t('按分类筛选图谱', 'Filter graph by category')}
              emptyLabel={t('全部分类', 'All categories')}
              value={
                collectionId
                  ? {
                      id: collectionId,
                      name:
                        collectionChoice?.id === collectionId
                          ? collectionChoice.name
                          : t('当前分类', 'Current category'),
                    }
                  : null
              }
              disabled={false}
              generation={attempt}
              onChange={(value) => {
                setCollectionId(value?.id ?? null);
                setCollectionChoice(value);
                setSelectedId(null);
                setPathSource(null);
                setPathTarget(null);
              }}
            />
          </div>
          <label className="graph-search">
            <Search size={15} aria-hidden="true" />
            <input
              type="search"
              value={query}
              maxLength={200}
              placeholder={t('按标题查找节点', 'Find a node by title')}
              aria-label={t('搜索图谱节点', 'Search graph nodes')}
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <div className="graph-kind-filters" role="group" aria-label={t('节点类型', 'Node types')}>
            {KIND_ORDER.map((kind) => (
              <button
                key={kind}
                type="button"
                className="graph-filter-chip"
                aria-pressed={kinds.includes(kind)}
                data-kind={kind}
                onClick={() => toggleKind(kind)}
              >
                {kindIcon(kind)}
                {nodeKindLabel(kind)}
              </button>
            ))}
          </div>
          {tagFacets.length > 0 && (
            <details
              ref={tagFacetDetails}
              className="graph-tag-facets"
              onToggle={(event) => {
                if (event.currentTarget.open) requestAnimationFrame(positionTagOptions);
              }}
            >
              <summary>
                {t('标签筛选', 'Tag filters')}
                {tagFilter.length > 0 && <span>{tagFilter.length}</span>}
              </summary>
              <div
                ref={tagOptions}
                className="graph-tag-options"
                role="group"
                aria-label={t('按标签筛选节点', 'Filter nodes by tag')}
              >
                {tagFacets.map((tag) => (
                  <button
                    key={tag.id}
                    type="button"
                    className="graph-filter-chip graph-tag-chip"
                    aria-pressed={tagFilter.includes(tag.id)}
                    title={tag.display_name}
                    onClick={() => toggleTag(tag.id)}
                  >
                    <span># {tag.display_name}</span>
                    <small>{tag.count}</small>
                  </button>
                ))}
                {tagFilter.length > 0 && (
                  <button
                    type="button"
                    className="graph-link-button"
                    onClick={() => setTagFilter([])}
                  >
                    <X size={13} /> {t('清除标签', 'Clear tags')}
                  </button>
                )}
              </div>
            </details>
          )}
          <label className="graph-check">
            <input
              type="checkbox"
              checked={orphans}
              onChange={(event) => setOrphans(event.target.checked)}
            />
            {t('显示孤立节点', 'Show orphans')}
          </label>
          <label className="graph-check">
            <input
              type="checkbox"
              checked={labels}
              onChange={(event) => setLabels(event.target.checked)}
            />
            {t('显示全部标签', 'Show all labels')}
          </label>
          <label className="graph-check">
            <input
              type="checkbox"
              checked={focus}
              disabled={!selected && !focus}
              onChange={(event) => setFocus(event.target.checked)}
            />
            {t('聚焦所选邻居', 'Focus selected neighbourhood')}
          </label>
          <div className="graph-zoom-actions" role="group" aria-label={t('图谱缩放', 'Graph zoom')}>
            <button
              type="button"
              className="icon-button"
              aria-label={
                fullscreen.active
                  ? t('退出全屏', 'Exit fullscreen')
                  : t('全屏图谱', 'Fullscreen graph')
              }
              onClick={() =>
                void fullscreen.toggle().catch((failure) => setError(message(failure)))
              }
            >
              {fullscreen.active ? <Minimize size={16} /> : <Maximize size={16} />}
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label={t('缩小', 'Zoom out')}
              onClick={() => canvas.current?.zoomOut()}
            >
              <Minus size={16} />
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label={t('放大', 'Zoom in')}
              onClick={() => canvas.current?.zoomIn()}
            >
              <Plus size={16} />
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label={t('适合窗口', 'Fit to window')}
              onClick={() => canvas.current?.fit()}
            >
              <Maximize2 size={16} />
            </button>
          </div>
        </div>
      }
    >
      {collectionError && (
        <p className="graph-filter-error" role="alert">
          {collectionError}{' '}
          <button className="text-button" onClick={() => setAttempt((value) => value + 1)}>
            {t('重试读取分类', 'Retry loading category')}
          </button>
        </p>
      )}
      <div className="graph-layout">
        <div className="graph-stage">
          {graph && visibleNodes.length > 0 && (
            <GraphCanvas
              key={viewportKey}
              viewportKey={viewportKey}
              ref={canvas}
              nodes={visibleNodes}
              edges={visibleEdges}
              selectedId={selectedId}
              showAllLabels={labels}
              pathNodeIds={pathNodeIds}
              onSelect={setSelectedId}
              onOpen={openNode}
            />
          )}
          {loading && (
            <div className="graph-overlay" role="status">
              <LoaderCircle className="spin" size={24} />
              <p>{t('正在整理知识关系…', 'Assembling knowledge links…')}</p>
            </div>
          )}
          {!loading && error && (
            <div className="graph-overlay graph-error" role="alert">
              <AlertCircle size={24} />
              <p>{error}</p>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setAttempt((value) => value + 1)}
              >
                <RefreshCw size={15} /> {t('重试', 'Retry')}
              </button>
            </div>
          )}
          {!loading && !error && graph && visibleNodes.length === 0 && (
            <div className="graph-overlay">
              <Network size={28} strokeWidth={1.5} />
              <h2>
                {graph.nodes.length
                  ? t('没有符合筛选的节点', 'No nodes match these filters')
                  : t('还没有可绘制的关系', 'No links to draw yet')}
              </h2>
              <p>
                {graph.nodes.length
                  ? t(
                      '调整搜索词、节点类型、标签或显示孤立节点。',
                      'Adjust the search, node types, tags or include orphans.',
                    )
                  : t(
                      '在笔记正文中插入双链引用后，关系会出现在这里。',
                      'Insert a note reference in a body to see it here.',
                    )}
              </p>
            </div>
          )}
          {graph?.truncated && (
            <p className="graph-truncated" role="status">
              {t(
                `为保证流畅，仅显示前 ${graph.node_limit.toLocaleString()} 个节点和前 ${graph.edge_limit.toLocaleString()} 组关系。`,
                `For responsive rendering, only the first ${graph.node_limit.toLocaleString()} nodes and ${graph.edge_limit.toLocaleString()} links are shown.`,
              )}
            </p>
          )}
          <div className="graph-stats" aria-hidden="true">
            <span>{t(`${visibleNodes.length} 个节点`, `${visibleNodes.length} nodes`)}</span>
            <span>{t(`${visibleEdges.length} 组连接`, `${visibleEdges.length} links`)}</span>
            <span>{t(`${totalLinks} 次引用`, `${totalLinks} references`)}</span>
          </div>
        </div>
        <aside className="graph-inspector" aria-label={t('图谱节点详情', 'Graph node details')}>
          <div className="graph-path-finder" aria-label={t('最短路径', 'Shortest path')}>
            <div className="graph-path-heading">
              <h2>
                <Route size={16} /> {t('最短路径', 'Shortest path')}
              </h2>
              {(pathSource || pathTarget) && (
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t('清除路径端点', 'Clear path endpoints')}
                  onClick={clearPath}
                >
                  <X size={15} />
                </button>
              )}
            </div>
            <p className="muted">
              {t(
                '先选择一个节点，再分别标记起点和终点。',
                'Select a node, then mark it as the start or end.',
              )}
            </p>
            <div className="graph-path-endpoints">
              <div className="graph-path-endpoint" data-tone="start">
                <small>{t('起点', 'Start')}</small>
                <strong>{pathStart?.title ?? t('未设置', 'Not set')}</strong>
                {pathStart && (
                  <button
                    type="button"
                    aria-label={t('清除起点', 'Clear start')}
                    onClick={() => setPathSource(null)}
                  >
                    <X size={12} />
                  </button>
                )}
              </div>
              <div className="graph-path-endpoint" data-tone="end">
                <small>{t('终点', 'End')}</small>
                <strong>{pathEnd?.title ?? t('未设置', 'Not set')}</strong>
                {pathEnd && (
                  <button
                    type="button"
                    aria-label={t('清除终点', 'Clear end')}
                    onClick={() => setPathTarget(null)}
                  >
                    <X size={12} />
                  </button>
                )}
              </div>
            </div>
            {selected && (
              <div className="graph-path-actions">
                <button
                  type="button"
                  className="secondary-button"
                  disabled={pathSource === selected.id}
                  onClick={() => setPathSource(selected.id)}
                >
                  {t('设为起点', 'Set as start')}
                </button>
                <button
                  type="button"
                  className="secondary-button"
                  disabled={pathTarget === selected.id}
                  onClick={() => setPathTarget(selected.id)}
                >
                  {t('设为终点', 'Set as end')}
                </button>
                <button
                  type="button"
                  className="secondary-button"
                  disabled={!pathSource || !pathTarget}
                  onClick={() => {
                    setPathSource(pathTarget);
                    setPathTarget(pathSource);
                  }}
                >
                  {t('交换', 'Swap')}
                </button>
              </div>
            )}
            {pathNodeIds.length > 0 && (
              <div className="graph-path-result">
                <p className="graph-path-summary" role="status">
                  {t(
                    `${pathNodeIds.length} 个节点 · ${pathNodeIds.length - 1} 步`,
                    `${pathNodeIds.length} ${pathNodeIds.length === 1 ? 'node' : 'nodes'} · ${pathNodeIds.length - 1} ${pathNodeIds.length - 1 === 1 ? 'step' : 'steps'}`,
                  )}
                </p>
                <ol aria-label={t('路径节点', 'Path nodes')}>
                  {pathPreview.map((nodeId, index) => {
                    const node = graph?.nodes.find((item) => item.id === nodeId);
                    return (
                      <li key={`${nodeId}-${index}`}>
                        <button
                          type="button"
                          className="graph-node-row"
                          onClick={() => setSelectedId(nodeId)}
                        >
                          <span data-kind={node?.kind ?? 'note'}>
                            {kindIcon(node?.kind ?? 'note')}
                          </span>
                          <span>
                            <strong>{node?.title ?? nodeId}</strong>
                            <small>
                              {index + 1} / {pathNodeIds.length}
                            </small>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ol>
                {pathNodeIds.length > pathPreview.length && (
                  <p className="muted">
                    {t('列表仅显示前 200 个路径节点。', 'The list shows the first 200 path nodes.')}
                  </p>
                )}
              </div>
            )}
            {pathMissing && (
              <p className="graph-path-empty" role="status">
                {t(
                  '当前筛选下没有连通路径。',
                  'No connected path exists under the current filters.',
                )}
              </p>
            )}
          </div>
          {selected ? (
            <div className="graph-selected">
              <span className="graph-kind-label" data-kind={selected.kind}>
                {kindIcon(selected.kind)}
                {nodeKindLabel(selected.kind)}
              </span>
              <h2>{selected.title}</h2>
              <p>
                {t(
                  `${selected.degree} 个关联节点`,
                  `${selected.degree} connected ${selected.degree === 1 ? 'node' : 'nodes'}`,
                )}
              </p>
              <button type="button" className="primary-button" onClick={() => openNode(selected)}>
                <SquareArrowOutUpRight size={15} /> {t('打开原对象', 'Open original')}
              </button>
              <h3>{t('邻近节点', 'Neighbours')}</h3>
              {neighbours.length ? (
                <div className="graph-neighbour-list">
                  {neighbours.map((node) => (
                    <button
                      key={node.id}
                      type="button"
                      className="graph-node-row"
                      onClick={() => setSelectedId(node.id)}
                    >
                      <span data-kind={node.kind}>{kindIcon(node.kind)}</span>
                      <span>
                        <strong>{node.title}</strong>
                        <small>
                          {nodeKindLabel(node.kind)} · {node.degree}
                        </small>
                      </span>
                    </button>
                  ))}
                </div>
              ) : (
                <p className="muted">
                  {t('此节点没有可见邻居。', 'This node has no visible neighbours.')}
                </p>
              )}
              <h3>{t('关系依据', 'Relation evidence')}</h3>
              {selectedEdges.length ? (
                <div
                  className="graph-paged-list"
                  role="group"
                  aria-label={t('关系依据列表', 'Relation evidence list')}
                >
                  <div
                    className="graph-neighbour-list"
                    ref={edgeList}
                    onScroll={(event) => setEdgeScroll(event.currentTarget.scrollTop)}
                  >
                    {edgeRows.map((edge) => {
                      const otherId =
                        edge.source_id === selected.id ? edge.target_id : edge.source_id;
                      const other = visibleNodes.find((node) => node.id === otherId);
                      return (
                        <button
                          key={`${edge.source_id}/${edge.target_id}/${edge.kind}`}
                          type="button"
                          className="graph-node-row"
                          onClick={() => setSelectedId(otherId)}
                        >
                          <span data-kind={other?.kind ?? 'note'}>
                            {kindIcon(other?.kind ?? 'note')}
                          </span>
                          <span>
                            <strong>{other?.title || t('未命名节点', 'Untitled node')}</strong>
                            <small>
                              {edgeKindLabel(edge.kind)} · {edge.weight}
                              {edge.context ? ` · ${edge.context}` : ''}
                            </small>
                          </span>
                        </button>
                      );
                    })}
                  </div>
                  {(selectedEdges.length > 20 || edgeSize !== 20) && (
                    <Pagination
                      index={edgeIndex}
                      size={edgeSize}
                      count={edgeRows.length}
                      total={selectedEdges.length}
                      canPrevious={edgeIndex > 1}
                      canNext={edgeIndex * edgeSize < selectedEdges.length}
                      onPrevious={() => setEdgePage(edgeIndex - 1)}
                      onNext={() => setEdgePage(edgeIndex + 1)}
                      onGoTo={setEdgePage}
                      onSize={(size) => {
                        setEdgeSize(size);
                        setEdgePage(1);
                      }}
                    />
                  )}
                </div>
              ) : (
                <p className="muted">
                  {t('没有可显示的引用依据。', 'No relation evidence is available.')}
                </p>
              )}
            </div>
          ) : (
            <div className="graph-selected-empty">
              <Focus size={24} strokeWidth={1.5} />
              <h2>{t('选择一个节点', 'Select a node')}</h2>
              <p>
                {t(
                  '单击画布节点查看关系和打开原对象。',
                  'Select a node on the canvas to inspect its links and open the original object.',
                )}
              </p>
            </div>
          )}
          {visibleNodes.length > 0 && (
            <details
              className="graph-node-index"
              open={nodeIndexOpen ?? visibleNodes.length <= 24}
              onToggle={(event) => setNodeIndexOpen(event.currentTarget.open)}
            >
              <summary>
                {t(
                  `可访问节点列表（${visibleNodes.length}）`,
                  `Accessible node list (${visibleNodes.length})`,
                )}
              </summary>
              <div
                className="graph-node-index-body graph-paged-list"
                role="group"
                aria-label={t('节点列表', 'Node list')}
              >
                <div
                  className="graph-node-index-list"
                  ref={nodeList}
                  onScroll={(event) => setNodeScroll(event.currentTarget.scrollTop)}
                >
                  {nodeRows.map((node) => (
                    <button
                      key={node.id}
                      type="button"
                      className={`graph-node-row ${node.id === selectedId ? 'active' : ''}`}
                      aria-current={node.id === selectedId ? 'true' : undefined}
                      onClick={() => setSelectedId(node.id)}
                      onDoubleClick={() => openNode(node)}
                    >
                      <span data-kind={node.kind}>{kindIcon(node.kind)}</span>
                      <span>
                        <strong>{node.title}</strong>
                        <small>
                          {nodeKindLabel(node.kind)} · {node.degree}
                        </small>
                      </span>
                    </button>
                  ))}
                </div>
                {(visibleNodes.length > 20 || nodeSize !== 20) && (
                  <Pagination
                    index={nodeIndex}
                    size={nodeSize}
                    count={nodeRows.length}
                    total={visibleNodes.length}
                    canPrevious={nodeIndex > 1}
                    canNext={nodeIndex * nodeSize < visibleNodes.length}
                    onPrevious={() => setNodePage(nodeIndex - 1)}
                    onNext={() => setNodePage(nodeIndex + 1)}
                    onGoTo={setNodePage}
                    onSize={(size) => {
                      setNodeSize(size);
                      setNodePage(1);
                    }}
                  />
                )}
              </div>
            </details>
          )}
        </aside>
      </div>
    </PageFrame>
  );
}
