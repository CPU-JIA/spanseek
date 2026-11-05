import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  CleaningParams,
  CleaningReport,
  CleaningSettings,
  WorkspaceIdentity,
} from '@pkms/contracts';
import { call } from '../api';
import { useI18n } from '../i18n';

export const DEFAULT_CLEANING_PARAMS: CleaningParams = {
  headers_footers: true,
  watermark: true,
  control_chars: true,
};

export async function loadCleaningSettings(identity: WorkspaceIdentity): Promise<CleaningSettings> {
  return call({ kind: 'cleaning_settings_get' }, 'cleaning_settings', identity);
}

export async function saveCleaningParams(
  identity: WorkspaceIdentity,
  baseRowVersion: number,
  params: CleaningParams,
): Promise<CleaningSettings> {
  const mutation = await call(
    {
      kind: 'cleaning_settings_set',
      payload: {
        command_id: crypto.randomUUID(),
        request: { base_row_version: baseRowVersion, params },
      },
    },
    'cleaning_settings_mutation',
    identity,
  );
  return mutation.settings;
}

export function useCleaningSettings(identity: WorkspaceIdentity | null) {
  const [stored, setStored] = useState<{ key: string; value: CleaningSettings } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const live = useRef(true);
  const key = identity ? `${identity.local_instance_id}/${identity.workspace_id}` : '';
  const activeKey = useRef(key);
  activeKey.current = key;
  const request = useRef(0);
  const settings = stored?.key === key ? stored.value : null;
  const setSettings = useCallback(
    (value: CleaningSettings) => {
      if (!live.current || activeKey.current !== key) return;
      request.current += 1;
      setStored({ key, value });
      setError(null);
      setBusy(false);
    },
    [key],
  );
  const reload = useCallback(async () => {
    const pending = ++request.current;
    if (!identity) {
      setStored(null);
      setError(null);
      setBusy(false);
      return null;
    }
    setBusy(true);
    setError(null);
    const current = () => live.current && activeKey.current === key && pending === request.current;
    try {
      const value = await loadCleaningSettings(identity);
      if (!current()) return null;
      setStored({ key, value });
      return value;
    } catch (failure) {
      if (current()) setError(failure instanceof Error ? failure.message : '无法读取清洗设置。');
      return null;
    } finally {
      if (current()) setBusy(false);
    }
  }, [key]);
  useEffect(() => {
    live.current = true;
    setStored(null);
    void reload();
    return () => {
      live.current = false;
      request.current += 1;
    };
  }, [reload]);
  return { settings, setSettings, error, busy, reload };
}

export function CleaningParamsForm({
  value,
  onChange,
  disabled,
}: {
  value: CleaningParams;
  onChange: (next: CleaningParams) => void;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className="retrieval-params">
      <label className="retrieval-param retrieval-param-switch">
        <span className="retrieval-param-copy">
          <strong>{t('去除重复页眉页脚', 'Remove repeated headers and footers')}</strong>
          <small>
            {t(
              '分页资料里每页开头/结尾重复出现的同一行文本会被判为页眉页脚并移除（需至少 3 页且过半页重复）。默认开启。',
              'Text repeated at the start or end of most pages is treated as a running header or footer (needs at least 3 pages and half of them). On by default.',
            )}
          </small>
        </span>
        <input
          type="checkbox"
          className="retrieval-param-checkbox"
          disabled={disabled}
          checked={value.headers_footers}
          onChange={(event) => onChange({ ...value, headers_footers: event.target.checked })}
        />
      </label>
      <label className="retrieval-param retrieval-param-switch">
        <span className="retrieval-param-copy">
          <strong>{t('去除重复水印文本', 'Remove repeated watermark text')}</strong>
          <small>
            {t(
              '在多数页面重复出现的短文本（≤40 字符）会被判为水印并移除。默认开启。',
              'Short text (≤40 characters) repeated on most pages is treated as a watermark. On by default.',
            )}
          </small>
        </span>
        <input
          type="checkbox"
          className="retrieval-param-checkbox"
          disabled={disabled}
          checked={value.watermark}
          onChange={(event) => onChange({ ...value, watermark: event.target.checked })}
        />
      </label>
      <label className="retrieval-param retrieval-param-switch">
        <span className="retrieval-param-copy">
          <strong>{t('去除编码异常字符', 'Remove invisible control characters')}</strong>
          <small>
            {t(
              '去掉零宽字符、BOM、软连字符等无文本含义的字符；替换符（�）只计数不删除。默认开启。',
              'Removes zero-width characters, BOM and soft hyphens; replacement characters (�) are counted, never deleted. On by default.',
            )}
          </small>
        </span>
        <input
          type="checkbox"
          className="retrieval-param-checkbox"
          disabled={disabled}
          checked={value.control_chars}
          onChange={(event) => onChange({ ...value, control_chars: event.target.checked })}
        />
      </label>
    </div>
  );
}

/** One sentence for the reader: what cleaning removed for this parse. */
export function cleaningSummary(
  report: CleaningReport,
  t: (zh: string, en: string) => string,
): string {
  const parts: string[] = [];
  if (report.headers_footers > 0)
    parts.push(
      t(`页眉页脚 ${report.headers_footers} 块`, `${report.headers_footers} header/footer blocks`),
    );
  if (report.watermark > 0)
    parts.push(t(`水印 ${report.watermark} 块`, `${report.watermark} watermark blocks`));
  if (report.control_chars > 0)
    parts.push(
      t(`异常字符 ${report.control_chars} 个`, `${report.control_chars} invisible characters`),
    );
  const removed = t(
    `共移除 ${report.removed_blocks} 块 / ${report.removed_chars} 字符`,
    `${report.removed_blocks} blocks / ${report.removed_chars} characters removed`,
  );
  const kept = report.kept_everything
    ? t('（为避免清空正文已跳过）', ' (skipped to keep the document readable)')
    : '';
  if (!parts.length) return `${t('未发现需要清洗的内容', 'Nothing needed cleaning')}${kept}`;
  return `${parts.join(' · ')}；${removed}${kept}`;
}
