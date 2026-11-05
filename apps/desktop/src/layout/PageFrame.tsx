import {
  createContext,
  useContext,
  useId,
  useState,
  type FocusEventHandler,
  type PointerEventHandler,
  type ReactNode,
  type WheelEventHandler,
} from 'react';
import { createPortal } from 'react-dom';
import './page-frame.css';

const FooterTarget = createContext<HTMLDivElement | null>(null);

export function PageFrame({
  title,
  titleHidden = false,
  description,
  actions,
  toolbar,
  footer,
  nestedFooter = false,
  children,
  className,
  onPointerDownCapture,
  onFocusCapture,
  onWheelCapture,
}: {
  title: string;
  titleHidden?: boolean;
  description?: string;
  actions?: ReactNode;
  toolbar?: ReactNode;
  footer?: ReactNode;
  /** A nested list can place its controls in the page's fixed footer. */
  nestedFooter?: boolean;
  children: ReactNode;
  className?: string;
  onPointerDownCapture?: PointerEventHandler<HTMLElement>;
  onFocusCapture?: FocusEventHandler<HTMLElement>;
  onWheelCapture?: WheelEventHandler<HTMLElement>;
}) {
  const titleId = useId();
  const [footerTarget, setFooterTarget] = useState<HTMLDivElement | null>(null);
  return (
    <section
      className={`page-frame ${className ?? ''}`.trim()}
      aria-labelledby={titleId}
      onPointerDownCapture={onPointerDownCapture}
      onFocusCapture={onFocusCapture}
      onWheelCapture={onWheelCapture}
    >
      <header className="page-frame-header">
        <div className="page-frame-heading">
          <h1 id={titleId} className={titleHidden ? 'sr-only' : undefined}>
            {title}
          </h1>
          {description ? <p>{description}</p> : null}
        </div>
        {actions ? <div className="page-frame-actions">{actions}</div> : null}
      </header>
      {toolbar ? <div className="page-frame-toolbar">{toolbar}</div> : null}
      <FooterTarget.Provider value={footerTarget}>
        <PageBody>{children}</PageBody>
      </FooterTarget.Provider>
      {footer || nestedFooter ? (
        <FooterBar>
          {footer ?? <div className="page-footer-target" ref={setFooterTarget} />}
        </FooterBar>
      ) : null}
    </section>
  );
}

/** Keeps a child-owned list's pagination outside the scrolling page body. */
export function PageFooter({ children }: { children: ReactNode }) {
  const target = useContext(FooterTarget);
  return target ? createPortal(children, target) : null;
}

export function PageBody({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={`page-frame-body ${className ?? ''}`.trim()}>{children}</div>;
}

export function FooterBar({
  children,
  inline = false,
  className,
}: {
  children: ReactNode;
  inline?: boolean;
  className?: string;
}) {
  return (
    <footer className={`page-frame-footer ${inline ? 'is-inline' : ''} ${className ?? ''}`.trim()}>
      {children}
    </footer>
  );
}
