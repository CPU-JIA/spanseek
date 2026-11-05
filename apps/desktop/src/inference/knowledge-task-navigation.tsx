import { createContext, useContext, useEffect, useRef, useState } from 'react';
import { Sparkles } from 'lucide-react';
import type { KnowledgeTaskMaterialTarget, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { message } from '../api';

export type TaskMaterialRequest = {
  id: string;
  identity: WorkspaceIdentity;
  targets: KnowledgeTaskMaterialTarget[];
};
export const KnowledgeTaskNavigation = createContext<{
  start: (targets: KnowledgeTaskMaterialTarget[]) => void;
  open: (taskId: string) => void;
} | null>(null);
export const useKnowledgeTaskNavigation = () => useContext(KnowledgeTaskNavigation);

export function KnowledgeTaskButton({
  prepare,
  disabled = false,
}: {
  prepare: () => Promise<KnowledgeTaskMaterialTarget[]>;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const navigation = useKnowledgeTaskNavigation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  if (!navigation) return null;
  return (
    <span className="knowledge-task-trigger">
      <button
        className="icon-button"
        aria-label={t('基于此内容发起任务', 'Start a task with this content')}
        title={t('基于此内容发起任务', 'Start a task with this content')}
        disabled={disabled || busy}
        onClick={() => {
          setBusy(true);
          setError('');
          void prepare()
            .then((targets) => {
              if (live.current) navigation.start(targets);
            })
            .catch((error) => {
              if (live.current) setError(message(error));
            })
            .finally(() => {
              if (live.current) setBusy(false);
            });
        }}
      >
        <Sparkles size={16} />
      </button>
      {error && <span role="alert">{error}</span>}
    </span>
  );
}
