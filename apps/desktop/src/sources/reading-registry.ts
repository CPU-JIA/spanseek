import type { PdfReadingView, SourceLocator, WorkspaceIdentity } from '@pkms/contracts';
import { call } from '../api';
import { getSourceActions } from './useSourceActions';
import { ReadingQueue } from './reading-queue';

const queues = new Map<string, { identity: WorkspaceIdentity; queue: ReadingQueue }>();
const workspaceKey = (identity: WorkspaceIdentity) =>
  JSON.stringify([identity.local_instance_id, identity.workspace_id]);
export function sourceReading(identity: WorkspaceIdentity, sourceId: string) {
  const key = JSON.stringify([workspaceKey(identity), sourceId]);
  let entry = queues.get(key);
  if (!entry) {
    const controller = getSourceActions(identity, `reader/${sourceId}`);
    async function settleActions() {
      await controller.load();
      if (controller.getSnapshot().busy)
        await new Promise<void>((resolve) => {
          const unsubscribe = controller.subscribe(() => {
            if (!controller.getSnapshot().busy) {
              unsubscribe();
              resolve();
            }
          });
        });
      if (controller.getSnapshot().pending) await controller.retry();
      if (controller.getSnapshot().locked)
        throw new Error(
          controller.getSnapshot().error ??
            'Reading position cannot be confirmed / 阅读位置暂无法确认',
        );
    }
    entry = {
      identity,
      queue: new ReadingQueue(
        async () => {
          await settleActions();
          return call(
            { kind: 'reading_get', payload: { source_id: sourceId } },
            'reading',
            identity,
          );
        },
        async (state, locator: SourceLocator, view: PdfReadingView | null, locatorReady) => {
          const success = await controller.execute(
            {
              kind: 'reading_set',
              payload: {
                command_id: crypto.randomUUID(),
                request: {
                  source_id: sourceId,
                  source_version_id: locator.source_version_id,
                  locator: locatorReady ? locator : null,
                  ...(view ? { pdf_view: view } : {}),
                  status: state.status === 'unread' ? 'reading' : state.status,
                  base_row_version: state.row_version,
                },
              },
            },
            'source_mutation',
          );
          if (!success)
            throw new Error(
              controller.getSnapshot().error ??
                'Reading position was not confirmed / 阅读位置尚未确认',
            );
        },
      ),
    };
    queues.set(key, entry);
  }
  return entry.queue;
}
/** Await this before tab/workspace close, app exit, or a consistent backup. */
export async function flushSourceReadings(identity: WorkspaceIdentity): Promise<void> {
  const key = workspaceKey(identity);
  await Promise.all(
    [...queues.values()]
      .filter((entry) => workspaceKey(entry.identity) === key)
      .map((entry) => entry.queue.flush()),
  );
}
