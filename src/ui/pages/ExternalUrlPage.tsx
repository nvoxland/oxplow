import { useEffect, useRef, useState } from "react";
import { Page } from "../tabs/Page.js";
import { classifyExternalUrl, describeRejection } from "../external-url-allowlist.js";

// React's built-in `webview` JSX intrinsic carries the Electron-style
// attributes we need (partition, webpreferences, src). We only set the
// attributes that lock the guest down further; `allowpopups`,
// `disablewebsecurity`, `nodeintegration`, `plugins` are deliberately
// left at default-off.

export interface ExternalUrlPageProps {
  url: string;
  /** Right-click "Open in browser" handler — wired by the host. */
  onOpenInBrowser?: (url: string) => void;
}

const PARTITION = "persist:external";

/**
 * Renders an external http(s) page inside a sandboxed Electron <webview>.
 *
 * Security stance (also enforced redundantly by main process; see
 * .context/agent-model.md "External URL tabs"):
 *
 * - `contextIsolation=yes,sandbox=yes` in webpreferences forces the
 *   guest into a sandbox process with an isolated context — no Node,
 *   no preload exposure of app APIs.
 * - `partition="persist:external"` keeps cookies/storage isolated from
 *   the app session so authenticated app endpoints can't leak to
 *   embedded sites.
 * - URL is gated through `classifyExternalUrl` before mount — anything
 *   non-http(s) renders a refusal instead of attaching the webview.
 * - No `allowpopups` — popups are denied; window.open targets that
 *   should open route through the main process's setWindowOpenHandler
 *   which goes through the same allowlist.
 */
export function ExternalUrlPage({ url, onOpenInBrowser }: ExternalUrlPageProps) {
  const verdict = classifyExternalUrl(url);
  const webviewRef = useRef<HTMLElement | null>(null);
  const [pageTitle, setPageTitle] = useState<string>(verdict.ok ? verdict.url : "External link");
  const [loading, setLoading] = useState<boolean>(verdict.ok);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    const el = webviewRef.current;
    if (!el || !verdict.ok) return;
    const onTitle = (event: Event) => {
      const next = (event as unknown as { title?: string }).title;
      if (typeof next === "string" && next.length > 0) setPageTitle(next);
    };
    const onStart = () => { setLoading(true); setLoadError(null); };
    const onStop = () => { setLoading(false); };
    const onFail = (event: Event) => {
      const detail = event as unknown as { errorDescription?: string; errorCode?: number; isMainFrame?: boolean };
      // Sub-frame failures are common (analytics blocked, etc.) and not
      // worth surfacing — only flag main-frame load failures.
      if (detail.isMainFrame === false) return;
      setLoading(false);
      setLoadError(detail.errorDescription ?? "Failed to load");
    };
    el.addEventListener("page-title-updated", onTitle);
    el.addEventListener("did-start-loading", onStart);
    el.addEventListener("did-stop-loading", onStop);
    el.addEventListener("did-fail-load", onFail);
    return () => {
      el.removeEventListener("page-title-updated", onTitle);
      el.removeEventListener("did-start-loading", onStart);
      el.removeEventListener("did-stop-loading", onStop);
      el.removeEventListener("did-fail-load", onFail);
    };
  }, [verdict.ok]);

  if (!verdict.ok) {
    const reason = describeRejection(verdict.reason);
    return (
      <Page testId="page-external-url" title="Link blocked" kind="external-url">
        <div style={{ padding: "16px 20px", maxWidth: 720 }}>
          <div style={{ color: "var(--severity-critical)", fontSize: 13, marginBottom: 8 }}>
            Couldn't open link
          </div>
          <div style={{ color: "var(--text-secondary)", fontSize: 12, marginBottom: 12 }}>{reason}</div>
          <pre
            style={{
              background: "var(--surface-app)",
              border: "1px solid var(--border-subtle)",
              borderRadius: 6,
              padding: "8px 10px",
              fontSize: 12,
              color: "var(--text-primary)",
              margin: 0,
              whiteSpace: "pre-wrap",
              wordBreak: "break-all",
            }}
          >
            {url}
          </pre>
        </div>
      </Page>
    );
  }

  const safeUrl = verdict.url;
  const chips = [
    { label: loading ? "loading…" : new URL(safeUrl).host },
  ];
  const actions = onOpenInBrowser ? (
    <button
      type="button"
      data-testid="page-external-url-open-in-browser"
      onClick={() => onOpenInBrowser(safeUrl)}
      style={{
        padding: "4px 10px",
        background: "var(--surface-tab-inactive)",
        color: "var(--text-primary)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        cursor: "pointer",
        fontSize: 12,
      }}
    >
      Open in browser
    </button>
  ) : null;

  return (
    <Page testId="page-external-url" title={pageTitle} kind="external-url" chips={chips} actions={actions}>
      <div style={{ position: "relative", height: "100%", minHeight: 0, display: "flex", flexDirection: "column" }}>
        {loadError ? (
          <div
            data-testid="page-external-url-error"
            style={{
              padding: "8px 12px",
              background: "var(--severity-critical-soft, var(--surface-app))",
              color: "var(--severity-critical)",
              fontSize: 12,
              borderBottom: "1px solid var(--border-subtle)",
            }}
          >
            {loadError}
          </div>
        ) : null}
        <webview
          ref={(el: HTMLElement | null) => { webviewRef.current = el; }}
          src={safeUrl}
          partition={PARTITION}
          webpreferences="contextIsolation=yes,sandbox=yes,nodeIntegration=no"
          data-testid="page-external-url-webview"
          style={{ flex: 1, minHeight: 0, width: "100%", border: 0, background: "white" }}
        />
      </div>
    </Page>
  );
}
