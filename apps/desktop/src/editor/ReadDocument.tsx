import { useI18n } from '../i18n';
import { useEffect, useRef, useState } from 'react';
import type { Note, WorkspaceIdentity } from '@pkms/contracts';
import { DocumentContent } from '../rich/DocumentContent';
import { referenceAnchor, revealReference, type ReferenceLocation } from './reference-model';

export function ReadDocument({
  note,
  identity,
  referenceLocation,
  imageImports,
}: {
  note: Note;
  identity: WorkspaceIdentity;
  referenceLocation?: ReferenceLocation;
  imageImports?: Record<string, string>;
}) {
  const { t } = useI18n();
  const root = useRef<HTMLDivElement>(null);
  const [locationError, setLocationError] = useState<string | null>(null);
  useEffect(() => {
    setLocationError(null);
    if (!referenceLocation || !root.current || referenceLocation.target.target_id !== note.id)
      return;
    if (!referenceLocation.target.target_block_id && !referenceLocation.occurrenceId) return;
    let clear: (() => void) | undefined;
    const frame = requestAnimationFrame(() => {
      if (!root.current) return;
      const anchor = referenceAnchor(
        root.current,
        referenceLocation.target.target_block_id,
        referenceLocation.occurrenceId,
      );
      if (anchor) clear = revealReference(anchor);
      else
        setLocationError(
          t(
            '此版本中找不到目标段落或引用位置。',
            'The paragraph or reference occurrence is absent from this version.',
          ),
        );
    });
    return () => {
      cancelAnimationFrame(frame);
      clear?.();
    };
  }, [note.id, note.revision_id, referenceLocation?.requestId]);
  return (
    <div ref={root} className="note-prose read-document">
      {locationError && (
        <p className="inline-error" role="alert">
          {locationError}
        </p>
      )}
      <DocumentContent note={note} identity={identity} imageImports={imageImports} />
    </div>
  );
}
