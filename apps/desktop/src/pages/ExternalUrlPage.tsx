import { useEffect, useState } from "react";
import { Page } from "../tabs/Page.js";
import { classifyExternalUrl, describeRejection } from "../external-url-allowlist.js";
import { desktopBridge } from "../api.js";

export interface ExternalUrlPageProps {
  url: string;
  /** "Open in browser" handler — wired by the host. */
  onOpenInBrowser?: (url: string) => void;
}

/**
 * Tauri 2 doesn't embed a browser-tag inside a webview. External URL
 * tabs open the link in a sandboxed `WebviewWindow` (capability
 * `external-url`, see `apps/desktop/src-tauri/capabilities/`) and
 * this React page becomes a status / re-open panel.
 *
 * Security stance preserved by the new model:
 * - The URL is gated through `classifyExternalUrl` before any open
 *   call; non-http(s) renders a refusal in this page.
 * - The opened window inherits zero oxplow commands and zero Tauri plugin
 *   permissions — it's effectively a browser tab.
 * - Cookies/storage isolation is provided by Tauri's per-window
 *   webview context, replacing the Electron `partition` mechanism.
 */
export function ExternalUrlPage({ url, onOpenInBrowser }: ExternalUrlPageProps) {
  const verdict = classifyExternalUrl(url);
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  async function openInWindow() {
    if (!verdict.ok) return;
    setOpening(true);
    setOpenError(null);
    const result = await desktopBridge().openExternalUrl(verdict.url);
    setOpening(false);
    if (!result.ok) {
      setOpenError(result.reason ?? "Failed to open link");
    }
  }

  // Auto-open once on mount when the URL is allowed.
  useEffect(() => {
    if (verdict.ok) void openInWindow();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [url]);

  if (!verdict.ok) {
    const reason = describeRejection(verdict.reason);
    return (
      <Page testId="page-external-url" title="Link Blocked" kind="external-url">
        <div style={{ padding: "16px 20px", maxWidth: 720 }}>
          <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)", marginBottom: 8 }}>
            Couldn't open link
          </div>
          <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 12 }}>{reason}</div>
          <pre
            style={{
              background: "var(--surface-app)",
              border: "1px solid var(--border-subtle)",
              borderRadius: 6,
              padding: "8px 10px",
              fontSize: "var(--text-xs)",
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
  const host = new URL(safeUrl).host;
  const chips = [{ label: opening ? "opening…" : host }];
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
        fontSize: "var(--text-xs)",
      }}
    >
      Open in browser
    </button>
  ) : null;

  return (
    <Page testId="page-external-url" title={host} kind="external-url" chips={chips} actions={actions}>
      <div
        data-testid="page-external-url-status"
        style={{
          padding: "16px 20px",
          maxWidth: 720,
          color: "var(--text-secondary)",
          fontSize: "var(--text-sm)",
        }}
      >
        <div style={{ marginBottom: 8, color: "var(--text-primary)" }}>
          Opened in a separate sandboxed window.
        </div>
        <div style={{ marginBottom: 12 }}>
          External pages run outside the app's webview so they can't reach oxplow's
          IPC surface. Cookies and storage are isolated per host.
        </div>
        <pre
          style={{
            background: "var(--surface-app)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            padding: "8px 10px",
            fontSize: "var(--text-xs)",
            color: "var(--text-primary)",
            margin: 0,
            marginBottom: 12,
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
          }}
        >
          {safeUrl}
        </pre>
        {openError ? (
          <div
            data-testid="page-external-url-error"
            style={{
              padding: "8px 12px",
              background: "var(--severity-critical-soft, var(--surface-app))",
              color: "var(--severity-critical)",
              fontSize: "var(--text-xs)",
              borderRadius: 6,
              marginBottom: 12,
            }}
          >
            {openError}
          </div>
        ) : null}
        <button
          type="button"
          data-testid="page-external-url-reopen"
          onClick={() => void openInWindow()}
          disabled={opening}
          style={{
            padding: "4px 10px",
            background: "var(--surface-tab-inactive)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            cursor: opening ? "default" : "pointer",
            fontSize: "var(--text-xs)",
          }}
        >
          {opening ? "Opening…" : "Open again"}
        </button>
      </div>
    </Page>
  );
}
