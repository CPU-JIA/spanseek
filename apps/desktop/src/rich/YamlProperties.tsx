import { Select } from '../ui/Select';
import { useEffect, useMemo, useState } from 'react';
import { useI18n } from '../i18n';
import { displayDateTime } from '../format';
import { formatDuration } from '../voice/voice-model';
import {
  objectTypeChangeAllowed,
  replaceYamlScalar,
  yamlFields,
  type PropertyField,
  type PropertyScalar,
} from './yaml-properties';
import './rich-content.css';

export interface ObjectTypeOption {
  value: string;
  label: string;
}

function propertyLabel(key: string, t: (zh: string, en: string) => string): string {
  switch (key) {
    case 'type':
    case 'object_type':
      return t('对象类型', 'Object type');
    case 'voice_capture':
      return t('语音记录标识', 'Recording ID');
    case 'voice_duration_ms':
      return t('录音时长', 'Recording duration');
    case 'voice_recorded_at':
      return t('录音时间', 'Recorded at');
    default:
      return key;
  }
}

const voiceProperties = new Set(['voice_capture', 'voice_duration_ms', 'voice_recorded_at']);

export function YamlProperties({
  source,
  onChange,
  objectTypeOptions,
}: {
  source: string;
  onChange?: (source: string) => void;
  objectTypeOptions?: readonly ObjectTypeOption[];
}) {
  const { t, language } = useI18n();
  const parsed = useMemo(() => yamlFields(source), [source]);
  const [error, setError] = useState<string | null>(null);
  const [rawDraft, setRawDraft] = useState(source);
  useEffect(() => {
    setRawDraft(source);
    setError(null);
  }, [source]);
  function typeChangeError(next: string) {
    return objectTypeChangeAllowed(
      source,
      next,
      objectTypeOptions?.map((option) => option.value) ?? [],
    )
      ? null
      : t('对象类型必须从下拉列表中选择。', 'Choose an object type from the list.');
  }
  function changeSource(value: string) {
    setRawDraft(value);
    try {
      const typeError = typeChangeError(value);
      if (typeError) throw new Error(typeError);
      onChange?.(value);
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : t('属性未修改。', 'The property was not changed.'),
      );
    }
  }
  function change(field: PropertyField, value: PropertyScalar) {
    try {
      const next = replaceYamlScalar(source, field.path, value);
      onChange?.(next);
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : t('属性未修改。', 'The property was not changed.'),
      );
    }
  }
  return (
    <details
      className="rich-properties"
      aria-label={t('正文属性', 'Document properties')}
      open={parsed.error || error ? true : undefined}
    >
      <summary>{t('属性与来源信息', 'Properties & source information')}</summary>
      {parsed.fields.length > 0 && (
        <dl>
          {parsed.fields.map((field) => (
            <div key={field.key} className="rich-property-row">
              <dt>{propertyLabel(field.key, t)}</dt>
              <dd>
                {field.key === 'voice_duration_ms' && typeof field.value === 'number' ? (
                  <span>{formatDuration(field.value)}</span>
                ) : field.key === 'voice_recorded_at' &&
                  typeof field.value === 'string' &&
                  Number.isFinite(Date.parse(field.value)) ? (
                  <span>{displayDateTime(field.value, language)}</span>
                ) : (
                  <PropertyValue
                    field={field}
                    editable={Boolean(onChange) && !voiceProperties.has(field.key)}
                    onChange={change}
                    objectTypeOptions={
                      field.key === 'type' || field.key === 'object_type'
                        ? objectTypeOptions
                        : undefined
                    }
                  />
                )}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {(parsed.error || error) && (
        <p role="alert" className="inline-error">
          {error || parsed.error}
        </p>
      )}
      {parsed.truncated && (
        <p className="rich-caption">
          {t(
            '部分属性仅在完整源码中显示。',
            'Additional properties are available in the complete source.',
          )}
        </p>
      )}
      <details className="rich-source">
        <summary>{t('YAML 源码', 'YAML source')}</summary>
        {onChange ? (
          <textarea
            aria-label={t('YAML 属性源码', 'YAML properties source')}
            spellCheck={false}
            value={rawDraft}
            onChange={(event) => changeSource(event.target.value)}
          />
        ) : (
          <pre>
            <code>{source}</code>
          </pre>
        )}
      </details>
    </details>
  );
}

function PropertyValue({
  field,
  editable,
  onChange,
  objectTypeOptions,
}: {
  field: PropertyField;
  editable: boolean;
  onChange: (field: PropertyField, value: PropertyScalar) => void;
  objectTypeOptions?: readonly ObjectTypeOption[];
}) {
  const { t } = useI18n();
  const label = propertyLabel(field.key, t);
  const original = field.value === null || field.value === undefined ? '' : String(field.value);
  const [draft, setDraft] = useState(original);
  const [error, setError] = useState(false);
  useEffect(() => {
    setDraft(original);
    setError(false);
  }, [original]);
  if (field.kind === 'nested')
    return (
      <span className="rich-caption">
        {t(
          '嵌套结构或引用，完整保留在源码中',
          'Nested structure or reference; preserved in source',
        )}
      </span>
    );
  if (field.kind === 'list')
    return (
      <ul>
        {field.children?.map((child) => (
          <li key={child.key}>
            <PropertyValue field={child} editable={editable} onChange={onChange} />
          </li>
        ))}
      </ul>
    );
  if (!editable)
    return field.kind === 'boolean' ? (
      <span>{field.value ? t('是', 'True') : t('否', 'False')}</span>
    ) : (
      <span className="rich-property-text">
        {field.value === null
          ? t('空值（未设置）', 'Null (unset)')
          : (objectTypeOptions?.find((option) => option.value === original)?.label ?? original)}
      </span>
    );
  if (
    (field.key === 'type' || field.key === 'object_type') &&
    (field.kind === 'text' || field.kind === 'null')
  ) {
    if (!objectTypeOptions?.length) {
      return (
        <>
          <span className="rich-property-text">{original || t('未设置', 'Not set')}</span>
          <span className="rich-caption">
            {t(
              '对象类型注册表不可用；当前值只读保留，不能在这里自由输入。',
              'The object type registry is unavailable. The current value is preserved read-only.',
            )}
          </span>
        </>
      );
    }
    const known = objectTypeOptions.some((option) => option.value === original);
    const choices = [
      ...(original && !known
        ? [{ value: original, label: `${original} · ${t('未注册类型', 'Unregistered type')}` }]
        : []),
      ...objectTypeOptions,
    ];
    return (
      <>
        <Select
          aria-label={label}
          className="rich-property-select"
          value={original}
          options={choices}
          placeholder={t('未设置', 'Not set')}
          onValueChange={(nextValue) => onChange(field, nextValue)}
        />
        {original && !known && (
          <span className="rich-caption">
            {t(
              '当前值不在已注册类型中；保留原值，或从下拉列表中选择。',
              'This value is not in the type registry. Keep it or choose a registered type.',
            )}
          </span>
        )}
      </>
    );
  }
  if (field.kind === 'boolean')
    return (
      <input
        type="checkbox"
        aria-label={label}
        checked={field.value === true}
        onChange={(event) => onChange(field, event.target.checked)}
      />
    );
  if (field.kind === 'null')
    return (
      <Select
        aria-label={label}
        value="null"
        onValueChange={(nextValue) =>
          onChange(field, nextValue === 'true' ? true : nextValue === 'false' ? false : '')
        }
      >
        <option value="null">{t('空值（未设置）', 'Null (unset)')}</option>
        <option value="text">{t('改为文本', 'Use text')}</option>
        <option value="true">{t('是', 'True')}</option>
        <option value="false">{t('否', 'False')}</option>
      </Select>
    );
  const commit = () => {
    if (draft === original) return;
    if (field.kind === 'number' && (!draft.trim() || !Number.isFinite(Number(draft)))) {
      setError(true);
      return;
    }
    onChange(field, field.kind === 'number' ? Number(draft) : draft);
    setError(false);
  };
  return (
    <>
      <input
        className="ui-input"
        aria-label={label}
        aria-invalid={error || undefined}
        type={
          field.kind === 'number'
            ? 'number'
            : field.kind === 'date'
              ? 'date'
              : field.kind === 'datetime'
                ? 'datetime-local'
                : 'text'
        }
        step={field.kind === 'number' ? 'any' : field.kind === 'datetime' ? '1' : undefined}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
            event.preventDefault();
            commit();
          }
        }}
      />
      {error && <span role="alert">{t('请输入有效数字。', 'Enter a valid number.')}</span>}
    </>
  );
}
