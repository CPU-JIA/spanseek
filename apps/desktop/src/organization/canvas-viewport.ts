import type { WorkspaceIdentity } from '@pkms/contracts';

export type CanvasTransform = { x: number; y: number; scale: number };
export type CanvasViewportPreference = {
  schema: 1;
  centerX: number;
  centerY: number;
  scale: number;
};

/** Per-library, per-device reading position; node geometry remains in the Rust canvas record. */
export function canvasViewportKey(identity: WorkspaceIdentity, canvasId: string) {
  return `spanseek/canvas-viewport/v1/${identity.local_instance_id}/${identity.workspace_id}/${canvasId}`;
}

export function readCanvasViewport(
  storage: Pick<Storage, 'getItem'>,
  key: string,
): CanvasViewportPreference | null {
  const raw = storage.getItem(key);
  if (raw === null) return null;
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== 'object') throw new Error('Invalid canvas viewport');
  const item = value as Partial<CanvasViewportPreference>;
  if (
    item.schema !== 1 ||
    !Number.isFinite(item.centerX) ||
    !Number.isFinite(item.centerY) ||
    Math.abs(item.centerX!) > 4_000_000 ||
    Math.abs(item.centerY!) > 4_000_000 ||
    !Number.isFinite(item.scale) ||
    item.scale! < 0.25 ||
    item.scale! > 2
  )
    throw new Error('Invalid canvas viewport');
  return { schema: 1, centerX: item.centerX!, centerY: item.centerY!, scale: item.scale! };
}

export function canvasViewportPreference(
  view: CanvasTransform,
  width: number,
  height: number,
): CanvasViewportPreference {
  return {
    schema: 1,
    centerX: (width / 2 - view.x) / view.scale,
    centerY: (height / 2 - view.y) / view.scale,
    scale: view.scale,
  };
}

export function restoreCanvasViewport(
  view: CanvasViewportPreference,
  width: number,
  height: number,
): CanvasTransform {
  return {
    x: width / 2 - view.centerX * view.scale,
    y: height / 2 - view.centerY * view.scale,
    scale: view.scale,
  };
}
