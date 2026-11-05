import { Select } from '../ui/Select';
import { useEffect, useRef, useState } from 'react';
import type { NoteSummary, WorkspaceIdentity } from '@pkms/contracts';
import { call, message } from '../api';
import { useI18n } from '../i18n';
import { useInferenceAction } from './useInferenceAction';
import { requestMarkdown } from './markdown-request';
import { useMarkdownDocument } from './useMarkdownDocument';
import { EDITOR_DOCUMENT_SCHEMA } from '../editor/document-schema';

export function ChatProposalPanel({
  identity,
  runId,
  text,
  onOpenProposal,
  initialTitle = '',
  sourceKind = 'chat',
}: {
  identity: WorkspaceIdentity;
  runId: string;
  text: string;
  onOpenProposal: (id: string) => void;
  initialTitle?: string;
  sourceKind?: 'chat' | 'task';
}) {
  const { t } = useI18n();
  const [format, setFormat] = useState<'rich' | 'source'>('rich');
  const [noteId, setNoteId] = useState('');
  const converted = useMarkdownDocument(text, !noteId, format === 'rich');
  const [title, setTitle] = useState(initialTitle);
  const [query, setQuery] = useState('');
  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const sequence = useRef(0);
  const live = useRef(true);
  const preparingRef = useRef(false);
  const preparation = useRef<AbortController | null>(null);
  const actions = useInferenceAction(identity, `chat-proposal/${runId}`);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      preparation.current?.abort();
      ++sequence.current;
    };
  }, []);
  async function find(after: string | null = null) {
    const current = ++sequence.current;
    try {
      const page = await call(
        { kind: 'notes_list', payload: { limit: 30, cursor: after, trashed: false, query } },
        'notes',
        identity,
      );
      if (!live.current || current !== sequence.current) return;
      setNotes((old) => (after ? [...old, ...page.items] : page.items));
      setCursor(page.next_cursor);
    } catch (e) {
      if (live.current) setError(message(e));
    }
  }
  async function create() {
    if (actions.locked || preparingRef.current) return;
    preparingRef.current = true;
    const controller = new AbortController();
    preparation.current = controller;
    setPreparing(true);
    setError(null);
    try {
      const run = await call(
        { kind: 'chat_runs_get', payload: { run_id: runId } },
        'chat_run',
        identity,
      );
      if (run.status !== 'completed')
        throw new Error(
          t('只能将完整回答创建为提案。', 'Only completed responses can become proposals.'),
        );
      const note = noteId
        ? await call({ kind: 'notes_get', payload: { note_id: noteId } }, 'note', identity)
        : null;
      if (!live.current || controller.signal.aborted) return;
      if (format === 'rich' && !converted.value)
        throw new Error(converted.error ?? 'Markdown conversion failed');
      const prepared = await requestMarkdown(
        sourceKind === 'task'
          ? {
              kind: 'task-proposal',
              source: text,
              base: note?.document ?? { type: 'doc', content: [] },
              materials: run.request.materials,
              format,
            }
          : {
              kind: 'chat-proposal',
              source: text,
              base: note?.document ?? { type: 'doc', content: [] },
              format,
              allowFrontmatter: !note,
            },
        controller.signal,
      );
      if (!live.current || controller.signal.aborted) return;
      const result = await actions.execute(
        {
          kind: 'proposals_create',
          payload: {
            command_id: crypto.randomUUID(),
            request: {
              chat_run_id: runId,
              note_id: note?.id ?? null,
              base_revision_id: note?.revision_id ?? null,
              title:
                note?.title ??
                (title.trim() || run.request.text.trim().split('\n')[0].slice(0, 120)),
              document_schema: EDITOR_DOCUMENT_SCHEMA,
              document: prepared.document,
            },
          },
        },
        'proposal_mutation',
      );
      if (result && live.current) onOpenProposal(result.proposal.id);
    } catch (e) {
      if (live.current && !controller.signal.aborted) setError(message(e));
    } finally {
      preparingRef.current = false;
      preparation.current = null;
      if (live.current) setPreparing(false);
    }
  }
  async function retry() {
    const result = await actions.retry<'proposal_mutation'>();
    if (result && live.current) onOpenProposal(result.proposal.id);
  }
  return (
    <div className="chat-proposal-panel">
      <p>
        {sourceKind === 'task'
          ? t(
              '核对草稿后创建保存提案。原始生成内容和保存记录保留在任务历史中。',
              'Create a save proposal after checking the draft. The original output and save records remain in task history.',
            )
          : t(
              '先审阅提案，再保存到笔记。标题、列表、表格等可转换为可编辑内容；对话中的原始回答始终保留。',
              'Review before saving. Headings, lists and tables become editable content; the original response stays in the conversation.',
            )}
      </p>
      <div className="choice-field">
        {t('保存格式', 'Save format')}
        <Select
          aria-label={t('保存格式', 'Save format')}
          value={format}
          disabled={preparing || actions.locked}
          onValueChange={(nextValue) => setFormat(nextValue as 'rich' | 'source')}
        >
          <option value="rich">{t('可编辑富文本', 'Editable rich text')}</option>
          <option value="source">{t('保留 Markdown 原文', 'Keep Markdown source')}</option>
        </Select>
      </div>
      {format === 'rich' && !!converted.value?.preserved.length && (
        <p role="status">
          {t(
            '部分语法超出当前编辑器支持范围，对应内容会按 Markdown 原文保留，包括不支持的格式、图片或链接；不会自动下载图片。请在提案中核对。',
            'Some syntax is unsupported by the editor. Those parts, including unsupported formatting, images or links, are kept as Markdown source. Images are not downloaded. Check them in the proposal.',
          )}
        </p>
      )}
      {format === 'rich' && converted.error && <p role="alert">{converted.error}</p>}
      <label>
        {t('新笔记标题', 'New note title')}
        <input
          value={title}
          disabled={!!noteId || preparing || actions.locked}
          onChange={(event) => setTitle(event.target.value)}
        />
      </label>
      <div className="chat-proposal-search">
        <input
          aria-label={t('查找要追加的笔记', 'Find a note to append to')}
          value={query}
          disabled={preparing || actions.locked}
          onChange={(event) => setQuery(event.target.value)}
        />
        <button type="button" disabled={preparing || actions.locked} onClick={() => void find()}>
          {t('查找笔记', 'Find notes')}
        </button>
      </div>
      <Select
        aria-label={t('提案目标', 'Proposal destination')}
        value={noteId}
        disabled={preparing || actions.locked}
        onValueChange={(nextValue) => setNoteId(nextValue)}
      >
        <option value="">{t('新建笔记', 'New note')}</option>
        {notes.map((note) => (
          <option key={note.id} value={note.id}>
            {note.title}
          </option>
        ))}
      </Select>
      {cursor && (
        <button
          type="button"
          disabled={preparing || actions.locked}
          onClick={() => void find(cursor)}
        >
          {t('更多笔记', 'More notes')}
        </button>
      )}
      {(error || actions.error) && <p role="alert">{error || actions.error}</p>}
      {format === 'rich' && converted.pending && (
        <p role="status">{t('正在准备格式…', 'Preparing formatting…')}</p>
      )}
      {preparing && !actions.locked && (
        <button
          type="button"
          onClick={() => {
            preparation.current?.abort();
            setError(t('已取消准备。', 'Preparation canceled.'));
          }}
        >
          {t('取消准备', 'Cancel preparation')}
        </button>
      )}
      {actions.pending ? (
        <button type="button" disabled={actions.busy} onClick={() => void retry()}>
          {t('确认上次提案', 'Recover proposal')}
        </button>
      ) : (
        <button
          type="button"
          disabled={actions.locked || preparing || (format === 'rich' && !converted.value)}
          onClick={() => void create()}
        >
          {t('创建并审阅提案', 'Create and review proposal')}
        </button>
      )}
    </div>
  );
}
