import { useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import type { Note, NoteCommit, ProposalRecord, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { ReadDocument } from '../editor/ReadDocument';
import { ReferenceProvider } from '../editor/reference-context';
import type { OpenReference } from '../editor/reference-model';
import { useInferenceAction } from './useInferenceAction';
import { PageFrame } from '../layout/PageFrame';
import './inference.css';
import { AiTrustNote } from './ai-trust';
import { ProposalApplyConfirmDialog } from './ai-apply-confirm';
import { proposalBlockChanges } from './proposal-changes';

export function ProposalPage({
  identity,
  proposalId,
  onOpenNote,
  withSavedNote,
  onOpenReference,
  onOpenTrash,
}: {
  identity: WorkspaceIdentity;
  proposalId: string;
  onOpenNote: (id: string) => Promise<boolean | void>;
  withSavedNote: (noteId: string | null, action: () => Promise<void>) => Promise<void>;
  onOpenReference?: OpenReference;
  onOpenTrash: () => void;
}) {
  const { t } = useI18n();
  const [proposal, setProposal] = useState<ProposalRecord | null>(null);
  const [base, setBase] = useState<Note | null>(null);
  const [current, setCurrent] = useState<Note | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [applying, setApplying] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const actions = useInferenceAction(identity, `proposal/${proposalId}`);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    let valid = true;
    setError(null);
    async function read() {
      try {
        const item = await call(
          { kind: 'proposals_get', payload: { proposal_id: proposalId } },
          'proposal',
          identity,
        );
        const [before, latest] =
          item.request.note_id && item.request.base_revision_id
            ? await Promise.all([
                call(
                  {
                    kind: 'revision_get',
                    payload: {
                      note_id: item.request.note_id,
                      revision_id: item.request.base_revision_id,
                    },
                  },
                  'note',
                  identity,
                ),
                call(
                  { kind: 'notes_get', payload: { note_id: item.request.note_id } },
                  'note',
                  identity,
                ),
              ])
            : [
                null,
                item.applied_note_id
                  ? await call(
                      { kind: 'notes_get', payload: { note_id: item.applied_note_id } },
                      'note',
                      identity,
                    )
                  : null,
              ];
        if (valid) {
          setProposal(item);
          setBase(before);
          setCurrent(latest);
        }
      } catch (e) {
        if (valid) setError(message(e));
      }
    }
    void read();
    return () => {
      valid = false;
      live.current = false;
    };
  }, [proposalId, reload]);
  async function apply(replay = false) {
    if (!proposal || applying) return;
    setApplying(true);
    setError(null);
    try {
      const submit = async () => {
        const result = replay
          ? await actions.retry<'note_commit'>()
          : await actions.execute(
              {
                kind: 'proposals_apply',
                payload: { command_id: crypto.randomUUID(), proposal_id: proposal.id },
              },
              'note_commit',
            );
        if (result && live.current) {
          setCurrent((result as NoteCommit).note);
          setReload((old) => old + 1);
        }
      };
      await withSavedNote(proposal.request.note_id, submit);
    } catch (e) {
      if (live.current) setError(message(e));
    } finally {
      if (live.current) setApplying(false);
    }
  }
  async function reject() {
    if (!proposal) return;
    const result = await actions.execute(
      {
        kind: 'proposals_reject',
        payload: { command_id: crypto.randomUUID(), proposal_id: proposal.id },
      },
      'proposal_mutation',
    );
    if (result && live.current) setProposal(result.proposal);
  }
  async function recover() {
    if (actions.pending?.command.kind === 'proposals_apply') {
      await apply(true);
      return;
    }
    const result = await actions.retry<'proposal_mutation'>();
    if (result && live.current) setProposal(result.proposal);
  }
  async function open() {
    if (!proposal) return;
    const target = proposal.request.note_id ?? proposal.applied_note_id;
    if (!target) return;
    try {
      await onOpenNote(target);
    } catch (e) {
      if (live.current) setError(message(e));
    }
  }
  const conflict =
    proposal &&
    proposal.request.note_id &&
    current &&
    current.revision_id !== proposal.request.base_revision_id &&
    proposal.status === 'pending';
  const targetTrashed = !!current?.trashed;
  const changes = proposal
    ? proposalBlockChanges(base?.document ?? null, proposal.request.document)
    : null;
  // A display projection of the persisted proposal, never a created note or a commit receipt.
  const preview: Note | null = proposal
    ? {
        ...(base ?? {
          id: proposal.id,
          revision: 0,
          revision_id: '',
          parent_revision_id: null,
          content_hash: '',
          author_kind: 'ai_proposal',
          trashed: false,
          created_at: proposal.created_at,
          updated_at: proposal.created_at,
        }),
        title: proposal.request.title,
        document: proposal.request.document,
        schema_version: proposal.request.document_schema,
      }
    : null;
  return (
    <PageFrame
      className="inference-page proposal-page-frame"
      title={t('AI 修改提案', 'AI change proposal')}
      description={t(
        '对照基准全文与建议全文，确认后整份应用。应用会创建新的可恢复版本。',
        'Compare the base and proposed documents, then explicitly apply the whole proposal. Application creates a recoverable revision.',
      )}
      actions={
        <button
          className="secondary-button"
          aria-label={t('重新读取提案与当前版本', 'Reload proposal and current revision')}
          title={t('重新读取提案与当前版本', 'Reload proposal and current revision')}
          onClick={() => setReload((old) => old + 1)}
        >
          <RefreshCw size={16} />
          {t('重新读取', 'Reload')}
        </button>
      }
    >
      {proposal?.status === 'pending' && <AiTrustNote mode="proposal" compact />}
      {(error || actions.error) && (
        <div className="inference-error" role="alert">
          {error || actions.error}
        </div>
      )}
      {(!actions.ready || actions.pending) && (
        <div className="inference-notice">
          <p>
            {t(
              '提案操作需要读取或重新确认，将使用原请求编号。',
              'A proposal operation needs reading or confirmation; its original request ID will be used.',
            )}
          </p>
          <button disabled={actions.busy || applying} onClick={() => void recover()}>
            {t('重新确认', 'Confirm again')}
          </button>
        </div>
      )}
      {proposal && preview ? (
        <>
          <section>
            <h2>{proposal.request.title}</h2>
            <p>
              {
                {
                  pending: t('等待审阅', 'Awaiting review'),
                  applied: t('已应用', 'Applied'),
                  rejected: t('已拒绝', 'Rejected'),
                }[proposal.status]
              }
            </p>
            {changes && (
              <>
                <p>
                  {t(
                    `新增 ${changes.added} 个段落块 · 修改 ${changes.changed} 个 · 移除 ${changes.removed} 个`,
                    `${changes.added} blocks added · ${changes.changed} changed · ${changes.removed} removed`,
                  )}
                </p>
                {changes.total > 0 ? (
                  <details className="proposal-change-list">
                    <summary>
                      {t(`变更块摘要 · ${changes.total}`, `Changed blocks · ${changes.total}`)}
                    </summary>
                    <ul>
                      {changes.changes.map((item) => (
                        <li key={`${item.kind}:${item.id}`}>
                          <span className={`proposal-change-kind proposal-change-${item.kind}`}>
                            {changeKindLabel(item.kind, t)}
                          </span>
                          <span className="proposal-change-preview">
                            {item.preview || proposalBlockTypeLabel(item.type, t)}
                          </span>
                          <small>{proposalBlockTypeLabel(item.type, t)}</small>
                        </li>
                      ))}
                    </ul>
                    {changes.hidden > 0 && (
                      <p className="muted">
                        {t(
                          `另有 ${changes.hidden} 个变更未在此展开；请继续对照下方全文。`,
                          `${changes.hidden} more changes are not expanded here; continue comparing the full documents below.`,
                        )}
                      </p>
                    )}
                  </details>
                ) : (
                  <p className="muted">
                    {t(
                      '正文块没有变化；标题或元数据可能不同，请对照下方全文。',
                      'No top-level block changes; the title or metadata may still differ. Compare the documents below.',
                    )}
                  </p>
                )}
              </>
            )}
            {conflict && (
              <div className="inference-error">
                {t(
                  '笔记已有更新版本。此提案不能覆盖新内容；请打开当前笔记核对，并重新创建提案。',
                  'The note has a newer revision. This proposal cannot overwrite it. Review the current note and create a new proposal.',
                )}
              </div>
            )}
            {targetTrashed && (
              <div className="inference-notice" role="status">
                <p>
                  {proposal.status === 'pending'
                    ? t(
                        '目标笔记在回收站中，暂时不能应用提案。先恢复笔记，再返回这里核对并保存。',
                        'The target note is in trash, so this proposal cannot be applied. Restore the note, then return here to review and save.',
                      )
                    : t(
                        '关联笔记在回收站中，恢复后可以继续打开；此提案的审阅记录保留。',
                        'The linked note is in trash. Restore it to open it again; this proposal’s review record is retained.',
                      )}
                </p>
                <button className="secondary-button" onClick={onOpenTrash}>
                  {t('前往回收站', 'Open trash')}
                </button>
              </div>
            )}
            <div className="inference-controls">
              <button
                className="primary-button"
                disabled={
                  actions.locked ||
                  applying ||
                  proposal.status !== 'pending' ||
                  !!conflict ||
                  targetTrashed
                }
                onClick={() => setConfirming(true)}
              >
                {applying
                  ? t('正在核对并应用…', 'Checking and applying…')
                  : t('明确应用全部修改', 'Apply all changes')}
              </button>
              <button
                className="secondary-button"
                disabled={actions.locked || applying || proposal.status !== 'pending'}
                onClick={() => void reject()}
              >
                {t('拒绝提案', 'Reject proposal')}
              </button>
              <button
                className="text-button"
                disabled={targetTrashed || (!proposal.request.note_id && !proposal.applied_note_id)}
                onClick={() => void open()}
              >
                {t('打开当前笔记', 'Open current note')}
              </button>
            </div>
            <p className="muted">
              {t(
                '应用会以同一基准核对并整份写入，不能逐块挑选；应用后可回到版本历史恢复。',
                'Application checks the same base and writes the whole proposal, not selected blocks. The resulting revision remains recoverable from revision history.',
              )}
            </p>
            <details>
              <summary>{t('基准与结果修订', 'Base and resulting revisions')}</summary>
              <p>{proposal.request.base_revision_id}</p>
              {proposal.applied_revision_id && <p>{proposal.applied_revision_id}</p>}
            </details>
            <ReferenceProvider
              identity={identity}
              generation={`${proposal.id}/${current?.revision_id ?? 'new'}`}
              onOpenReference={onOpenReference}
            >
              <div className="inference-columns">
                <section className="inference-card">
                  <h3>{t('基准正文', 'Base document')}</h3>
                  {base ? (
                    <>
                      <h4>{base.title}</h4>
                      <ReadDocument identity={identity} note={base} />
                    </>
                  ) : (
                    <p>
                      {t(
                        '此提案将新建笔记，没有需要替换的原正文。',
                        'This proposal creates a new note; no existing document is replaced.',
                      )}
                    </p>
                  )}
                </section>
                <section className="inference-card">
                  <h3>{t('建议正文', 'Proposed document')}</h3>
                  <h4>{proposal.request.title}</h4>
                  <ReadDocument identity={identity} note={preview} />
                </section>
              </div>
            </ReferenceProvider>
          </section>
        </>
      ) : (
        !error && <p>{t('正在读取提案与基准版本…', 'Loading proposal and base revision…')}</p>
      )}
      {proposal && confirming && proposal.status === 'pending' && !conflict && !targetTrashed && (
        <ProposalApplyConfirmDialog
          noteTitle={proposal.request.title}
          baseRevisionId={proposal.request.base_revision_id ?? ''}
          summary={{
            added: changes?.added ?? 0,
            changed: changes?.changed ?? 0,
            removed: changes?.removed ?? 0,
            total: changes?.total ?? 0,
          }}
          busy={applying}
          onCancel={() => setConfirming(false)}
          onConfirm={() => {
            setConfirming(false);
            void apply();
          }}
        />
      )}
    </PageFrame>
  );
}

function changeKindLabel(
  kind: 'added' | 'changed' | 'removed',
  t: (zh: string, en: string) => string,
): string {
  return kind === 'added'
    ? t('新增', 'Added')
    : kind === 'changed'
      ? t('修改', 'Changed')
      : t('移除', 'Removed');
}

function proposalBlockTypeLabel(type: string, t: (zh: string, en: string) => string): string {
  const labels: Record<string, [string, string]> = {
    paragraph: ['段落', 'Paragraph'],
    heading: ['标题', 'Heading'],
    bulletList: ['列表', 'List'],
    orderedList: ['编号列表', 'Numbered list'],
    taskList: ['任务列表', 'Task list'],
    blockquote: ['引用', 'Quote'],
    codeBlock: ['代码块', 'Code block'],
    image: ['图片', 'Image'],
    table: ['表格', 'Table'],
    callout: ['提示块', 'Callout'],
    wikiLink: ['双链', 'Wiki link'],
    noteReference: ['笔记引用', 'Note reference'],
    comment: ['批注', 'Comment'],
    frontmatter: ['前言属性', 'Frontmatter'],
  };
  const pair = labels[type];
  return pair ? t(pair[0], pair[1]) : type;
}
