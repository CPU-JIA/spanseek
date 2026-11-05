import { useEffect, useRef } from 'react';
import { clamp } from './columns';
import './columns.css';

export function ColumnResizer({
  label,
  controls,
  value,
  min,
  max,
  disabled,
  onPreview,
  onCommit,
  onCancel,
}: {
  label: string;
  controls: string;
  value: number;
  min: number;
  max: number;
  disabled?: boolean;
  onPreview: (value: number) => void;
  onCommit: (value: number | null) => void;
  onCancel: () => void;
}) {
  const drag = useRef<{ x: number; start: number; latest: number } | null>(null);
  useEffect(
    () => () => {
      if (drag.current) delete document.documentElement.dataset.resizing;
    },
    [],
  );
  function cancel() {
    if (drag.current) {
      drag.current = null;
      delete document.documentElement.dataset.resizing;
      onCancel();
    }
  }
  return (
    <div
      className="column-resizer"
      role="separator"
      aria-label={label}
      aria-controls={controls}
      aria-orientation="vertical"
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={Math.round(max)}
      aria-disabled={disabled}
      tabIndex={disabled ? -1 : 0}
      onPointerDown={(event) => {
        if (disabled || event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, start: value, latest: value };
        document.documentElement.dataset.resizing = 'columns';
      }}
      onPointerMove={(event) => {
        if (!drag.current) return;
        const next = Math.round(
          clamp(drag.current.start + event.clientX - drag.current.x, min, max),
        );
        drag.current.latest = next;
        onPreview(next);
      }}
      onPointerUp={(event) => {
        if (!drag.current) return;
        const { latest, start } = drag.current;
        drag.current = null;
        delete document.documentElement.dataset.resizing;
        event.currentTarget.releasePointerCapture(event.pointerId);
        if (latest !== start) onCommit(latest);
        else onCancel();
      }}
      onPointerCancel={cancel}
      onLostPointerCapture={cancel}
      onDoubleClick={() => {
        if (!disabled) onCommit(null);
      }}
      onKeyDown={(event) => {
        if (disabled) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          cancel();
          return;
        }
        let next: number;
        if (event.key === 'Home') next = min;
        else if (event.key === 'End') next = max;
        else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight')
          next = value + (event.key === 'ArrowLeft' ? -1 : 1) * (event.shiftKey ? 32 : 8);
        else return;
        event.preventDefault();
        onCommit(Math.round(clamp(next, min, max)));
      }}
    />
  );
}
