import { ResourcePage } from "./ResourcePage.js";
import { diffViewOf, isPage, newTaskRef, viewParamsOf, workItemTabRef, type DiffViewPayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../api.js";
import { openExternalUrl } from "../api.js";
import type { CenterTab } from "../components/CenterTabs/CenterTabs.js";
import type { BuiltInPageName } from "./declaredPages.js";
import type { AppScope, SlotNav } from "./appScope.js";
import { SettingsPage } from "./SettingsPage.js";
import { settingsArea } from "./settingsAreas.js";
import { DiffViewPage } from "./DiffViewPage.js";
import { GitDashboardPage } from "./GitDashboardPage.js";
import { TerminalPage } from "./TerminalPage.js";
import { FilesPage } from "./FilesPage.js";
import { TasksPage } from "./TasksPage.js";
import { BacklogPage } from "./BacklogPage.js";
import { MetricsPage } from "./MetricsPage.js";
import { CustomDashboardPage } from "./CustomDashboardPage.js";
import { ExploreDataPage } from "./ExploreDataPage.js";
import { BoardPage } from "./BoardPage.js";
import { ExternalUrlPage } from "./ExternalUrlPage.js";

/** What a built-in page body gets: its tab ref and the ref's payload, the slot's navigation, and core's scope. */
export interface BuiltInPageContext {
  ref: TabRef;
  /** The id's params (`pagePayload(ref).params`). */
  params: Record<string, string>;
  nav: SlotNav;
  scope: AppScope;
}

export type BuiltInPage = (ctx: BuiltInPageContext) => CenterTab | null;

const workPage: BuiltInPage = ({ ref, nav, scope }) => {
  const sharedProps = {
    thread: scope.thread,
    threadWork: scope.threadWork,
    agentStatus: scope.agentStatus,
    backlog: scope.backlog,
    profile: scope.workListProfile,
    fields: scope.workListProfile.fields,
    onUpdateTask: scope.handleUpdateTask,
    onDeleteTask: scope.handleDeleteTask,
    onReorderTasks: scope.handleReorderTasks,
    onReorderBacklog: scope.handleReorderBacklog,
    registerOpenCreate: (fn: () => void) => { scope.planOpenCreateRef.current = fn; },
    onOpenNewTaskPage: (payload: { parentRef?: string | null }) =>
      nav.navOpen(newTaskRef(payload)),
    onOpenTaskPage: (ref: string) => nav.navOpen(workItemTabRef(ref)),
  };
  const tasks = isPage(ref, "tasks");
  return {
    id: ref.id,
    label: tasks ? "Tasks" : "Backlog",
    closable: true,
    render: () =>
      tasks ? (
        <TasksPage {...sharedProps} streams={scope.streams} currentStreamId={scope.stream?.id ?? null} onOpenPage={nav.navOpen} />
      ) : (
        <BacklogPage {...sharedProps} />
      ),
  };
};

// A snapshot, an effort, an agent turn (entity pages, P2.11) and an
// ad-hoc endpoints pair (`diff-view` route) all render as a diff.
export const diffViewTab = (ref: TabRef, payload: DiffViewPayload | null, nav: SlotNav, stream: Stream | null): CenterTab | null => {
  if (!payload) return null;
  const label =
    payload.mode === "snapshot"
      ? `Snapshot ${payload.snapshotId}`
      : payload.mode === "effort"
        ? `Effort ${payload.effortId}`
        : payload.mode === "turn"
          ? `Turn ${payload.turnId}`
          : "Diff";
  return {
    id: ref.id,
    label,
    closable: true,
    render: () => (
      <DiffViewPage
        stream={stream}
        spec={payload}
        onOpenDiff={nav.navOpenDiff}
        onOpenDiffInTab={nav.navOpenDiff}
        onOpenPage={nav.navOpen}
        onOpenFile={nav.navOpenFile}
      />
    ),
  };
};

export const builtInPages: Record<BuiltInPageName, BuiltInPage> = {
  settings: ({ ref, nav }) => {
    const view = viewParamsOf(ref);
    const area = settingsArea(typeof view.area === "string" ? view.area : undefined);
    const section = typeof view.section === "string" ? view.section : null;
    return {
      id: ref.id,
      label: area.id === "overview" ? "Settings" : `Settings: ${area.title}`,
      closable: true,
      render: () => <SettingsPage area={area.id} section={section} onOpenPage={nav.navOpen} />,
    };
  },
  "diff-view": ({ ref, nav, scope }) => diffViewTab(ref, diffViewOf(ref), nav, scope.stream),
  // What a URI no page renders opens on.
  resource: ({ ref }) => ({
    id: ref.id,
    label: "",
    closable: true,
    render: () => <ResourcePage uri={ref.id} />,
  }),
  "git-dashboard": ({ ref, nav, scope }) => {
    return {
      id: ref.id,
      label: "Git Dashboard",
      closable: true,
      render: () => (
        <GitDashboardPage
          stream={scope.stream}
          onOpenPage={nav.navOpen}
          onRevealCommit={nav.navRevealCommit}
        />
      ),
    };
  },
  terminal: ({ ref, scope }) => {
    return {
      id: ref.id,
      label: "Terminal",
      closable: true,
      render: () => (
        <TerminalPage
          stream={scope.stream}
          visible={scope.effectiveCenterActive === ref.id}
          onOpenFile={(absPath, line, column) => {
            if (!scope.stream) return;
            const wt = scope.stream.worktree_path.endsWith("/")
              ? scope.stream.worktree_path.slice(0, -1)
              : scope.stream.worktree_path;
            const rel = absPath.startsWith(wt + "/")
              ? absPath.slice(wt.length + 1)
              : absPath;
            if (typeof line === "number" && line > 0) {
              void scope.handleNavigateToLocation({ path: rel, line, column: column ?? 1 });
            } else {
              void scope.handleOpenFile(rel);
            }
          }}
        />
      ),
    };
  },
  files: ({ ref, nav, scope }) => {
    return {
      id: ref.id,
      label: "Files",
      closable: true,
      render: () => (
        <FilesPage
          stream={scope.stream}
          vcsEnabled={scope.vcsEnabled}
          selectedFilePath={scope.selectedFilePath}
          generated={scope.generatedExclude}
          onOpenFile={nav.navOpenFile}
          onOpenDiff={nav.navOpenDiff}
          onCreateFile={scope.handleCreateFile}
          onCreateDirectory={scope.handleCreateDirectory}
          onRenamePath={scope.handleRenamePath}
          onDeletePath={scope.handleDeletePath}
          onToggleGenerated={scope.handleToggleGenerated}
          commitRequest={scope.commitFilesRequest}
        />
      ),
    };
  },
  "metrics-recorded": ({ ref, nav }) => {
    return {
      id: ref.id,
      label: "Metrics",
      closable: true,
      render: () => <MetricsPage onOpenPage={nav.navOpen} />,
    };
  },
  "custom-dashboard": ({ ref, params, nav }) => {
    // `customDashboardRef(id)` — one user-created dashboard (grid of tiles).
    return {
      id: ref.id,
      label: "Dashboard",
      closable: true,
      render: () => <CustomDashboardPage dashboardId={params.id} onOpenPage={nav.navOpen} />,
    };
  },
  board: ({ ref, nav, scope }) => ({
    id: ref.id,
    label: "Board",
    closable: true,
    render: () => (
      <BoardPage threadId={scope.threadId ?? null} streamId={scope.stream?.id ?? null} onOpenPage={nav.navOpen} />
    ),
  }),
  "explore-data": ({ ref, nav, scope }) => {
    return {
      id: ref.id,
      label: "Explore Data",
      closable: true,
      render: () => <ExploreDataPage stream={scope.stream} onOpenPage={nav.navOpen} />,
    };
  },
  tasks: workPage,
  backlog: workPage,
  // A web page: its tab's id is its URL.
  "external-url": ({ ref }) => {
    const externalUrl = ref.id;
    let label = externalUrl;
    try {
      const u = new URL(externalUrl);
      label = u.host + (u.pathname && u.pathname !== "/" ? u.pathname : "");
    } catch { /* keep raw */ }
    return {
      id: ref.id,
      label: label.length > 40 ? label.slice(0, 40) + "…" : label,
      closable: true,
      contextMenu: [
        {
          id: "external-url.open-in-browser",
          label: "Open in Browser",
          enabled: true,
          run: () => { void openExternalUrl(externalUrl); },
        },
        {
          id: "external-url.copy",
          label: "Copy URL",
          enabled: true,
          run: () => { void navigator.clipboard.writeText(externalUrl).catch(() => {}); },
        },
      ],
      render: () => (
        <ExternalUrlPage
          url={externalUrl}
          onOpenInBrowser={(u) => { void openExternalUrl(u); }}
        />
      ),
    };
  },
};
