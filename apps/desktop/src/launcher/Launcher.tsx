import { useCallback, useEffect, useState } from "react";

import { createProject, desktopBridge, listRecentProjects, openProject, removeRecentProject } from "../api.js";
import { DEVICES_MENU_ID, DevicesSlideover } from "../host/DevicesSlideover.js";
import { RemoteHostsSection } from "./RemoteHostsSection.js";
import type { RecentProjectView } from "../tauri-bridge/generated/bindings.js";
import { pickFolder } from "../tauri-bridge/nativeDialog.js";
import { useRowContextMenu } from "../components/useRowContextMenu.js";
import { formatRelativeSeconds } from "../components/format.js";
import { logUi } from "../logger.js";
import type { MenuItem } from "../menu.js";
import { openInForPath } from "../openIn.js";
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

  // Oxplow ▸ Devices…: pairing works with no project open.
  const [devicesOpen, setDevicesOpen] = useState(false);
  useEffect(() => {
    // The launcher's menu is the app menu alone; registering it makes this
    // window the one a menu item (Devices…) goes to while it is in front.
    void desktopBridge().setNativeMenu([]);
    return desktopBridge().onMenuCommand((id) => {
      if (id === DEVICES_MENU_ID) setDevicesOpen(true);
    });
  }, []);

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
      <DevicesSlideover open={devicesOpen} onClose={() => setDevicesOpen(false)} />
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

        <RemoteHostsSection />
      </div>
    </div>
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
