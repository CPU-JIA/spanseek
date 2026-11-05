import { useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { ExternalLink } from 'lucide-react';
import { message, openExternalUrl } from '../api';
import { useI18n } from '../i18n';
import { localLinkAnchor, ordinaryLinkTarget } from './link-target';
import './document-link.css';

export async function openDocumentLink(href: string, root: HTMLElement) {
  const target = ordinaryLinkTarget(href);
  if (target.kind === 'external') {
    await openExternalUrl(target.href);
    return;
  }
  const element = localLinkAnchor(root, target.anchor);
  if (!element) throw new Error('本文找不到此链接的标题或段落。');
  for (
    let parent = element.parentElement;
    parent && parent !== root.parentElement;
    parent = parent.parentElement
  )
    if (parent instanceof HTMLDetailsElement) parent.open = true;
  element.scrollIntoView({ block: 'center', behavior: 'instant' });
  element.focus({ preventScroll: true });
}

export function DocumentLink({
  href,
  title,
  className,
  style,
  children,
  atom = false,
}: {
  href: string;
  title?: string | null;
  className: string;
  style?: CSSProperties;
  children: ReactNode;
  atom?: boolean;
}) {
  const { t } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  let valid = false;
  try {
    ordinaryLinkTarget(href);
    valid = true;
  } catch {
    /* Render an actionable invalid link with its real error. */
  }
  const link = (
    <a
      href={valid ? href : undefined}
      role={valid ? undefined : 'link'}
      tabIndex={valid ? undefined : 0}
      className={className}
      title={title ?? (href || t('链接地址为空', 'Empty link URL'))}
      style={style}
      rel="noopener noreferrer"
      target="_blank"
      aria-label={atom ? t('打开附加链接', 'Open attached link') : undefined}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (busy.current) return;
        busy.current = true;
        setError(null);
        void openDocumentLink(href, event.currentTarget)
          .catch((cause) => setError(message(cause)))
          .finally(() => {
            busy.current = false;
          });
      }}
      onKeyDown={(event) => {
        if (!valid && event.key === 'Enter') {
          event.preventDefault();
          event.currentTarget.click();
        }
      }}
    >
      {atom ? <ExternalLink size={12} aria-hidden="true" /> : children}
    </a>
  );
  return (
    <>
      {atom ? (
        <>
          {children}
          <span className="document-atom-link">{link}</span>
        </>
      ) : (
        link
      )}
      {error && (
        <span className="document-link-error" role="alert">
          {error}
        </span>
      )}
    </>
  );
}
