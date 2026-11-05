import { memo } from 'react';
import type { SearchEvidence, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { StandaloneDocumentContent } from '../rich/DocumentContent';
import { useMarkdownDocument } from './useMarkdownDocument';

/** Historical messages do not reparse on each new streamed chunk or composer keystroke. */
export const ChatMarkdown = memo(function ChatMarkdown({
  text,
  identity,
  renderRich,
  materials,
}: {
  text: string;
  identity: WorkspaceIdentity;
  renderRich: boolean;
  materials?: readonly SearchEvidence[];
}) {
  const { t } = useI18n();
  const parsed = useMarkdownDocument(text, true, renderRich, materials);
  return (
    <div className="chat-markdown" aria-busy={parsed.pending}>
      {!!parsed.value?.unresolvedCitations?.length && (
        <p role="status">
          {t(
            '以下编号没有对应材料，保留为文字，请核对：',
            'These citations have no matching material and remain text. Check: ',
          )}
          {parsed.value.unresolvedCitations.map((n) => `[${n}]`).join(' ')}
        </p>
      )}
      {parsed.error && (
        <p role="alert">
          {t(
            '格式转换失败，以下保留原文：',
            'Formatting failed; the original text is retained below: ',
          )}
          {parsed.error}
        </p>
      )}
      {parsed.value ? (
        <StandaloneDocumentContent document={parsed.value.document} identity={identity} />
      ) : (
        <div style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{text}</div>
      )}
    </div>
  );
});
