import { createContext, useContext, useEffect, useState, type ReactNode } from 'react';
import type { ResolveLink, ResolvedLink, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import type { OpenReference } from './reference-model';
import { ReferenceCache } from './reference-cache';
import './reference-ui.css';

type ReferenceContextValue = {
  identity: WorkspaceIdentity;
  generation: string;
  onOpenReference?: OpenReference;
};
const ReferenceContext = createContext<ReferenceContextValue | null>(null);
export function ReferenceProvider({
  children,
  ...value
}: ReferenceContextValue & { children: ReactNode }) {
  return <ReferenceContext.Provider value={value}>{children}</ReferenceContext.Provider>;
}
export const useReferenceContext = () => useContext(ReferenceContext);

const resolved = new ReferenceCache<ResolvedLink>();
const referenceKey = (identity: WorkspaceIdentity, target: ResolveLink) =>
  `${identity.local_instance_id}/${identity.workspace_id}/${JSON.stringify(target)}`;

/**
 * One resolution per target, shared by every chip, embed and picker.
 *
 * `fresh` asks the backend again instead of reusing a remembered answer: every
 * path that acts on the result (opening a target, validating an insertion)
 * passes it, while rendering the same reference again does not.
 */
export function resolveReference(
  identity: WorkspaceIdentity,
  target: ResolveLink,
  options: { fresh?: boolean } = {},
): Promise<ResolvedLink> {
  return resolved.resolve(
    referenceKey(identity, target),
    () => call({ kind: 'links_resolve', payload: { input: target } }, 'resolved_link', identity),
    options,
  );
}

/** A remembered answer, so a re-resolving chip renders without an empty frame. */
export function rememberedReference(
  identity: WorkspaceIdentity,
  target: ResolveLink,
): ResolvedLink | null {
  return resolved.peek(referenceKey(identity, target));
}

/** Forget one target, so the next resolve asks the backend again. */
export function invalidateReference(identity: WorkspaceIdentity, target: ResolveLink): void {
  resolved.invalidate(referenceKey(identity, target));
}
export function useResolvedReference(
  identity: WorkspaceIdentity,
  target: ResolveLink,
  generation: string,
) {
  const [state, setState] = useState<{
    key: string;
    result: ResolvedLink | null;
    error: string | null;
  }>({ key: '', result: null, error: null });
  const key = `${referenceKey(identity, target)}/${generation}`;
  const remembered = rememberedReference(identity, target);
  useEffect(() => {
    let live = true;
    void resolveReference(identity, target)
      .then((result) => {
        if (live) setState({ key, result, error: null });
      })
      .catch((error) => {
        if (live) setState({ key, result: null, error: message(error) });
      });
    return () => {
      live = false;
    };
  }, [key]);
  if (state.key === key) return state;
  return remembered ? { key, result: remembered, error: null } : { key, result: null, error: null };
}
