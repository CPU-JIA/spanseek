import { BookOpen, CheckSquare, FileText, Globe, MoreHorizontal } from 'lucide-react';
import type { SourceSummary } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { PopupMenu } from '../ui/PopupMenu';

function originLabel(source: SourceSummary) {
  if (source.origin_url) {
    try {
      const url = new URL(source.origin_url);
      return url.host + decodeURI(url.pathname === '/' ? '' : url.pathname);
    } catch {
      return source.origin_url;
    }
  }
  return source.original_name || source.display_title;
}

export function SourceLibraryRow({
  source,
  selecting,
  selected,
  selectionFull,
  sameTitle,
  onOpen,
  onSelect,
  onStartTask,
}: {
  source: SourceSummary;
  selecting: boolean;
  selected: boolean;
  selectionFull: boolean;
  sameTitle: boolean;
  onOpen: () => void;
  onSelect: (checked: boolean) => void;
  onStartTask?: () => void;
}) {
  const { t, language } = useI18n();
  const formatLabels: Record<SourceSummary['format'], string> = {
    text: 'TXT',
    markdown: 'MD',
    pdf: 'PDF',
    html: t('网页', 'Web'),
    docx: 'DOCX',
    doc: 'DOC',
    epub: 'EPUB',
    mobi: 'MOBI',
    csv: 'CSV',
    xlsx: 'XLSX',
    xls: 'XLS',
    pptx: 'PPTX',
  };
  const format = formatLabels[source.format] ?? source.format.toUpperCase();
  const origin = originLabel(source);
  const preview = source.preview_text?.replace(/\s+/g, ' ').trim();
  const date = new Date(source.updated_at);
  const dateLabel = Number.isNaN(date.getTime())
    ? source.updated_at
    : new Intl.DateTimeFormat(language === 'en' ? 'en-US' : 'zh-CN', {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
      }).format(date);
  const fingerprint = (source.content_hash || source.id).slice(0, 8);
  const accessibleTitle = sameTitle
    ? `${source.display_title} · ${format} · ${origin} · ${fingerprint}`
    : source.display_title;
  return (
    <li className="source-entry" data-source-id={source.id} data-selected={selected || undefined}>
      {selecting && (
        <label className="source-check-label">
          <input
            type="checkbox"
            aria-label={t('选择资料：' + accessibleTitle, 'Select document: ' + accessibleTitle)}
            checked={selected}
            disabled={selectionFull && !selected}
            onChange={(event) => onSelect(event.target.checked)}
          />
        </label>
      )}
      <button
        type="button"
        className="source-row"
        onClick={onOpen}
        aria-label={t('阅读：' + accessibleTitle, 'Read: ' + accessibleTitle)}
      >
        <span className="source-format-icon" data-format={source.format} aria-hidden="true">
          {source.format === 'html' ? (
            <Globe size={22} strokeWidth={1.6} />
          ) : (
            <FileText size={23} strokeWidth={1.6} />
          )}
          <span>{format}</span>
        </span>
        <span className="source-row-main">
          <span className="source-row-title">
            <strong title={source.display_title}>{source.display_title}</strong>
          </span>
          <span className="source-row-origin" title={source.origin_url || origin}>
            <span>{origin}</span>
            {sameTitle && (
              <span
                className="source-fingerprint"
                title={t(
                  '内容标识，用于区分同名资料',
                  'Content identifier for documents with the same title',
                )}
              >
                {fingerprint}
              </span>
            )}
          </span>
          {preview ? (
            <span className="source-row-preview">{preview}</span>
          ) : (
            <span className="source-row-preview source-row-preview-empty">
              {source.format === 'pdf'
                ? t('打开 PDF 原件阅读', 'Open the original PDF to read')
                : source.text_ready
                  ? t('打开资料阅读完整内容', 'Open the document to read in full')
                  : t('原件已保存，正文暂不可用', 'Original saved; text is not available yet')}
            </span>
          )}
        </span>
        <span className="source-row-aside">
          <time
            dateTime={source.updated_at}
            title={Number.isNaN(date.getTime()) ? '' : date.toLocaleString()}
          >
            {dateLabel}
          </time>
          {source.trashed && <span className="source-status">{t('已回收', 'In trash')}</span>}
          {!source.trashed && !source.text_ready && (
            <span className="source-status">{t('原件已保存', 'Original saved')}</span>
          )}
          <BookOpen className="source-open-hint" size={16} aria-hidden="true" />
        </span>
      </button>
      <PopupMenu
        label={t('资料操作：' + accessibleTitle, 'Document actions: ' + accessibleTitle)}
        trigger={<MoreHorizontal size={17} aria-hidden="true" />}
        placement="bottom-start"
        className="icon-button source-row-menu"
        items={[
          {
            id: 'open',
            label: t('打开原文', 'Open document'),
            icon: <BookOpen size={16} />,
            onSelect: onOpen,
          },
          {
            id: 'select',
            label: selected ? t('取消选择', 'Deselect') : t('选择此资料', 'Select document'),
            icon: <CheckSquare size={16} />,
            disabled: selectionFull && !selected,
            onSelect: () => onSelect(!selected),
          },
          {
            id: 'knowledge-task',
            label: t('基于此资料发起任务', 'Start a task with this source'),
            disabled: !onStartTask || source.trashed,
            onSelect: () => onStartTask?.(),
          },
        ]}
      />
    </li>
  );
}
