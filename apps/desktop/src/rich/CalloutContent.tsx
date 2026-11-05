import { useEffect, useState, type CSSProperties, type ReactNode } from 'react';
import {
  Bug,
  Check,
  CircleCheck,
  CircleHelp,
  FlaskConical,
  Info,
  Lightbulb,
  List,
  Pencil,
  Quote,
  TriangleAlert,
  X,
  Zap,
} from 'lucide-react';
import { getCallout } from './callouts';
import { useI18n } from '../i18n';
import './rich-content.css';

const icons = {
  pencil: Pencil,
  list: List,
  info: Info,
  'circle-check': CircleCheck,
  lightbulb: Lightbulb,
  check: Check,
  'circle-help': CircleHelp,
  'triangle-alert': TriangleAlert,
  x: X,
  zap: Zap,
  bug: Bug,
  'flask-conical': FlaskConical,
  quote: Quote,
};
export function CalloutIcon({ type, size = 18 }: { type?: string | null; size?: number }) {
  const Icon = icons[getCallout(type).icon];
  return <Icon size={size} aria-hidden="true" />;
}
export function CalloutContent({
  type,
  title,
  collapsed,
  children,
}: {
  type?: string | null;
  title?: string | null;
  collapsed?: boolean | null;
  children: ReactNode;
}) {
  const { t } = useI18n();
  const callout = getCallout(type),
    Icon = icons[callout.icon];
  const [open, setOpen] = useState(collapsed !== true);
  useEffect(() => setOpen(collapsed !== true), [collapsed]);
  const style = { '--callout-color': callout.color } as CSSProperties;
  const heading = (
    <>
      <Icon size={18} aria-hidden="true" />
      <span>
        {title ||
          (callout.type === 'note' && type && type.toLowerCase() !== 'note'
            ? type
            : t(callout.label, callout.en))}
      </span>
    </>
  );
  return typeof collapsed === 'boolean' ? (
    <details
      className="rich-callout"
      data-callout-type={type ?? 'note'}
      style={style}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>{heading}</summary>
      <div className="rich-callout-body">{children}</div>
    </details>
  ) : (
    <aside className="rich-callout" data-callout-type={type ?? 'note'} style={style}>
      <div className="rich-callout-title">{heading}</div>
      <div className="rich-callout-body">{children}</div>
    </aside>
  );
}
