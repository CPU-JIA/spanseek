import { AlertCircle, LoaderCircle, RefreshCw } from 'lucide-react';
import './page-state.css';

export function PageState({
  kind,
  title,
  detail,
  onRetry,
  retryLabel,
  compact = false,
  className,
}: {
  kind: 'loading' | 'error';
  title: string;
  detail?: string | null;
  onRetry?: () => void;
  retryLabel?: string;
  compact?: boolean;
  className?: string;
}) {
  const failed = kind === 'error';
  return (
    <div
      className={['page-state', compact ? 'compact' : '', className ?? '']
        .filter(Boolean)
        .join(' ')}
      data-kind={kind}
      role={failed ? 'alert' : 'status'}
      aria-live={failed ? 'assertive' : 'polite'}
    >
      <span className="page-state-icon" aria-hidden="true">
        {failed ? <AlertCircle size={22} /> : <LoaderCircle size={22} className="spin" />}
      </span>
      <div className="page-state-copy">
        <strong>{title}</strong>
        {detail && <p>{detail}</p>}
      </div>
      {failed && onRetry && (
        <button type="button" className="secondary-button page-state-action" onClick={onRetry}>
          <RefreshCw size={15} aria-hidden="true" />
          {retryLabel}
        </button>
      )}
    </div>
  );
}
