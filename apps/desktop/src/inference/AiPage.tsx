import { lazy, Suspense, type ComponentProps } from 'react';
import { AnswersPage } from './AnswersPage';
import { useI18n } from '../i18n';
import { usePageNavigationState } from '../layout/page-navigation-state';
import { SectionSwitch } from '../ui/SectionSwitch';
import type { TaskMaterialRequest } from './knowledge-task-navigation';
import './knowledge-tasks.css';

const ChatPage = lazy(() => import('./ChatPage').then((module) => ({ default: module.ChatPage })));
const KnowledgeTasksPage = lazy(() =>
  import('./KnowledgeTasksPage').then((module) => ({ default: module.KnowledgeTasksPage })),
);

export function AiPage(
  props: ComponentProps<typeof AnswersPage> & {
    taskMode: boolean;
    settingsRevision?: number;
    materialRequest?: TaskMaterialRequest | null;
    onMaterialsHandled?: (id: string) => void;
    onTaskModeChange: (enabled: boolean) => void;
    onOpenNote: (id: string) => Promise<boolean | void>;
  },
) {
  const { t } = useI18n();
  const [strict, setStrict] = usePageNavigationState(
    `ai.strict/${props.answerId ?? 'new'}`,
    !!props.answerId,
  );
  return (
    <div className="ai-workspace">
      <div className="ai-workspace-modes">
        <SectionSwitch
          label={t('AI 助手模式', 'AI assistant mode')}
          value={props.taskMode ? 'tasks' : strict ? 'strict' : 'chat'}
          options={[
            { value: 'chat', label: t('自由对话', 'Chat') },
            { value: 'strict', label: t('依证问答', 'Evidence answers') },
            { value: 'tasks', label: t('知识任务', 'Knowledge tasks') },
          ]}
          onChange={(value) => {
            if (value !== 'tasks') setStrict(value === 'strict');
            props.onTaskModeChange(value === 'tasks');
          }}
        />
      </div>
      <Suspense fallback={<p>{t('正在打开 AI 助手…', 'Opening AI assistant…')}</p>}>
        {props.taskMode ? (
          <KnowledgeTasksPage {...props} />
        ) : !strict ? (
          <ChatPage
            identity={props.identity}
            initialScope={props.initialScope}
            onOpenModels={props.onOpenModels}
            onOpenProposal={props.onOpenProposal}
            onStrict={() => setStrict(true)}
          />
        ) : (
          <AnswersPage {...props} />
        )}
      </Suspense>
    </div>
  );
}
