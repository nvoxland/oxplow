/// `embed` viz (`.context/external-url-tabs.md`): a web page. Tauri 2 has no
/// browser tag inside a webview, so the page opens in a sandboxed window
/// (capability `external-url`) and this is its status panel: where it opened,
/// Open again, Open in Browser (the system browser) and Copy URL. Opened once
/// per tab entry (`opened` in its history state), so a restored tab doesn't
/// spawn a window again. A URL the allowlist refuses draws "Couldn't open
/// link" and opens nothing.
///
/// Security stance: the URL is gated through `classifyExternalUrl` before any
/// open; the window inherits no oxplow commands and no plugin permissions —
/// effectively a browser tab with storage isolated per window.
import { useEffect, useState } from "react";

import { desktopBridge, openInSystemBrowser } from "../api.js";
import { classifyExternalUrl, describeRejection } from "../external-url-allowlist.js";
import { useHistoryState } from "../tabs/useHistoryState.js";

export function EmbedViz({ url }: { url: string | null }) {
  const verdict = classifyExternalUrl(url ?? "");
  const [opened, setOpened] = useHistoryState<string | null>("opened", null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  async function open() {
    if (!verdict.ok) return;
    setOpening(true);
    setOpenError(null);
    const result = await desktopBridge().openExternalUrl(verdict.url);
    setOpening(false);
    if (!result.ok) setOpenError(result.reason ?? "Failed to open link");
  }

  // Opened once per entry, and again when the page it shows changes.
  const target = verdict.ok ? verdict.url : null;
  useEffect(() => {
    if (target === null || opened === target) return;
    setOpened(target);
    void open();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target]);

  if (!url) return <p className="ox-empty-line">This page names no web page.</p>;
  if (!verdict.ok) {
    return (
      <div data-testid="lens-embed-blocked" style={panelStyle}>
        <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)", marginBottom: 8 }}>
          Couldn't open link
        </div>
        <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)", marginBottom: 12 }}>
          {describeRejection(verdict.reason)}
        </div>
        <pre style={urlStyle}>{url}</pre>
      </div>
    );
  }
  const safeUrl = verdict.url;
  const host = new URL(safeUrl).host;
  return (
    <div data-testid="lens-embed" style={{ ...panelStyle, color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
      <div style={{ marginBottom: 8, color: "var(--text-primary)" }}>
        {opening ? "Opening…" : `Opened ${host} in a separate sandboxed window.`}
      </div>
      <div style={{ marginBottom: 12 }}>
        External pages run outside the app's webview so they can't reach oxplow's IPC surface. Cookies and storage
        are isolated per host.
      </div>
      <pre style={{ ...urlStyle, marginBottom: 12 }}>{safeUrl}</pre>
      {openError ? (
        <div data-testid="lens-embed-error" style={errorStyle}>
          {openError}
        </div>
      ) : null}
      <div style={{ display: "flex", gap: 8 }}>
        <button type="button" className="ox-button" data-testid="lens-embed-reopen" disabled={opening} onClick={() => void open()}>
          {opening ? "Opening…" : "Open again"}
        </button>
        <button type="button" className="ox-button" data-testid="lens-embed-browser" onClick={() => void openInSystemBrowser(safeUrl)}>
          Open in Browser
        </button>
        <button
          type="button"
          className="ox-button"
          data-testid="lens-embed-copy"
          onClick={() => void navigator.clipboard.writeText(safeUrl).catch(() => {})}
        >
          Copy URL
        </button>
      </div>
    </div>
  );
}

const panelStyle = { padding: "16px 20px", maxWidth: 720 } as const;
const urlStyle = {
  background: "var(--surface-app)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "8px 10px",
  fontSize: "var(--text-xs)",
  color: "var(--text-primary)",
  margin: 0,
  whiteSpace: "pre-wrap",
  wordBreak: "break-all",
} as const;
const errorStyle = {
  padding: "8px 12px",
  background: "var(--severity-critical-soft, var(--surface-app))",
  color: "var(--severity-critical)",
  fontSize: "var(--text-xs)",
  borderRadius: 6,
  marginBottom: 12,
} as const;
