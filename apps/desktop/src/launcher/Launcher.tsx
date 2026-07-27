import { useCallback, useEffect, useState } from "react";

import { createProject, listRecentProjects, openProject, removeRecentProject } from "../api.js";
import type { RecentProjectView } from "../tauri-bridge/generated/bindings.js";
import { connectRemote, probeRemoteDaemon } from "../tauri-bridge/transport.js";
import { pickFolder } from "../tauri-bridge/nativeDialog.js";
import { useContextMenu, useRowContextMenu } from "../components/useRowContextMenu.js";
import { logUi } from "../logger.js";
import {
  forgetRemote,
  loadRecentRemotes,
  normalizeBase,
  rememberRemote,
  type RecentRemote,
} from "./remoteRecents.js";

/// Start screen shown when oxplow launches with no project (a bare
/// Finder/dock launch). Lists recent projects to reopen and offers two
/// separate doors: "New Project…" initializes `.oxplow/` in a plain
/// folder, "Open Project…" only opens a folder that already is a
/// project. Opening a project spawns a fresh process for it
/// (process-per-window) and — for an existing project window —
/// replaces or adds a window. The launcher itself has no `Services`,
/// so it only calls the `commands::launch` surface.
export function Launcher() {
  const [recents, setRecents] = useState<RecentProjectView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    listRecentProjects()
      .then((list) => {
        setRecents(list);
        setLoading(false);
      })
      .catch((e) => {
        logUi("error", "launcher: failed to list recent projects", { error: String(e) });
        setError(String(e));
        setLoading(false);
      });
  }, []);

  useEffect(reload, [reload]);

  const handleOpen = useCallback(
    async (path: string, newWindow: boolean) => {
      setError(null);
      try {
        await openProject(path, newWindow);
        // On a replace-open this process exits before resolving; on a
        // new-window open we stay on the launcher.
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        setError(message);
        logUi("warn", "launcher: open project failed", { path, error: message });
      }
    },
    [],
  );

  const handleRemove = useCallback(
    async (path: string) => {
      try {
        await removeRecentProject(path);
      } catch (e) {
        logUi("warn", "launcher: remove recent failed", { path, error: String(e) });
      }
      reload();
    },
    [reload],
  );

  const handleOpenFolder = useCallback(async () => {
    setError(null);
    const selected = await pickFolder("Open Project");
    if (selected !== null) {
      await handleOpen(selected, false);
    }
  }, [handleOpen]);

  /// Create a project in a folder that isn't one yet. Opens in a new
  /// window (the launcher stays put), and a folder that already is a
  /// project is refused by `create_project` rather than silently opened.
  const handleNewProject = useCallback(async () => {
    setError(null);
    const selected = await pickFolder("New Project");
    if (selected === null) return;
    try {
      await createProject(selected);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      logUi("warn", "launcher: create project failed", { path: selected, error: message });
    }
  }, []);

  return (
    <div data-testid="launcher" style={rootStyle}>
      <div style={cardStyle}>
        <header style={{ marginBottom: 20 }}>
          <h1 style={titleStyle}>Oxplow</h1>
          <p style={subtitleStyle}>Open a project to get started.</p>
        </header>

        {error ? (
          <div data-testid="launcher-error" style={errorStyle}>
            {error}
          </div>
        ) : null}

        <div style={doorsStyle}>
          <button
            type="button"
            data-testid="launcher-new-project"
            onClick={handleNewProject}
            style={newProjectButtonStyle}
          >
            New Project…
          </button>
          <button
            type="button"
            data-testid="launcher-open-project"
            onClick={handleOpenFolder}
            style={openProjectButtonStyle}
          >
            Open Project…
          </button>
        </div>

        <h2 style={sectionHeaderStyle}>Recent Projects</h2>
        {loading ? (
          <div style={emptyStyle}>Loading…</div>
        ) : recents.length === 0 ? (
          <div data-testid="launcher-empty" style={emptyStyle}>
            No recent projects. Create a new one, or open an existing project folder.
          </div>
        ) : (
          <ul style={listStyle}>
            {recents.map((p) => (
              <RecentRow
                key={p.path}
                project={p}
                onOpen={handleOpen}
                onRemove={handleRemove}
              />
            ))}
          </ul>
        )}

        <RemoteConnectSection />
      </div>
    </div>
  );
}

/// "Connect to Remote Daemon" — attach this window to an oxplow-daemon
/// reached through an SSH tunnel. The daemon is project-scoped (it was
/// started with --project on the remote box), so connecting IS picking
/// the project; on success the remote base is persisted and the window
/// reloads into remote mode (see tauri-bridge/transport.ts).
function RemoteConnectSection() {
  const [url, setUrl] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recents, setRecents] = useState<RecentRemote[]>(() =>
    loadRecentRemotes(window.localStorage),
  );

  const connect = useCallback(async (raw: string) => {
    const base = normalizeBase(raw);
    if (!base) return;
    setError(null);
    setConnecting(true);
    try {
      await probeRemoteDaemon(base);
      rememberRemote(window.localStorage, base);
      connectRemote(base); // reloads the window into remote mode
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      setConnecting(false);
      logUi("warn", "launcher: remote daemon probe failed", { base, error: message });
    }
  }, []);

  const handleForget = useCallback((base: string) => {
    setRecents(forgetRemote(window.localStorage, base));
  }, []);

  const cm = useContextMenu();

  return (
    <section>
      <h2 style={sectionHeaderStyle}>Remote Daemon</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void connect(url);
        }}
        style={remoteFormStyle}
      >
        <input
          data-testid="launcher-remote-url"
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://127.0.0.1:7420"
          spellCheck={false}
          style={remoteInputStyle}
          disabled={connecting}
        />
        <button
          type="submit"
          data-testid="launcher-remote-connect"
          disabled={connecting || normalizeBase(url).length === 0}
          style={remoteConnectButtonStyle}
        >
          {connecting ? "Connecting…" : "Connect"}
        </button>
      </form>
      <p style={remoteHintStyle}>
        Start <code>oxplow-daemon --project &lt;dir&gt;</code> on the remote box, tunnel with{" "}
        <code>ssh -L 7420:127.0.0.1:7420 &lt;host&gt;</code>, then connect.
      </p>
      {error ? (
        <div data-testid="launcher-remote-error" style={errorStyle}>
          {error}
        </div>
      ) : null}
      {recents.length > 0 ? (
        <ul style={listStyle}>
          {recents.map((r) => (
            <li
              key={r.base}
              data-testid={`launcher-remote-recent-${r.base}`}
              style={rowStyle}
              onContextMenu={(e) =>
                cm.open(e, [
                  {
                    id: "launcher.remoteForget",
                    label: "Remove from List",
                    enabled: true,
                    run: () => handleForget(r.base),
                  },
                ])
              }
            >
              <button
                type="button"
                onClick={() => void connect(r.base)}
                disabled={connecting}
                style={rowOpenButtonStyle}
                title={r.base}
              >
                <span style={rowTitleStyle}>{r.base}</span>
              </button>
              <div style={rowMetaStyle}>
                <span style={rowTimeStyle}>{formatRelative(Math.floor(r.lastConnectedAt / 1000))}</span>
              </div>
            </li>
          ))}
        </ul>
      ) : null}
      {cm.menu}
    </section>
  );
}

function RecentRow({
  project,
  onOpen,
  onRemove,
}: {
  project: RecentProjectView;
  onOpen: (path: string, newWindow: boolean) => void;
  onRemove: (path: string) => void;
}) {
  const items = [
    {
      id: "launcher.open",
      label: "Open",
      enabled: project.exists,
      run: () => onOpen(project.path, false),
    },
    {
      id: "launcher.openNewWindow",
      label: "Open in New Window",
      enabled: project.exists,
      run: () => onOpen(project.path, true),
    },
    {
      id: "launcher.remove",
      label: "Remove from List",
      enabled: true,
      run: () => onRemove(project.path),
    },
  ];

  const cm = useRowContextMenu(items);
  return (
    <li
      data-testid={`launcher-recent-${project.path}`}
      style={{ ...rowStyle, opacity: project.exists ? 1 : 0.55 }}
      onContextMenu={cm.onContextMenu}
    >
      <button
        type="button"
        onClick={() => project.exists && onOpen(project.path, false)}
        disabled={!project.exists}
        style={rowOpenButtonStyle}
        title={project.exists ? project.path : `${project.path} (missing)`}
      >
        <span style={rowTitleStyle}>{project.title}</span>
        <span style={rowPathStyle}>{project.path}</span>
      </button>
      <div style={rowMetaStyle}>
        {!project.exists ? <span style={missingBadgeStyle}>missing</span> : null}
        <span style={rowTimeStyle}>{formatRelative(project.lastOpenedAt)}</span>
      </div>
      {cm.menu}
    </li>
  );
}

/// Compact "x ago" rendering of a unix-seconds timestamp.
function formatRelative(unixSeconds: number): string {
  const deltaSec = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
  if (deltaSec < 60) return "just now";
  const min = Math.floor(deltaSec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const days = Math.floor(hr / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(unixSeconds * 1000).toLocaleDateString();
}

const rootStyle: React.CSSProperties = {
  height: "100vh",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
};

const cardStyle: React.CSSProperties = {
  width: 560,
  maxWidth: "90vw",
  maxHeight: "85vh",
  display: "flex",
  flexDirection: "column",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 10,
  padding: 28,
  boxSizing: "border-box",
};

const titleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 22,
  fontWeight: "var(--weight-bold)" as unknown as number,
};

const subtitleStyle: React.CSSProperties = {
  margin: "4px 0 0",
  color: "var(--text-secondary)",
};

const sectionHeaderStyle: React.CSSProperties = {
  margin: "20px 0 8px",
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: "var(--text-muted)",
};

const doorsStyle: React.CSSProperties = {
  display: "flex",
  gap: 8,
};

const doorButtonStyle: React.CSSProperties = {
  flex: 1,
  padding: "10px 14px",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: "var(--weight-medium)" as unknown as number,
};

/// New Project is the accented primary — the launcher's job for a
/// first-time user is getting a project made.
const newProjectButtonStyle: React.CSSProperties = {
  ...doorButtonStyle,
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
  border: "none",
};

const openProjectButtonStyle: React.CSSProperties = {
  ...doorButtonStyle,
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
};

const listStyle: React.CSSProperties = {
  listStyle: "none",
  margin: 0,
  padding: 0,
  overflowY: "auto",
};

const rowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "8px 4px",
  borderBottom: "1px solid var(--border-subtle)",
};

const rowOpenButtonStyle: React.CSSProperties = {
  flex: 1,
  minWidth: 0,
  display: "flex",
  flexDirection: "column",
  alignItems: "flex-start",
  gap: 2,
  background: "transparent",
  border: "none",
  cursor: "pointer",
  textAlign: "left",
  padding: 0,
  color: "inherit",
};

const rowTitleStyle: React.CSSProperties = {
  fontWeight: "var(--weight-medium)" as unknown as number,
  color: "var(--text-primary)",
};

const rowPathStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
  maxWidth: "100%",
};

const rowMetaStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexShrink: 0,
};

const rowTimeStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
};

const missingBadgeStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--severity-critical)",
  border: "1px solid var(--severity-critical)",
  borderRadius: 4,
  padding: "0 4px",
};

const emptyStyle: React.CSSProperties = {
  color: "var(--text-muted)",
  padding: "12px 4px",
};

const errorStyle: React.CSSProperties = {
  background: "var(--accent-soft-bg)",
  color: "var(--severity-critical)",
  border: "1px solid var(--severity-critical)",
  borderRadius: 6,
  padding: "8px 12px",
  marginBottom: 12,
  fontSize: "var(--text-xs)",
};

const remoteFormStyle: React.CSSProperties = {
  display: "flex",
  gap: 8,
};

const remoteInputStyle: React.CSSProperties = {
  flex: 1,
  minWidth: 0,
  padding: "8px 10px",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
};

const remoteConnectButtonStyle: React.CSSProperties = {
  padding: "8px 14px",
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
  border: "none",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: "var(--weight-medium)" as unknown as number,
};

const remoteHintStyle: React.CSSProperties = {
  margin: "8px 0 12px",
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
  lineHeight: 1.5,
};
