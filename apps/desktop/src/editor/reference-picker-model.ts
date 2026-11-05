import type {
  AnnotationSummary,
  NoteBlockSummary,
  NoteSummary,
  ResolveLink,
  SourceBlock,
  SourceSummary,
} from '@pkms/contracts';

export type ReferenceCandidate =
  | { kind: 'note'; value: NoteSummary }
  | { kind: 'source'; value: SourceSummary }
  | { kind: 'annotation'; value: AnnotationSummary }
  | { kind: 'block'; value: NoteBlockSummary };
export type ReferenceDraft = {
  target: ResolveLink;
  label: string;
  preview?: string;
};

/** Source blocks belong to an immutable parse/version; never follow a newer source version. */
export function sourceBlockReference(
  source: SourceSummary,
  versionId: string,
  block: SourceBlock | null,
): ReferenceDraft {
  return {
    target: {
      target_id: source.id,
      target_revision_id: versionId,
      target_block_id: block?.id ?? null,
    },
    label: source.display_title,
    preview: block?.text,
  };
}

export function noteBlockReference(block: NoteBlockSummary): ReferenceDraft {
  return {
    target: {
      target_id: block.note_id,
      target_revision_id: null,
      target_block_id: block.id,
    },
    label: block.note_title,
    preview: block.text,
  };
}

export function annotationReference(annotation: AnnotationSummary): ReferenceDraft {
  return {
    target: {
      target_id: annotation.id,
      target_revision_id: annotation.source_version_id,
      target_block_id: null,
    },
    label: annotation.source_title,
    preview: annotation.quote_preview,
  };
}

/** The first loaded block of a PDF page supplies a stable UUID; text blocks remain individual. */
export function sourceReferenceBlocks(blocks: SourceBlock[], format: SourceSummary['format']) {
  if (format !== 'pdf') return blocks;
  const pages = new Set<number>();
  return blocks.filter((block) => {
    if (block.page_index === null || pages.has(block.page_index)) return false;
    pages.add(block.page_index);
    return true;
  });
}
