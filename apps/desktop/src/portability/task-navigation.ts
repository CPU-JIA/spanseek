import type { AcceptedAction } from './pending-actions';

export type TaskSection = 'sources' | 'exports' | 'operations' | 'research';
export type TaskDestination = { section: TaskSection; taskId?: string };
export type OpenTasks = (destination?: TaskDestination) => void;

/** The accepted ID belongs to the job/operation, not to its input command. */
export function acceptedTaskDestination(accepted: AcceptedAction): TaskDestination {
  return {
    section: accepted.kind === 'job' ? 'exports' : 'operations',
    taskId: accepted.id,
  };
}
