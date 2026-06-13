import { useEffect, useState } from "react";

import {
  disconnectRemote,
  isRemote,
  onRemoteConnectionState,
  remoteBaseUrl,
  type RemoteConnectionState,
} from "../tauri-bridge/transport.js";

/// What the banner shows. `hidden` while the connection is healthy;
/// `down` while the WS is dropped (transport keeps retrying with
/// backoff); `restored` once it comes back. On recovery the transport
/// auto-resyncs the client stores (see `onRemoteReconnect` in
/// transport.ts), so `restored` is just a brief non-blocking
/// confirmation — no reload prompt, and it auto-dismisses. The
/// daemon-side work (tmux agents, watchers) ran through the gap either
/// way.
export type BannerState = "hidden" | "down" | "restored";

/// How long the "reconnected" confirmation lingers before it
/// auto-dismisses (ms). Non-blocking — purely informational.
export const RESTORED_AUTO_DISMISS_MS = 4000;

/// Pure transition: drop → down; recovery after a drop → restored
/// (sticky until dismissed); an "up" with no preceding drop (the
/// initial connect) stays hidden.
export function nextBannerState(prev: BannerState, event: RemoteConnectionState): BannerState {
  if (event === "down") return "down";
  return prev === "down" ? "restored" : prev;
}

/// Top-of-window strip shown only in remote mode. Mounted once in App.
export function RemoteConnectionBanner() {
  const [state, setState] = useState<BannerState>("hidden");

  useEffect(() => {
    if (!isRemote()) return;
    return onRemoteConnectionState((event) => {
      setState((prev) => nextBannerState(prev, event));
    });
  }, []);

  // The "reconnected" confirmation is informational only — auto-dismiss
  // it after a few seconds so it doesn't linger over the UI.
  useEffect(() => {
    if (state !== "restored") return;
    const t = setTimeout(() => setState("hidden"), RESTORED_AUTO_DISMISS_MS);
    return () => clearTimeout(t);
  }, [state]);

  if (!isRemote() || state === "hidden") return null;

  if (state === "down") {
    return (
      <div data-testid="remote-banner-down" style={{ ...stripStyle, ...downStyle }}>
        <span>
          Connection to remote daemon lost ({remoteBaseUrl()}) — reconnecting… Agent work keeps
          running on the remote box.
        </span>
        <button
          type="button"
          data-testid="remote-banner-disconnect"
          onClick={() => disconnectRemote()}
          style={buttonStyle}
        >
          Disconnect
        </button>
      </div>
    );
  }

  return (
    <div data-testid="remote-banner-restored" style={{ ...stripStyle, ...restoredStyle }}>
      <span>Connection restored — state resynced.</span>
      <button
        type="button"
        data-testid="remote-banner-dismiss"
        onClick={() => setState("hidden")}
        style={buttonStyle}
      >
        Dismiss
      </button>
    </div>
  );
}

const stripStyle: React.CSSProperties = {
  position: "fixed",
  top: 0,
  left: 0,
  right: 0,
  zIndex: 1000,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 12,
  padding: "6px 12px",
  fontSize: "var(--text-xs)",
};

const downStyle: React.CSSProperties = {
  background: "var(--severity-critical)",
  color: "var(--accent-on-accent)",
};

const restoredStyle: React.CSSProperties = {
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
};

const buttonStyle: React.CSSProperties = {
  padding: "2px 10px",
  background: "rgba(0,0,0,0.25)",
  color: "inherit",
  border: "1px solid rgba(255,255,255,0.4)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
