import type { Note } from '@pkms/contracts';
import type { DraftController } from './draft-controller';
import type { DraftRecord } from './drafts';

export interface ConflictComparison {
  local: DraftRecord;
  current: Note;
}

export async function compareConflict(
  controller: DraftController,
  loadCurrent: () => Promise<Note>,
): Promise<ConflictComparison> {
  const snapshot = controller.getSnapshot();
  if (snapshot.status !== 'conflict' || snapshot.historyBusy || snapshot.record.historyIntent)
    throw new Error('当前草稿没有待处理的保存冲突。');
  const current = await loadCurrent();
  if (controller.getSnapshot().record !== snapshot.record)
    throw new Error('读取期间本地草稿已变化，请重新比较。');
  if (current.id !== snapshot.record.base.id) throw new Error('笔记身份不一致，草稿已保留。');
  return { local: snapshot.record, current };
}
