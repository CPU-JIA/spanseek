import { ShieldCheck } from 'lucide-react';
import { useI18n } from '../i18n';
import './ai-trust.css';

export function AiTrustNote({
  mode,
  compact = false,
}: {
  mode: 'chat' | 'answers' | 'proposal';
  compact?: boolean;
}) {
  const { t } = useI18n();
  const copy =
    mode === 'chat'
      ? t(
          'AI 回复可能有误。保存到笔记前会先生成提案，只有你核对并明确应用后才会写入。',
          'AI responses can be wrong. Saving to a note creates a proposal first; nothing is written until you review and explicitly apply it.',
        )
      : mode === 'answers'
        ? t(
            '答案只依据本次冻结的证据，引用可回查；证据不足时会拒答。AI 仍可能误读，请核对后再保存。',
            'Answers use only the evidence frozen for this run, and citations can be reopened. Insufficient evidence produces a refusal. AI can still misread it; verify before saving.',
          )
        : t(
            '这是 AI 提案，不是已保存正文。请对照基准与建议；应用后会创建新的可恢复修订。',
            'This is an AI proposal, not saved note content. Compare the base and proposal; applying creates a new recoverable revision.',
          );
  return (
    <aside className={`ai-trust-note${compact ? ' ai-trust-note-compact' : ''}`} role="note">
      <ShieldCheck size={15} aria-hidden="true" />
      <p>{copy}</p>
    </aside>
  );
}
