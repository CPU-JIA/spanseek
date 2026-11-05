import { createContext, useContext, type ReactNode } from 'react';
import type { Note, WorkspaceIdentity } from '@pkms/contracts';

export type RichDocumentContextValue = {
  note: Note | null;
  identity: WorkspaceIdentity;
  depth: number;
  visited: ReadonlySet<string>;
  imageImports?: Record<string, string>;
  renderNote: (note: Note, depth: number, visited: ReadonlySet<string>) => ReactNode;
};
export const RichDocumentContext = createContext<RichDocumentContextValue | null>(null);
export const useRichDocument = () => useContext(RichDocumentContext);
