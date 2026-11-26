import { useCallback, useEffect, useState } from "react";

import { createProject, listRecentProjects, openProject, removeRecentProject } from "../api.js";
import type { RecentProjectView } from "../tauri-bridge/generated/bindings.js";
import { connectRemote, parseRemoteInput, probeRemoteDaemon } from "../tauri-bridge/transport.js";
import { pickFolder } from "../tauri-bridge/nativeDialog.js";
import { useContextMenu, useRowContextMenu } from "../components/useRowContextMenu.js";
import { formatRelativeSeconds } from "../components/format.js";
import { logUi } from "../logger.js";
import type { MenuItem } from "../menu.js";
import { openInForPath } from "../openIn.js";
import {
  forgetRemote,
  loadRecentRemotes,
  normalizeBase,
  rememberRemote,
  type RecentRemote,
} from "./remoteRecents.js";
import { shellStorage } from "../shellStorage.js";
import {
  rootStyle,
  cardStyle,
  titleStyle,
  subtitleStyle,
  sectionHeaderStyle,
  doorsStyle,
  newProjectButtonStyle,
  openProjectButtonStyle,
  listStyle,
  rowStyle,
  rowOpenButtonStyle,
  rowTitleStyle,
  rowPathStyle,
  rowMetaStyle,
  rowTimeStyle,
  missingBadgeStyle,
  emptyStyle,
  errorStyle,
  remoteFormStyle,
  remoteInputStyle,
  remoteConnectButtonStyle,
  remoteHintStyle,
} from "./screenStyles.js";

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
  // The daemon prints its UI token ("ui token: …") when it starts; the
  // address may also carry it as `#oxplow-token=…`.
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recents, setRecents] = useState<RecentRemote[]>(() =>
    loadRecentRemotes(shellStorage),
  );

  const connect = useCallback(async (raw: string, typedToken: string) => {
    const parsed = parseRemoteInput(raw);
    const base = normalizeBase(parsed.base);
    if (!base) return;
    const uiToken = parsed.token ?? (typedToken.trim() || null);
    setError(null);
    setConnecting(true);
    try {
      await probeRemoteDaemon(base, uiToken);
      rememberRemote(shellStorage, base);
      connectRemote(base, uiToken); // reloads the window into remote mode
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      setConnecting(false);
      logUi("warn", "launcher: remote daemon probe failed", { base, error: message });
    }
  }, []);

  const handleForget = useCallback((base: string) => {
    setRecents(forgetRemote(shellStorage, base));
  }, []);

  const cm = useContextMenu();

  return (
    <section>
      <h2 style={sectionHeaderStyle}>Remote Daemon</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void connect(url, token);
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
        <input
          data-testid="launcher-remote-token"
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="UI token"
          spellCheck={false}
          autoComplete="off"
          style={remoteInputStyle}
          disabled={connecting}
        />
        <button
          type="submit"
          data-testid="launcher-remote-connect"
          disabled={connecting || normalizeBase(parseRemoteInput(url).base).length === 0}
          style={remoteConnectButtonStyle}
        >
          {connecting ? "Connecting…" : "Connect"}
        </button>
      </form>
      <p style={remoteHintStyle}>
        Start <code>oxplow-daemon --project &lt;dir&gt;</code> on the remote box, tunnel with{" "}
        <code>ssh -L 7420:127.0.0.1:7420 &lt;host&gt;</code>, then connect with the{" "}
        <code>ui token</code> it printed.
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
                onClick={() => void connect(r.base, token)}
                disabled={connecting}
                style={rowOpenButtonStyle}
                title={r.base}
              >
                <span style={rowTitleStyle}>{r.base}</span>
              </button>
              <div style={rowMetaStyle}>
                <span style={rowTimeStyle}>{formatRelativeSeconds(Math.floor(r.lastConnectedAt / 1000))}</span>
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
  const items: MenuItem[] = [
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
    // The project's folder in the OS file manager (`openIn.ts`).
    ...[project.exists ? openInForPath("dir", project.path, [], { terminal: false }) : null].filter(
      (item): item is MenuItem => item !== null,
    ),
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
        <span style={rowTimeStyle}>{formatRelativeSeconds(project.lastOpenedAt)}</span>
      </div>
      {cm.menu}
    </li>
  );
}
