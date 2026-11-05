import type { DiagnosticVersions } from '@pkms/contracts';
import { useI18n } from '../i18n';
import { diagnosticEnvironment } from './diagnostics-report.ts';

/**
 * The runtime the interface is actually drawing in: application and schema versions, the
 * WebView2 version the native layer reported, and whether that runtime is new enough for
 * the styles the app ships.
 *
 * The installer enforces no minimum (`MINIMUMWEBVIEW2VERSION` is empty in the generated
 * NSIS and the runtime is fetched with `downloadBootstrapper`), so an old runtime starts
 * the app and silently drops style declarations. The shell also shows a dismissible
 * launch banner; this panel keeps the full explanation and install policy.
 * `webview-compat.ts` owns which versions matter and why.
 */
export function RuntimeEnvironment({ versions }: { versions: DiagnosticVersions }) {
  const { language } = useI18n();
  const environment = diagnosticEnvironment(versions, language);
  if (!environment) return null;
  return (
    <div className="diagnostics-counts">
      <dl>
        {environment.rows.map((row) => (
          <div key={row.label}>
            <dt>{row.label}</dt>
            <dd>{row.value}</dd>
          </div>
        ))}
      </dl>
      <p
        className={
          environment.notice.level === 'warn'
            ? 'diagnostics-notice diagnostics-notice-warn'
            : 'diagnostics-muted'
        }
      >
        {environment.notice.text}
      </p>
      <p className="diagnostics-muted">{environment.installNotice}</p>
    </div>
  );
}
