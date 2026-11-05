import { AlertTriangle, ShieldCheck, X } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';
import { useI18n } from '../i18n';
import './action-confirm.css';

export type ActionConfirmInput = {
  label: string;
  initialValue?: string;
  placeholder?: string;
  multiline?: boolean;
  validate?: (value: string) => string | null;
};

export function ActionConfirmDialog({
  title,
  description,
  confirmLabel,
  tone = 'default',
  input,
  onCancel,
  onConfirm,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  tone?: 'default' | 'danger';
  input?: ActionConfirmInput;
  onCancel: () => void;
  onConfirm: (value: string) => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [value, setValue] = useState(input?.initialValue ?? '');
  const [inputError, setInputError] = useState<string | null>(null);
  const inputErrorId = useId();
  const titleId = useId();
  const descriptionId = useId();
  const Icon = tone === 'danger' ? AlertTriangle : ShieldCheck;
  function confirm() {
    const error = input?.validate?.(value);
    if (error) {
      setInputError(error);
      return;
    }
    onConfirm(value);
  }
  useEffect(() => {
    if (inputError)
      dialog.current?.querySelector('[role="alert"]')?.scrollIntoView({ block: 'nearest' });
  }, [inputError]);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    element.showModal();
    inputRef.current?.focus({ preventScroll: true });
    textareaRef.current?.focus({ preventScroll: true });
    if (!inputRef.current && !textareaRef.current) cancel.current?.focus({ preventScroll: true });
    return () => {
      if (element.open) element.close();
      if (previous?.isConnected && !previous.matches(':disabled') && !previous.closest('[inert]'))
        previous.focus({ preventScroll: true });
    };
  }, []);

  return (
    <dialog
      ref={dialog}
      role="alertdialog"
      className={`action-confirm-dialog is-${tone}`}
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      onCancel={(event) => {
        event.preventDefault();
        onCancel();
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && (event.nativeEvent.isComposing || event.keyCode === 229)) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
    >
      <header>
        <span className="action-confirm-icon">
          <Icon size={22} strokeWidth={1.7} aria-hidden="true" />
        </span>
        <h2 id={titleId}>{title}</h2>
        <button
          type="button"
          className="icon-button"
          aria-label={t('关闭确认框', 'Close confirmation')}
          onClick={onCancel}
        >
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <div className="action-confirm-content">
        <p id={descriptionId}>{description}</p>
        {input && (
          <label className="action-confirm-field">
            <span>{input.label}</span>
            {input.multiline ? (
              <textarea
                ref={textareaRef}
                rows={3}
                value={value}
                placeholder={input.placeholder}
                aria-invalid={!!inputError}
                aria-describedby={inputError ? inputErrorId : undefined}
                onChange={(event) => {
                  setValue(event.target.value);
                  setInputError(null);
                }}
              />
            ) : (
              <input
                ref={inputRef}
                value={value}
                placeholder={input.placeholder}
                aria-invalid={!!inputError}
                aria-describedby={inputError ? inputErrorId : undefined}
                onKeyDown={(event) => {
                  if (
                    event.key === 'Enter' &&
                    !event.nativeEvent.isComposing &&
                    event.keyCode !== 229
                  ) {
                    event.preventDefault();
                    confirm();
                  }
                }}
                onChange={(event) => {
                  setValue(event.target.value);
                  setInputError(null);
                }}
              />
            )}
          </label>
        )}
        {inputError && (
          <p id={inputErrorId} role="alert">
            {inputError}
          </p>
        )}
      </div>
      <footer>
        <button ref={cancel} type="button" className="secondary-button" onClick={onCancel}>
          {t('取消', 'Cancel')}
        </button>
        <button type="button" className="primary-button" onClick={confirm}>
          {confirmLabel}
        </button>
      </footer>
    </dialog>
  );
}
