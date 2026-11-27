import { useCallback, useEffect, useState } from "react";

import { InlineConfirm } from "../components/InlineConfirm.js";
import { forgetHostToken, hostOrigin, hostToken } from "../tauri-bridge/transport.js";
import { forgetDevice, HostRefusal, listProjects, setHold, type HostProject } from "./hostApi.js";
import { PairScreen } from "./PairScreen.js";
import {
  cardStyle,
  emptyStyle,
  errorStyle,
  listStyle,
  primaryButtonStyle,
  rootStyle,
  rowStyle,
  rowTitleStyle,
  sectionHeaderStyle,
  subtitleStyle,
  titleStyle,
} from "../launcher/screenStyles.js";

/** How often the list is read again while a project starts or stops. */
const WHILE_CHANGING_MS = 1_500;

export const STATE_WORDS: Record<HostProject["state"], string> = {
  starting: "starting…",
  running: "running",
  stopping: "stopping…",
  stopped: "not running",
  crashed: "did not start",
};

/** The host's API; one object, so the reads below keep their identity. */
const HOST_API = { listProjects, setHold, forgetDevice };

/** The host's home (`/`) on a device: pair first, then the host's projects.
 *  Open holds the project for this device (its daemon starts, and runs while
 *  any device holds it) and goes to its page; Close lets go of it. A
 *  project's page lists the projects too (the tablet's switcher), and links
 *  back here to let go. `api` and `go` are a test's stand-ins. */
export function HostHome({
  api = HOST_API,
  go = (path: string) => window.location.assign(path),
}: {
  api?: { listProjects: typeof listProjects; setHold: typeof setHold; forgetDevice: typeof forgetDevice };
  go?: (path: string) => void;
}) {
  const host = hostOrigin() ?? "";
  const token = hostToken();
  const [projects, setProjects] = useState<HostProject[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    if (token === null) return;
    try {
      setProjects(await api.listProjects(host, token));
      setError(null);
    } catch (e) {
      // This device was removed: pair again.
      if (e instanceof HostRefusal && e.status === 401) forgetHostToken();
      else setError(e instanceof Error ? e.message : String(e));
    }
  }, [api, host, token]);

  useEffect(() => {
    void read();
  }, [read]);

  const changing = projects?.some((p) => p.state === "starting" || p.state === "stopping") ?? false;
  useEffect(() => {
    if (!changing) return;
    const timer = setInterval(() => void read(), WHILE_CHANGING_MS);
    return () => clearInterval(timer);
  }, [changing, read]);

  if (token === null) return <PairScreen />;

  const hold = async (project: HostProject, held: boolean) => {
    try {
      await api.setHold(host, token, project.id, held);
      if (held) go(`/p/${project.id}/`);
      else await read();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div data-testid="host-home" style={rootStyle}>
      <div style={cardStyle}>
        <header style={{ marginBottom: 12 }}>
          <h1 style={titleStyle}>Oxplow</h1>
          <p style={subtitleStyle}>The projects on {host.replace(/^https?:\/\//, "")}.</p>
        </header>
        {error ? (
          <div data-testid="host-home-error" role="alert" style={errorStyle}>
            {error}
          </div>
        ) : null}
        <h2 style={sectionHeaderStyle}>Projects</h2>
        {projects === null ? (
          <div style={emptyStyle}>Loading…</div>
        ) : projects.length === 0 ? (
          <div data-testid="host-home-empty" style={emptyStyle}>
            No projects yet. Open one in Oxplow on the computer that runs it.
          </div>
        ) : (
          <ul style={listStyle}>
            {projects.map((p) => (
              <li key={p.id} data-testid={`host-project-${p.id}`} style={{ ...rowStyle, gap: 8 }}>
                <span style={{ ...rowTitleStyle, flex: 1 }}>{p.name}</span>
                <span style={{ color: "var(--text-muted)" }}>{STATE_WORDS[p.state] ?? p.state}</span>
                {p.held_by_you ? (
                  <button
                    type="button"
                    data-testid={`host-project-close-${p.id}`}
                    onClick={() => void hold(p, false)}
                    title="Let go of it here; it stops when no device holds it"
                  >
                    Close
                  </button>
                ) : null}
                <button
                  type="button"
                  data-testid={`host-project-open-${p.id}`}
                  onClick={() => void hold(p, true)}
                  style={primaryButtonStyle}
                >
                  Open
                </button>
              </li>
            ))}
          </ul>
        )}
        <div style={{ marginTop: 20 }}>
          <InlineConfirm
            triggerLabel="Forget this device"
            confirmLabel="Forget"
            testIdPrefix="host-forget"
            title="This device must pair again to use this host"
            onConfirm={() => {
              void api.forgetDevice(host, token).finally(forgetHostToken);
            }}
          />
        </div>
      </div>
    </div>
  );
}
