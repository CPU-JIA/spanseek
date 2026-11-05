import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ArrowUpRight, BookOpen, FileText, Highlighter, Link2Off, Pin, X } from 'lucide-react';
import type { NodeAttributes, ResolveLink, WorkspaceIdentity } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { message } from '../api';
import {
  invalidateReference,
  resolveReference,
  useReferenceContext,
  useResolvedReference,
} from './reference-context';
import { opensReference, type ReferenceInteraction } from './reference-model';

export function ReferenceChip({
  attrs,
  identity,
  interaction = 'read',
  onSelect,
  displayLabel,
}: {
  attrs: NodeAttributes;
  identity: WorkspaceIdentity;
  interaction?: ReferenceInteraction;
  onSelect?: () => void;
  displayLabel?: string;
}) {
  const { t } = useI18n();
  const context = useReferenceContext();
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  const target: ResolveLink = {
    target_id: attrs.target_id ?? '',
    target_block_id: attrs.target_block_id ?? null,
    target_revision_id: attrs.target_revision_id ?? null,
  };
  const [refresh, setRefresh] = useState(0);
  const { result, error } = useResolvedReference(
    identity,
    target,
    `${context?.generation ?? ''}/${refresh}`,
  );
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const details = useRef<HTMLDialogElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const actionKey = `${identity.local_instance_id}/${identity.workspace_id}/${JSON.stringify(target)}`;
  const latestActionKey = useRef(actionKey);
  const navigationSerial = useRef(0);
  latestActionKey.current = actionKey;
  useEffect(() => {
    navigationSerial.current++;
    setBusy(false);
    setActionError(null);
    details.current?.close();
  }, [actionKey]);
  const unavailable =
    result &&
    (result.state !== 'available' || result.source_target?.locator_status === 'unavailable');
  const pageLocation =
    result?.source_target?.annotation_id === null &&
    result.source_target.locator?.kind === 'pdf' &&
    result.source_target.locator.rects.length === 0;
  const stateText =
    result?.state === 'target_trashed'
      ? t('目标已在回收站', 'Target is in Trash')
      : result?.state === 'target_missing'
        ? t('目标在当前知识库中不存在', 'Target is missing from this workspace')
        : result?.state === 'block_missing'
          ? t('原段落已移除', 'Original paragraph was removed')
          : result?.state === 'revision_missing'
            ? t('目标版本不可用', 'Target version is unavailable')
            : result?.source_target?.locator_status === 'unavailable'
              ? t('原文位置无法定位', 'Original source position is unavailable')
              : result?.source_target?.locator_status === 'approximate'
                ? pageLocation
                  ? t(
                      '定位资料页，未标记具体文字',
                      'Links to a source page without a text highlight',
                    )
                  : t('原文为近似定位，请核对', 'Approximate source location; verify the original')
                : error
                  ? t('引用解析失败', 'Reference could not be resolved')
                  : !result
                    ? t('正在解析引用…', 'Resolving reference…')
                    : attrs.target_revision_id
                      ? t('固定引用版本', 'Pinned reference version')
                      : t('跟随当前版本', 'Follows current version');
  // Keep the inline marker short so a line with several unresolved references stays readable;
  // the full sentence remains in the tooltip, the accessible name and the details dialog.
  const stateShort =
    result?.state === 'target_trashed'
      ? t('回收站', 'Trash')
      : result?.state === 'target_missing'
        ? t('缺失', 'Missing')
        : result?.state === 'block_missing'
          ? t('段落已移除', 'Block removed')
          : result?.state === 'revision_missing'
            ? t('版本不可用', 'Version unavailable')
            : result?.source_target?.locator_status === 'unavailable'
              ? t('位置不可用', 'No location')
              : result?.source_target?.locator_status === 'approximate'
                ? t('近似', 'Approximate')
                : error
                  ? t('解析失败', 'Failed')
                  : '';
  const targetTitle = result?.title || attrs.label || t('引用', 'Reference');
  const title = displayLabel || targetTitle;
  function closeDetails() {
    details.current?.close();
    trigger.current?.focus({ preventScroll: true });
  }
  function showDetails() {
    if (!details.current?.open) details.current?.showModal();
  }
  async function open(input: ResolveLink = target) {
    if (busy) return;
    const serial = ++navigationSerial.current;
    setBusy(true);
    setActionError(null);
    try {
      // Opening must never act on a remembered answer.
      const latest = await resolveReference(identity, input, { fresh: true });
      if (
        !active.current ||
        latestActionKey.current !== actionKey ||
        serial !== navigationSerial.current
      )
        return;
      if (latest.state !== 'available' || latest.source_target?.locator_status === 'unavailable') {
        // Show the state the fresh resolution just found, not the remembered one.
        invalidateReference(identity, target);
        setRefresh((value) => value + 1);
        showDetails();
        return;
      }
      if (!context?.onOpenReference) {
        setActionError(
          t(
            '当前区域不能打开目标，请在工作台继续。',
            'Open this reference from the workspace to navigate.',
          ),
        );
        showDetails();
        return;
      }
      details.current?.close();
      await context.onOpenReference(input);
    } catch (failure) {
      if (
        active.current &&
        latestActionKey.current === actionKey &&
        serial === navigationSerial.current
      ) {
        setActionError(message(failure));
        showDetails();
      }
    } finally {
      if (
        active.current &&
        latestActionKey.current === actionKey &&
        serial === navigationSerial.current
      )
        setBusy(false);
    }
  }
  return (
    <>
      <button
        ref={trigger}
        type="button"
        className={`reference-chip${unavailable || error ? ' is-unavailable' : ''}`}
        data-reference-occurrence={attrs.occurrence_id ?? undefined}
        title={`${title} · ${stateText}${interaction === 'edit' ? ` · ${t('Ctrl+点击打开；右键查看详情', 'Ctrl+click to open; right-click for details')}` : ''}${result?.block_text ? `\n${result.block_text}` : ''}`}
        aria-label={`${title} · ${stateText}`}
        disabled={busy}
        onClick={(event) => {
          if (opensReference(interaction, event)) void open();
          else onSelect?.();
        }}
        onKeyDown={(event) => {
          if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
            event.preventDefault();
            event.stopPropagation();
            showDetails();
          }
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          showDetails();
        }}
      >
        {unavailable || error ? (
          <Link2Off size={13} aria-hidden="true" />
        ) : result?.target_kind === 'annotation' ? (
          <Highlighter size={13} aria-hidden="true" />
        ) : result?.target_kind === 'source' ? (
          <BookOpen size={13} aria-hidden="true" />
        ) : (
          <FileText size={13} aria-hidden="true" />
        )}
        <span>{title}</span>
        {attrs.target_block_id && <span className="reference-block-mark">¶</span>}
        {attrs.target_revision_id && <Pin size={11} aria-label={t('固定版本', 'Pinned version')} />}
        {unavailable && (
          <span className="reference-state" title={stateText}>
            {stateShort || stateText}
          </span>
        )}
      </button>
      {createPortal(
        <dialog
          ref={details}
          className="reference-detail"
          aria-label={t('引用目标', 'Reference target')}
          onClick={(event) => event.stopPropagation()}
          onCancel={(event) => {
            event.preventDefault();
            event.stopPropagation();
            closeDetails();
          }}
        >
          <header>
            <h2>{t('引用目标', 'Reference target')}</h2>
            <button className="icon-button" aria-label={t('关闭', 'Close')} onClick={closeDetails}>
              <X size={18} />
            </button>
          </header>
          <h3>{targetTitle}</h3>
          {displayLabel && displayLabel !== targetTitle && (
            <p>
              {t('显示名称', 'Display name')} · {displayLabel}
            </p>
          )}
          <p className="reference-identity">
            {t('目标标识', 'Target ID')} · {target.target_id}
          </p>
          <p className={unavailable || error ? 'inline-error' : 'muted'}>{stateText}</p>
          {result?.block_text && <blockquote>{result.block_text}</blockquote>}
          {(error || actionError) && (
            <p role="alert" className="inline-error">
              {actionError || error}
            </p>
          )}
          {result?.state === 'available' && !unavailable && context?.onOpenReference && (
            <button className="primary-button" disabled={busy} onClick={() => void open()}>
              {t('打开目标', 'Open target')} <ArrowUpRight size={14} />
            </button>
          )}
          {result?.state === 'available' &&
            result.source_target?.locator_status === 'unavailable' &&
            context?.onOpenReference && (
              <button
                type="button"
                className="secondary-button"
                disabled={busy}
                onClick={() =>
                  void open({
                    target_id: result.source_target!.source_id,
                    target_revision_id: result.source_target!.source_version_id,
                    target_block_id: null,
                  })
                }
              >
                {t('查看整份原件', 'Open the whole original')}
              </button>
            )}
          {result?.state === 'block_missing' && context?.onOpenReference && (
            <div className="button-row">
              <button
                className="secondary-button"
                disabled={busy}
                onClick={() => void open({ ...target, target_block_id: null })}
              >
                {t('查看完整目标', 'Open whole target')}
              </button>
              {result.last_seen_revision_id && (
                <button
                  className="primary-button"
                  disabled={busy}
                  onClick={() =>
                    void open({ ...target, target_revision_id: result.last_seen_revision_id })
                  }
                >
                  {t('查看原段落历史', 'View paragraph history')}
                </button>
              )}
            </div>
          )}
          {result?.state === 'revision_missing' && context?.onOpenReference && (
            <button
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={() =>
                void open({ ...target, target_revision_id: null, target_block_id: null })
              }
            >
              {t('查看目标当前版本', 'Open the current target')}
            </button>
          )}
          <button
            className="text-button"
            onClick={() => {
              // Re-resolving asks again: forget the remembered answer first.
              invalidateReference(identity, target);
              setRefresh((value) => value + 1);
            }}
          >
            {t('重新解析', 'Resolve again')}
          </button>
        </dialog>,
        document.body,
      )}
    </>
  );
}
