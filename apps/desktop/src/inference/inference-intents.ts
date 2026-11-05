import type { DesktopCommand, DesktopData, WorkspaceIdentity } from '@pkms/contracts';
export type InferenceIntent = {
  identity: WorkspaceIdentity;
  command: DesktopCommand;
  expected: DesktopData['kind'];
};
const expectedKinds: Record<string, DesktopData['kind']> = {
  knowledge_tasks_start: 'knowledge_task_mutation',
  knowledge_task_templates_save: 'knowledge_task_template_mutation',
  knowledge_task_templates_delete: 'knowledge_task_template_mutation',
  tags_create: 'tag_management_commit',
  tags_update: 'tag_management_commit',
  tags_delete: 'tag_management_commit',
  tags_assign: 'metadata_commit',
  tags_remove: 'metadata_commit',
  chat_conversations_create: 'chat_conversation_mutation',
  chat_runs_start: 'chat_run_mutation',
  chat_runs_cancel: 'chat_run_mutation',
  answers_start: 'answer_mutation',
  answers_agent_start: 'answer_mutation',
  answers_cancel: 'answer_mutation',
  answers_feedback: 'answer_mutation',
  proposals_create: 'proposal_mutation',
  proposals_apply: 'note_commit',
  proposals_reject: 'proposal_mutation',
  embedding_profile_register: 'embedding_profile_mutation',
  embedding_remote_configure: 'embedding_profile_mutation',
  generation_settings_set: 'generation_settings_mutation',
  generation_channel_save: 'generation_channel_mutation',
  generation_channel_delete: 'generation_channel_mutation',
  generation_channel_select: 'generation_settings_mutation',
  index_rebuild_start: 'index_rebuild_mutation',
  index_rebuild_cancel: 'index_rebuild_mutation',
};
function hasApiCredential(value: unknown): boolean {
  return (
    value !== null &&
    typeof value === 'object' &&
    Object.entries(value).some(([key, child]) => key === 'api_key' || hasApiCredential(child))
  );
}
export function validateInferenceIntent(intent: InferenceIntent, identity: WorkspaceIdentity) {
  if (
    !intent ||
    intent.identity?.local_instance_id !== identity.local_instance_id ||
    intent.identity?.workspace_id !== identity.workspace_id ||
    expectedKinds[intent.command?.kind] !== intent.expected ||
    !('payload' in intent.command) ||
    hasApiCredential(intent.command.payload) ||
    !('command_id' in intent.command.payload) ||
    typeof intent.command.payload.command_id !== 'string'
  )
    throw new Error('待确认请求不匹配，记录已保留 / Invalid pending request; record preserved');
}
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem' | 'removeItem'>;
export function storeInferenceIntent(storage: Storage, key: string, intent: InferenceIntent) {
  validateInferenceIntent(intent, intent.identity);
  if (storage.getItem(key))
    throw new Error('仍有待确认请求，请重新读取 / Pending request exists; reload');
  storage.setItem(key, JSON.stringify(intent));
}
export function confirmInferenceIntent(storage: Storage, key: string, intent: InferenceIntent) {
  if (storage.getItem(key) !== JSON.stringify(intent))
    throw new Error('待确认记录已变化，请重新读取 / Pending record changed; reload');
  storage.removeItem(key);
}
