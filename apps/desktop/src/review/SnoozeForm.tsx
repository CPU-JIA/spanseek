import type { KeyboardEvent } from 'react';
import { useI18n } from '../i18n';
import { snoozeKeyAction } from './snooze-panel';

export type SnoozeFormProps = {
  /** The local `datetime-local` value the field edits. */
  date: string;
  /** The local next minute; the field never offers a time the submit validator refuses. */
  min: string;
  disabled: boolean;
  onDate: (value: string) => void;
  onSubmit: () => void;
  /** Escape (or the 稍后回顾 button): leave the panel and hand the focus back to it. */
  onCancel: () => void;
};

/**
 * The 稍后回顾 field. The page mounts it only while the panel is open for the entry
 * on screen, so the date a user typed for one item can never be edited or sent for
 * another one.
 */
export function SnoozeForm({ date, min, disabled, onDate, onSubmit, onCancel }: SnoozeFormProps) {
  const { t } = useI18n();
  const keys = (event: KeyboardEvent<HTMLFormElement>) => {
    // Escape leaves the panel; Enter stays the form's own submit, and Tab keeps
    // walking the fields, so nothing else is intercepted here.
    if (snoozeKeyAction(event.key) !== 'cancel') return;
    event.preventDefault();
    event.stopPropagation();
    onCancel();
  };
  return (
    <form
      className="review-snooze"
      onKeyDown={keys}
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit();
      }}
    >
      <label>
        {t('下次回顾时间', 'Next review')}
        <input
          type="datetime-local"
          required
          autoFocus
          min={min}
          value={date}
          disabled={disabled}
          onChange={(event) => onDate(event.target.value)}
        />
      </label>
      <button type="submit" disabled={disabled || !date}>
        {t('确认时间', 'Set time')}
      </button>
    </form>
  );
}
