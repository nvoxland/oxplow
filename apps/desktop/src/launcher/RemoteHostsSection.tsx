import { useCallback, useEffect, useState } from "react";

import { connectHost, forgetRemoteHost, openRemoteProject, remoteHosts } from "../api.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import type { RemoteHostView } from "../tauri-bridge/generated/bindings.js";
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
  connectHost: typeof connectHost;
  openRemoteProject: typeof openRemoteProject;
  forgetRemoteHost: typeof forgetRemoteHost;
};

const REMOTE_HOSTS_API: RemoteHostsApi = { remoteHosts, connectHost, openRemoteProject, forgetRemoteHost };

/** The launcher's hosts on other computers (a headless host on a server,
 *  `oxplow host serve`): connect with its address and a code from `oxplow
 *  host pair` there, then open its projects here. A project there runs
 *  there: quitting here leaves it running. */
export function RemoteHostsSection({ api = REMOTE_HOSTS_API }: { api?: RemoteHostsApi }) {
  const [hosts, setHosts] = useState<RemoteHostView[]>([]);
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
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
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (address.trim() === "" || code.trim() === "") return;
          void run(async () => {
            await api.connectHost(address, code);
            setAddress("");
            setCode("");
          });
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
        <input
          data-testid="remote-host-code"
          aria-label="Pairing code"
          value={code}
          onChange={(e) => setCode(e.target.value.toUpperCase())}
          placeholder="ABCD-EFGH"
          spellCheck={false}
          autoComplete="off"
          disabled={busy}
          style={inputStyle}
        />
        <button
          type="submit"
          data-testid="remote-host-connect"
          disabled={busy || address.trim() === "" || code.trim() === ""}
          style={primaryButtonStyle}
        >
          {busy ? "Connecting…" : "Connect"}
        </button>
      </form>
      <p style={hintStyle}>
        On the other computer run <code>oxplow host serve</code> and <code>oxplow host pair</code>, then
        type its address and the code here.
      </p>
      {error ? (
        <div data-testid="remote-host-error" role="alert" style={errorStyle}>
          {error}
        </div>
      ) : null}
    </section>
  );
}
