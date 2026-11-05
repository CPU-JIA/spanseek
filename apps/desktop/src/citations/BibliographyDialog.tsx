import { Select } from '../ui/Select';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { BookMarked, ClipboardCopy, FileUp, Trash2, X } from 'lucide-react';
import type { BibliographyReport, CitationStyleSummary, WorkspaceIdentity } from '@pkms/contracts';
import { call, choose, message } from '../api';
import { useI18n } from '../i18n';
import { ActionConfirmDialog } from '../ui/ActionConfirmDialog';
import '../editor/reference-ui.css';
import './citation-picker.css';

/**
 * Read-only bibliography for one note: resolves `[@key]` markers through the
 * selected built-in or imported CSL style, lists missing keys, previews in-text
 * citation groups, and inserts or copies the rendered reference list.
 */
export function BibliographyDialog({
  identity,
  noteId,
  initialStyleId,
  onInsert,
  onClose,
}: {
  identity: WorkspaceIdentity;
  noteId: string;
  initialStyleId?: string;
  onInsert: (entries: BibliographyReport['entries'], missing: string[], styleId: string) => void;
  onClose: () => void;
}) {
  const { t, language } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [report, setReport] = useState<BibliographyReport | null>(null);
  const [styles, setStyles] = useState<CitationStyleSummary[]>([]);
  const [styleId, setStyleId] = useState(initialStyleId ?? 'plain');
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [importing, setImporting] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteCandidate, setDeleteCandidate] = useState<CitationStyleSummary | null>(null);
  const [styleReloadKey, setStyleReloadKey] = useState(0);
  const [reportReloadKey, setReportReloadKey] = useState(0);
  const builtInStyles = useMemo(() => styles.filter((style) => style.kind === 'builtin'), [styles]);
  const importedStyles = useMemo(
    () => styles.filter((style) => style.kind !== 'builtin'),
    [styles],
  );
  const selectedStyle = useMemo(
    () => styles.find((style) => style.id === styleId) ?? null,
    [styleId, styles],
  );
  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  useEffect(() => {
    let live = true;
    setError(null);
    call({ kind: 'citation_styles' }, 'citation_styles', identity).then(
      (value) => {
        if (!live) return;
        setStyles(value);
        setStyleId((current) => (value.some((style) => style.id === current) ? current : 'plain'));
      },
      (reason: unknown) => {
        if (live) setError(message(reason));
      },
    );
    return () => {
      live = false;
    };
  }, [identity, styleReloadKey]);
  useEffect(() => {
    let live = true;
    setReport(null);
    setError(null);
    call(
      { kind: 'bibliography_with_style', payload: { note_id: noteId, style_id: styleId } },
      'bibliography',
      identity,
    ).then(
      (value) => {
        if (live) setReport(value);
      },
      (reason: unknown) => {
        if (live) setError(message(reason));
      },
    );
    return () => {
      live = false;
    };
  }, [identity, noteId, reportReloadKey, styleId]);
  async function importStyle() {
    if (importing) return;
    setImporting(true);
    setError(null);
    setStatus(null);
    try {
      const grant = await choose('import_bibliography', identity);
      if (!grant) return;
      const imported = await call(
        { kind: 'citation_style_import', payload: { token: grant.token } },
        'citation_style_import',
        identity,
      );
      setStyleId(imported.style.id);
      setStyleReloadKey((value) => value + 1);
      setStatus(
        language === 'zh-CN'
          ? `已导入 CSL 样式：${imported.style.title}`
          : `Imported CSL style: ${imported.style.title}`,
      );
    } catch (reason) {
      setError(message(reason));
    } finally {
      setImporting(false);
    }
  }
  async function deleteStyle() {
    const candidate = deleteCandidate;
    if (!candidate || deleting) return;
    setDeleting(true);
    setError(null);
    setStatus(null);
    try {
      const remainingStyles = await call(
        { kind: 'citation_style_delete', payload: { style_id: candidate.id } },
        'citation_styles',
        identity,
      );
      setDeleteCandidate(null);
      setStyles(remainingStyles);
      setStyleId('plain');
      setReportReloadKey((value) => value + 1);
      setStatus(
        language === 'zh-CN'
          ? `已删除 CSL 样式：${candidate.title}`
          : `Deleted CSL style: ${candidate.title}`,
      );
    } catch (reason) {
      setDeleteCandidate(null);
      setError(message(reason));
    } finally {
      setDeleting(false);
    }
  }
  async function copy() {
    if (!report?.markdown) return;
    try {
      await navigator.clipboard.writeText(report.markdown);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (reason) {
      setError(message(reason));
    }
  }
  function retry() {
    setStyleReloadKey((value) => value + 1);
    setReportReloadKey((value) => value + 1);
  }
  return (
    <>
      <dialog
        ref={dialog}
        className="reference-picker bibliography-dialog"
        aria-labelledby={titleId}
        onCancel={(event) => {
          event.preventDefault();
          onClose();
        }}
        onClose={onClose}
      >
        <header>
          <h2 id={titleId}>{t('参考文献', 'Bibliography')}</h2>
          <div className="citation-actions">
            <button
              type="button"
              className="text-button"
              disabled={!report?.entries.length}
              onClick={() => {
                if (!report) return;
                onInsert(report.entries, report.missing, report.style_id);
                onClose();
              }}
            >
              <BookMarked size={15} /> {t('插入文献表', 'Insert bibliography')}
            </button>
            <button
              type="button"
              className="text-button"
              disabled={!report?.markdown}
              onClick={() => void copy()}
            >
              <ClipboardCopy size={15} />{' '}
              {copied ? t('已复制', 'Copied') : t('复制文献表', 'Copy list')}
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label={t('关闭', 'Close')}
              onClick={onClose}
            >
              <X size={17} />
            </button>
          </div>
        </header>
        <div className="bibliography-style">
          <span id={`${titleId}-style-label`}>{t('样式', 'Style')}</span>
          <Select
            id={`${titleId}-style`}
            aria-labelledby={`${titleId}-style-label`}
            value={styleId}
            disabled={!report && !error}
            onValueChange={(value) => {
              setStyleId(value);
              setError(null);
              setStatus(null);
            }}
          >
            <optgroup label={t('内置', 'Built-in')}>
              {builtInStyles.map((style) => (
                <option key={style.id} value={style.id}>
                  {style.id === 'plain'
                    ? t('默认（简式，GB/T 顺序）', 'Default (plain, GB/T order)')
                    : style.title}
                </option>
              ))}
            </optgroup>
            {importedStyles.length > 0 && (
              <optgroup label={t('已导入', 'Imported')}>
                {importedStyles.map((style) => (
                  <option key={style.id} value={style.id}>
                    {style.title}
                    {style.kind === 'dependent' ? t('（依赖样式）', ' (dependent)') : ''}
                  </option>
                ))}
              </optgroup>
            )}
          </Select>
          <div className="bibliography-style-actions">
            <button
              type="button"
              className="text-button bibliography-import-style"
              disabled={importing}
              aria-busy={importing}
              onClick={() => void importStyle()}
            >
              <FileUp size={15} />
              {importing ? t('正在导入…', 'Importing…') : t('导入 .csl', 'Import .csl')}
            </button>
            <button
              type="button"
              className="text-button bibliography-delete-style"
              disabled={!selectedStyle || selectedStyle.kind === 'builtin' || deleting}
              title={t('内置样式不可删除。', 'Built-in styles cannot be deleted.')}
              onClick={() => {
                if (selectedStyle) setDeleteCandidate(selectedStyle);
              }}
            >
              <Trash2 size={15} />
              {deleting ? t('正在删除…', 'Deleting…') : t('删除样式', 'Delete style')}
            </button>
          </div>
        </div>
        {status && <p role="status">{status}</p>}
        {!report && !error && <p role="status">{t('正在解析引用…', 'Resolving citations…')}</p>}
        {error && (
          <div className="inline-error" role="alert">
            <p>{error}</p>
            <button type="button" onClick={retry}>
              {t('重试', 'Retry')}
            </button>
          </div>
        )}
        {report && !report.keys.length && (
          <p className="muted">
            {t(
              '本文还没有引用。插入 [@键] 后这里会列出参考文献。',
              'This note has no citations yet. Insert [@key] to build a bibliography.',
            )}
          </p>
        )}
        {report && report.keys.length > 0 && (
          <>
            {report.in_text.length > 0 && (
              <section
                className="bibliography-in-text"
                aria-label={t('正文引注预览', 'In-text citation preview')}
              >
                <h3>{t('正文引注预览', 'In-text citation preview')}</h3>
                <div>
                  {report.in_text.map((citation, index) => (
                    <span key={`${index}-${citation.marker}`} className="bibliography-in-text-chip">
                      <small>{citation.marker}</small>
                      <strong>{citation.rendered}</strong>
                    </span>
                  ))}
                </div>
              </section>
            )}
            <div
              className="reference-target-list"
              aria-label={t('参考文献条目', 'Reference entries')}
            >
              {report.entries.map((entry) => (
                <div key={entry.key} className="reference-target bibliography-entry">
                  <BookMarked size={17} />
                  <span>
                    <strong>{entry.title}</strong>
                    <small>{entry.formatted}</small>
                    <small className="reference-identity">@{entry.key}</small>
                  </span>
                </div>
              ))}
            </div>
            {report.missing.length > 0 && (
              <div className="inline-error" role="alert">
                <p>
                  {t('缺少引用目标：', 'Missing targets: ')}
                  {report.missing.join(', ')}
                </p>
              </div>
            )}
          </>
        )}
        <p className="reference-picker-hint">
          {t(
            '只读视图；正文 [@键] 仍是引用源，文献表与引注按所选 CSL 样式生成。',
            'Read-only view; [@key] remains the citation source and both in-text citations and references use the selected CSL style.',
          )}
        </p>
      </dialog>
      {deleteCandidate && (
        <ActionConfirmDialog
          title={t('删除 CSL 样式？', 'Delete CSL style?')}
          description={t(
            `将删除“${deleteCandidate.title}”。仍被子样式依赖的父样式不能删除；删除后当前笔记会改用默认样式。`,
            `This removes “${deleteCandidate.title}”. Parent styles still required by dependent styles cannot be deleted; this note returns to the default style.`,
          )}
          confirmLabel={t('确认删除样式', 'Delete style')}
          tone="danger"
          onCancel={() => setDeleteCandidate(null)}
          onConfirm={() => void deleteStyle()}
        />
      )}
    </>
  );
}
