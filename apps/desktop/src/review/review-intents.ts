import type { DesktopCommand, WorkspaceIdentity } from '@pkms/contracts';

export type ReviewCommand = Extract<
  DesktopCommand,
  { kind: 'review_add' | 'review_complete' | 'review_snooze' | 'review_remove' }
>;
export type ReviewIntent = { identity: WorkspaceIdentity; command: ReviewCommand };
type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
export function validateReviewIntent(value: ReviewIntent, identity: WorkspaceIdentity) {
  if (
    !value ||
    value.identity?.local_instance_id !== identity.local_instance_id ||
    value.identity?.workspace_id !== identity.workspace_id ||
    !['review_add', 'review_complete', 'review_snooze', 'review_remove'].includes(
      value.command?.kind,
    ) ||
    typeof value.command?.payload?.command_id !== 'string' ||
    (value.command.kind === 'review_add'
      ? typeof value.command.payload.request?.target?.target_id !== 'string' ||
        (value.command.payload.request.base_row_version !== null &&
          !Number.isInteger(value.command.payload.request.base_row_version))
      : typeof value.command.payload.request?.entry_id !== 'string' ||
        !Number.isInteger(value.command.payload.request?.base_row_version))
  ) {
    throw new Error(
      '待确认回顾记录不匹配，请保留记录 / Pending review does not match this workspace',
    );
  }
}
export function storeReviewIntent(store: Store, key: string, value: ReviewIntent) {
  if (store.getItem(key)) throw new Error('仍有待确认回顾操作 / A review operation is pending');
  store.setItem(key, JSON.stringify(value));
}
export function confirmReviewIntent(store: Store, key: string, value: ReviewIntent) {
  if (store.getItem(key) !== JSON.stringify(value))
    throw new Error('回顾记录已变化 / Pending review changed');
  store.removeItem(key);
}
/** Interpret a date chosen by the user as local time, not UTC midnight. */
export function snoozeTimestamp(value: string, now = Date.now()): string {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))
    throw new Error('请选择完整日期和时间 / Choose a date and time');
  const date = new Date(value);
  const [year, month, day, hour, minute] = value.split(/[-T:]/).map(Number);
  if (
    !Number.isFinite(date.getTime()) ||
    date.getTime() <= now ||
    date.getFullYear() !== year ||
    date.getMonth() + 1 !== month ||
    date.getDate() !== day ||
    date.getHours() !== hour ||
    date.getMinutes() !== minute
  ) {
    throw new Error('请选择未来的有效时间 / Choose a valid future time');
  }
  return date.toISOString();
}
