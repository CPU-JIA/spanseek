import type { Annotation, KnowledgeObject, NoteDocument, WorkspaceIdentity } from '@pkms/contracts';
import { annotationNoteItems, createAnnotationNoteAction } from './annotation-to-note.ts';

export type InheritedExcerptMetadata = {
  collectionIds: string[];
  collectionNames: string[];
  tagNames: string[];
};

export function excerptNoteDocument(
  identity: WorkspaceIdentity,
  annotations: Annotation[],
  title: string,
  label: string,
): NoteDocument {
  const action = createAnnotationNoteAction(
    identity,
    annotations,
    crypto.randomUUID(),
    title,
    label,
  );
  return {
    type: 'doc',
    content: annotationNoteItems(action).flatMap((item) => structuredClone(item.nodes)),
  };
}

export function inheritedExcerptMetadata(items: KnowledgeObject[]): InheritedExcerptMetadata {
  const collections = new Map<string, string>();
  const tags = new Set<string>();
  for (const item of items) {
    for (const collection of item.collections) {
      collections.set(collection.id, collection.name);
    }
    for (const tag of item.object.tags) {
      tags.add(tag.display_name);
    }
  }
  return {
    collectionIds: [...collections.keys()],
    collectionNames: [...collections.values()],
    tagNames: [...tags],
  };
}
