import { useCallback, useEffect, useState } from "react";

import { askHost, collectHost, forgetRemoteHost, openRemoteProject, remoteHosts } from "../api.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import type { HostAsk, RemoteHostView } from "../tauri-bridge/generated/bindings.js";
import {
  emptyStyle,
  errorStyle,
  formStyle,
  hintStyle,
  inputStyle,
  listStyle,
  primaryButtonStyle,
  rowStyle,
  rowTitleStyle,
  sectionHeaderStyle,
} from "./screenStyles.js";

export type RemoteHostsApi = {
  remoteHosts: typeof remoteHosts;
  askHost: typeof askHost;
  collectHost: typeof collectHost;
  openRemoteProject: typeof openRemoteProject;
  forgetRemoteHost: typeof forgetRemoteHost;
};

const REMOTE_HOSTS_API: RemoteHostsApi = { remoteHosts, askHost, collectHost, openRemoteProject, forgetRemoteHost };

/** The pause between two asks whether the person there typed the code. */
export const COLLECT_PAUSE_MS = 500;

/** The launcher's hosts on other computers (a headless host on a server,
 *  `oxplow host serve`): connect with its address, and type the code this
 *  Mac shows there (`oxplow host pair <code>`), then open its projects here. A project there runs
 *  there: quitting here leaves it running. */
export function RemoteHostsSection({
  api = REMOTE_HOSTS_API,
  pause = COLLECT_PAUSE_MS,
}: {
  api?: RemoteHostsApi;
  /** The pause between two collects (a test's is 0). */
  pause?: number;
}) {
  const [hosts, setHosts] = useState<RemoteHostView[]>([]);
  const [address, setAddress] = useState("");
  /** This Mac's ask, while it waits for the person there to type its code. */
  const [asking, setAsking] = useState<HostAsk | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    try {
      setHosts(await api.remoteHosts());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [api]);

  useEffect(() => {
    void read();
  }, [read]);

  // Wait for the person there to type the code: each collect waits a
  // while; ask again until paired, ended, or cancelled.
  useEffect(() => {
    if (!asking) return;
    let live = true;
    void (async () => {
      try {
        while (live && !(await api.collectHost(asking.origin, asking.request))) {
          // Still waiting (the host waits a while each time): ask again,
          // never in a tight loop.
          await new Promise((r) => setTimeout(r, pause));
        }
        if (!live) return;
        setAsking(null);
        setAddress("");
        await read();
      } catch (e) {
        if (!live) return;
        setAsking(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [asking, pause, api, read]);

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
      await read();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section data-testid="launcher-remote-hosts">
      <h2 style={sectionHeaderStyle}>Other computers</h2>
      {hosts.length === 0 ? (
        <div style={emptyStyle}>No other computer yet.</div>
      ) : (
        hosts.map((h) => (
          <div key={h.origin} data-testid={`remote-host-${h.origin}`} style={{ marginBottom: 12 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ ...rowTitleStyle, flex: 1 }}>{h.origin.replace(/^https?:\/\//, "")}</span>
              <InlineConfirm
                triggerLabel="Forget"
                confirmLabel="Forget"
                testIdPrefix={`remote-host-forget-${h.origin}`}
                title="This Mac must pair again to use it; its projects there keep running"
                onConfirm={() => void run(() => api.forgetRemoteHost(h.origin))}
              />
            </div>
            {h.error ? (
              <div style={errorStyle}>It did not answer: {h.error}</div>
            ) : h.otherVersion ? (
              <div style={hintStyle}>It runs Oxplow {h.version}; this Mac runs another version.</div>
            ) : null}
            <ul style={listStyle}>
              {h.projects.map((p) => (
                <li key={p.id} style={rowStyle} data-testid={`remote-project-${p.id}`}>
                  <span style={{ flex: 1 }}>{p.name}</span>
                  <span style={hintStyle}>{p.state === "running" ? "running" : p.state}</span>
                  <button
                    type="button"
                    data-testid={`remote-project-open-${p.id}`}
                    disabled={busy}
                    onClick={() => void run(() => api.openRemoteProject(h.origin, p.id, true))}
                  >
                    Open
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ))
      )}
      {asking ? (
        <div data-testid="remote-host-ask" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div>
            On {asking.origin.replace(/^https?:\/\//, "")}, run{" "}
            <code>oxplow host pair {asking.code}</code> (or type the code in its Devices page):
          </div>
          <div
            data-testid="remote-host-ask-code"
            style={{ fontSize: "var(--text-xl)", fontWeight: 600, letterSpacing: "0.1em", fontFamily: "var(--font-mono)" }}
          >
            {asking.code}
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={hintStyle}>Waiting for it there…</span>
            <button type="button" data-testid="remote-host-ask-cancel" onClick={() => setAsking(null)}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (address.trim() === "") return;
            void run(async () => setAsking(await api.askHost(address)));
          }}
          style={formStyle}
        >
          <input
            data-testid="remote-host-address"
            aria-label="The host's address"
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            placeholder="server.tailnet.ts.net:7400"
            spellCheck={false}
            disabled={busy}
            style={inputStyle}
          />
          <button
            type="submit"
            data-testid="remote-host-connect"
            disabled={busy || address.trim() === ""}
            style={primaryButtonStyle}
          >
            {busy ? "Connecting…" : "Connect"}
          </button>
        </form>
      )}
      <p style={hintStyle}>
        On the other computer run <code>oxplow host serve</code>, then type its address here: this Mac
        shows a code to type there.
      </p>
      {error ? (
        <div data-testid="remote-host-error" role="alert" style={errorStyle}>
          {error}
        </div>
      ) : null}
    </section>
  );
}
