import { call } from '../api';
import type { WorkspaceIdentity } from '@pkms/contracts';
import type { AutomationCommand, AutomationReplies } from './automation-intents';

const replies = {
  automation_create_change_set: 'change_set_commit',
  automation_apply_item: 'change_item_commit',
  automation_skip_item: 'change_item_commit',
  automation_reject_change_set: 'change_set_commit',
  research_create: 'research_commit',
  research_advance: 'research_commit',
  research_run_external: 'research_commit',
  research_cancel: 'research_commit',
  audit_undo: 'audit_undo_commit',
  review_insight_settings_set: 'review_settings_commit',
} as const;

export async function sendAutomationCommand<C extends AutomationCommand>(
  command: C,
  identity: WorkspaceIdentity,
): Promise<AutomationReplies[C['kind']]> {
  const result = await call(command, replies[command.kind], identity);
  // A dev-only loss after a real Core transaction; no fabricated success or data.
  if (import.meta.env.DEV) {
    const fixture = globalThis as { __PKMS_AUTOMATION_LOST_REPLIES__?: unknown };
    const losses = fixture.__PKMS_AUTOMATION_LOST_REPLIES__;
    if (Array.isArray(losses) && losses.includes(command.kind)) {
      fixture.__PKMS_AUTOMATION_LOST_REPLIES__ = losses.filter((kind) => kind !== command.kind);
      throw new Error('受控自动化回执丢失 / Controlled automation receipt loss');
    }
  }
  return result as AutomationReplies[C['kind']];
}
