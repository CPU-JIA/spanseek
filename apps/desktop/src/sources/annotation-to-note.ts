import type {
  Annotation,
  DocumentNode,
  NoteDocument,
  SourceLocator,
  WorkspaceIdentity,
} from '@pkms/contracts';
import type { DraftController } from '../editor/draft-controller.ts';

export interface AnnotationNoteIntent {
  schema: 1;
  key: string;
  identity: WorkspaceIdentity;
  annotationId: string;
  sourceVersionId: string;
  annotationVersion: number;
  /** Absent only in legacy single-excerpt actions. */
  locator?: SourceLocator;
  noteId: string;
  noteTitle: string;
  occurrenceId: string;
  nodes: DocumentNode[];
}
export interface AnnotationNoteBatchIntent {
  schema: 2;
  key: string;
  identity: WorkspaceIdentity;
  noteId: string;
  noteTitle: string;
  occurrenceId: string;
  items: AnnotationNoteIntent[];
}
export type AnnotationNoteAction = AnnotationNoteIntent | AnnotationNoteBatchIntent;
export const MAX_ANNOTATIONS_TO_NOTE = 100;
export const annotationNoteKey = (identity: WorkspaceIdentity, annotationId: string) =>
  `${identity.local_instance_id}/${identity.workspace_id}/${annotationId}`;
export const annotationNoteActionKey = (identity: WorkspaceIdentity, annotationIds: string[]) =>
  annotationIds.length === 1
    ? annotationNoteKey(identity, annotationIds[0])
    : annotationNoteKey(identity, `batch/${[...annotationIds].sort().join('+')}`);
export const annotationNoteItems = (action: AnnotationNoteAction): AnnotationNoteIntent[] =>
  action.schema === 1 ? [action] : action.items;

export function createAnnotationNoteIntent(
  identity: WorkspaceIdentity,
  annotation: Annotation,
  noteId: string,
  noteTitle: string,
  label: string,
): AnnotationNoteIntent {
  const paragraph = (text: string): DocumentNode => ({
    type: 'paragraph',
    attrs: { id: crypto.randomUUID() },
    content: text ? [{ type: 'text', text }] : [],
  });
  const occurrenceId = crypto.randomUUID();
  return {
    schema: 1,
    key: annotationNoteKey(identity, annotation.id),
    identity,
    annotationId: annotation.id,
    sourceVersionId: annotation.source_version_id,
    annotationVersion: annotation.row_version,
    locator: structuredClone(annotation.locator),
    noteId,
    noteTitle,
    occurrenceId,
    nodes: [
      {
        type: 'blockquote',
        attrs: { id: crypto.randomUUID() },
        content: annotation.locator.quote.split(/\r?\n/).map(paragraph),
      },
      {
        type: 'paragraph',
        attrs: { id: crypto.randomUUID() },
        content: [
          {
            type: 'noteReference',
            attrs: {
              target_id: annotation.id,
              target_block_id: null,
              target_revision_id: annotation.source_version_id,
              occurrence_id: occurrenceId,
              label:
                annotation.locator.kind === 'pdf'
                  ? `${label} · p. ${annotation.locator.page_label ?? annotation.locator.page_index + 1}`
                  : label,
            },
          },
        ],
      },
      ...annotation.comment
        .split(/\r?\n/)
        .filter((line) => line.trim())
        .map(paragraph),
    ],
  };
}
export function createAnnotationNoteAction(
  identity: WorkspaceIdentity,
  annotations: Annotation[],
  noteId: string,
  noteTitle: string,
  label: string,
): AnnotationNoteAction {
  if (
    !annotations.length ||
    annotations.length > MAX_ANNOTATIONS_TO_NOTE ||
    new Set(annotations.map((item) => item.id)).size !== annotations.length
  )
    throw new Error('请选择 1 至 100 条不同摘录。 / Select 1 to 100 distinct excerpts.');
  const items = annotations.map((item) =>
    createAnnotationNoteIntent(identity, item, noteId, noteTitle, label),
  );
  if (items.length === 1) return items[0];
  return {
    schema: 2,
    key: annotationNoteActionKey(
      identity,
      annotations.map((item) => item.id),
    ),
    identity,
    noteId,
    noteTitle,
    occurrenceId: crypto.randomUUID(),
    items,
  };
}
export function containsAnnotationIntent(
  document: NoteDocument,
  intent: AnnotationNoteIntent,
): boolean {
  const visit = (nodes: DocumentNode[]): boolean =>
    nodes.some(
      (node) =>
        (node.type === 'noteReference' &&
          node.attrs?.occurrence_id === intent.occurrenceId &&
          node.attrs.target_id === intent.annotationId) ||
        (node.content ? visit(node.content) : false),
    );
  return visit(document.content);
}
export function validateAnnotationNoteIntent(
  intent: AnnotationNoteIntent,
  key: string,
  identity: WorkspaceIdentity,
): void {
  const reference = intent?.nodes?.[1]?.content?.[0];
  if (
    intent?.schema !== 1 ||
    intent.key !== key ||
    annotationNoteKey(identity, intent.annotationId) !== key ||
    intent.identity?.local_instance_id !== identity.local_instance_id ||
    intent.identity?.workspace_id !== identity.workspace_id ||
    !Array.isArray(intent.nodes) ||
    !intent.noteId ||
    !intent.sourceVersionId ||
    !Number.isSafeInteger(intent.annotationVersion) ||
    intent.annotationVersion < 1 ||
    (intent.locator && intent.locator.source_version_id !== intent.sourceVersionId) ||
    (intent.locator && reference?.attrs?.target_revision_id !== intent.sourceVersionId) ||
    !intent.occurrenceId ||
    !containsAnnotationIntent({ type: 'doc', content: intent.nodes }, intent)
  )
    throw new Error(
      '摘录操作记录不匹配，已保留供恢复。 / The saved excerpt action does not match this workspace.',
    );
}

export function validateAnnotationNoteAction(
  action: AnnotationNoteAction,
  identity: WorkspaceIdentity,
): void {
  if (action?.schema === 1) {
    validateAnnotationNoteIntent(action, action.key, identity);
    return;
  }
  if (
    action?.schema !== 2 ||
    !Array.isArray(action.items) ||
    action.items.length < 2 ||
    action.items.length > MAX_ANNOTATIONS_TO_NOTE ||
    !action.occurrenceId ||
    action.identity?.local_instance_id !== identity.local_instance_id ||
    action.identity?.workspace_id !== identity.workspace_id ||
    action.key !==
      annotationNoteActionKey(
        identity,
        action.items.map((item) => item?.annotationId),
      ) ||
    new Set(action.items.map((item) => item?.annotationId)).size !== action.items.length ||
    new Set(action.items.map((item) => item?.occurrenceId)).size !== action.items.length
  )
    throw new Error(
      '批量摘录操作记录不匹配，已保留供恢复。 / The saved excerpt batch does not match.',
    );
  for (const item of action.items) {
    validateAnnotationNoteIntent(item, item?.key, identity);
    if (item.noteId !== action.noteId || item.noteTitle !== action.noteTitle)
      throw new Error(
        '批量摘录的目标笔记不一致。 / The batch contains different destination notes.',
      );
  }
}

/** Any selected member can reopen its entire durable batch after a restart. */
export function pendingAnnotationNoteAction(
  actions: AnnotationNoteAction[],
  identity: WorkspaceIdentity,
  annotationIds: string[],
): AnnotationNoteAction | undefined {
  const selected = new Set(annotationIds);
  const prefix = annotationNoteKey(identity, '');
  const matching = actions.filter((action) => {
    if (!action.key?.startsWith(prefix)) return false;
    if (action.key.startsWith(`${prefix}batch/`)) {
      validateAnnotationNoteAction(action, identity);
      return annotationNoteItems(action).some((item) => selected.has(item.annotationId));
    }
    return annotationIds.some((id) => annotationNoteKey(identity, id) === action.key);
  });
  for (const action of matching) validateAnnotationNoteAction(action, identity);
  if (matching.length > 1)
    throw new Error(
      '所选摘录有多项待确认操作，请先逐条打开摘录并确认保存。 / Selected excerpts have separate pending actions. Open each excerpt to confirm its save first.',
    );
  return matching[0];
}

export const containsAnyAnnotationIntent = (document: NoteDocument, action: AnnotationNoteAction) =>
  annotationNoteItems(action).some((item) => containsAnnotationIntent(document, item));

/** Reuses the one shared editor controller and its exact pending-save receipt protocol. */
export async function appendAnnotationToNote(
  controller: DraftController,
  intent: AnnotationNoteAction,
  verifySource: () => Promise<void>,
): Promise<void> {
  const finish = controller.beginAction();
  try {
    let snapshot = controller.getSnapshot();
    validateAnnotationNoteAction(intent, snapshot.record.identity);
    const items = annotationNoteItems(intent);
    const containsAll = (document: NoteDocument) =>
      items.every((item) => containsAnnotationIntent(document, item));
    const sameScope = () =>
      snapshot.record.base.id === intent.noteId &&
      snapshot.record.identity.local_instance_id === intent.identity.local_instance_id &&
      snapshot.record.identity.workspace_id === intent.identity.workspace_id;
    const checkAppend = () => {
      if (!sameScope() || snapshot.record.base.trashed)
        throw new Error('目标笔记已变化或不可用。 / The selected note is unavailable.');
      if (snapshot.status === 'conflict' || snapshot.record.historyIntent)
        throw new Error(
          '请先打开笔记处理版本冲突或待确认操作。 / Open the note to resolve its pending version action or conflict.',
        );
      if (containsAll(snapshot.record.document)) return false;
      if (
        containsAnyAnnotationIntent(snapshot.record.document, intent) ||
        containsAnyAnnotationIntent(snapshot.record.base.document, intent) ||
        (snapshot.record.pending &&
          containsAnyAnnotationIntent(snapshot.record.pending.request.document, intent))
      )
        throw new Error(
          '摘录已在草稿中被移除，请先打开笔记确认。 / This excerpt was removed from the draft. Open the note to review it.',
        );
      return true;
    };
    if (checkAppend()) {
      await verifySource();
      // Another pane may have edited the shared draft while the source was checked.
      snapshot = controller.getSnapshot();
      if (checkAppend())
        controller.update(snapshot.record.title, {
          ...snapshot.record.document,
          content: [
            ...snapshot.record.document.content,
            ...items.flatMap((item) => structuredClone(item.nodes)),
          ],
        });
    }
    await controller.flush();
    snapshot = controller.getSnapshot();
    if (
      snapshot.status !== 'saved' ||
      snapshot.record.pending ||
      snapshot.record.generation !== snapshot.record.acknowledged ||
      !containsAll(snapshot.record.base.document)
    )
      throw new Error(
        '保存尚未确认，请继续此操作。 / Saving is not yet confirmed. Retry this action.',
      );
  } finally {
    finish();
  }
}
