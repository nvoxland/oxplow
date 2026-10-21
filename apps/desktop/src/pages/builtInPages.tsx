import { agentSessionRef, diffSpecOf, duplicateBlockOf, endpointsOf, fileRef, isPage, newTaskRef, workItemTabRef, type DiffViewPayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { idOf } from "../refs/ref.js";
import type { Stream } from "../api.js";
import { openAgentSession, openExternalUrl } from "../api.js";
import { saveNewThreadSession } from "../newThreadSession.js";
import { personCommands } from "../personCommands.js";
import type { CenterTab } from "../components/CenterTabs/CenterTabs.js";
import type { BuiltInPageName } from "./declaredPages.js";
import type { AppScope, SlotNav } from "./appScope.js";
import { DiffPage } from "./DiffPage.js";
import { DuplicateBlockPage } from "./DuplicateBlockPage.js";
import { SettingsPage } from "./SettingsPage.js";
import { settingsArea } from "./settingsAreas.js";
import { DiffViewPage } from "./DiffViewPage.js";
import { GitDashboardPage } from "./GitDashboardPage.js";
import { UncommittedChangesPage } from "./UncommittedChangesPage.js";
import { NewSessionPage } from "./NewSessionPage.js";
import { TerminalPage } from "./TerminalPage.js";
import { FilesPage } from "./FilesPage.js";
import { TasksPage } from "./TasksPage.js";
import { BacklogPage } from "./BacklogPage.js";
import { CommentsInboxPage } from "./CommentsInboxPage.js";
import { MetricsPage } from "./MetricsPage.js";
import { CustomDashboardPage } from "./CustomDashboardPage.js";
import { ExploreDataPage } from "./ExploreDataPage.js";
import { BoardPage } from "./BoardPage.js";
import { ExternalUrlPage } from "./ExternalUrlPage.js";
import { StreamSettingsPage } from "./StreamSettingsPage.js";
import { ThreadSettingsPage } from "./ThreadSettingsPage.js";
import { NewStreamPage } from "./NewStreamPage.js";
import { NewTaskPage } from "./NewTaskPage.js";
import { AlertsPage } from "./AlertsPage.js";

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
  "new-session": ({ ref, scope }) => ({
    id: ref.id,
    label: "New session",
    closable: true,
    render: () => (
      <NewSessionPage
        thread={scope.thread}
        harnesses={scope.harnesses}
        streamId={scope.stream?.id ?? null}
        onOpenPage={scope.handleOpenPage}
        onStart={async (harness, remember) => {
          if (!scope.thread) return;
          if (remember) await saveNewThreadSession(harness);
          const opened = await openAgentSession(scope.thread.id, harness);
          // The picker gives way to the session's tab (it follows the
          // row once the model re-reads).
          scope.closePageTab(ref.id);
          scope.setCenterActive(agentSessionRef(opened.id).id);
        }}
        onTakeEarlier={async (row) => {
          if (!scope.thread) return;
          const sessionId = row.session ? idOf(row.session, "agent_session") : null;
          if (row.action === "switch" && sessionId) {
            // Already open: show it, and keep the picker for another pick.
            scope.setCenterActive(agentSessionRef(sessionId).id);
            return;
          }
          let tab: string | null = null;
          if (row.action === "reopen" && sessionId) {
            // Undoable: the toast offers Undo, which closes it again.
            const reopened = await personCommands.run(`Reopen ${row.label}`, "oxplow.agent_session.reopen", {
              session: row.session,
            });
            if (!reopened) return;
            tab = agentSessionRef(sessionId).id;
          } else if (row.action === "resume" && row.conversation) {
            const opened = await openAgentSession(scope.thread.id, row.harness, undefined, row.conversation);
            tab = agentSessionRef(opened.id).id;
          }
          if (!tab) return;
          // The picker gives way to the session's tab.
          scope.closePageTab(ref.id);
          scope.setCenterActive(tab);
        }}
      />
    ),
  }),
  diff: ({ ref, params, nav, scope }) => {
    const spec = diffSpecOf(params);
    if (!spec) return null;
    const label = spec.path.split("/").pop() ?? spec.path;
    const suffix = spec.labelOverride ?? "diff";
    return {
      id: ref.id,
      label: `${label} (${suffix})`,
      closable: true,
      render: () => scope.stream ? (
        <DiffPage
          stream={scope.stream}
          spec={spec}
          visible={scope.effectiveCenterActive === ref.id}
          onJumpToSource={(p) => {
            // In-tab navigation: replace the slot's diff with
            // the file. Back returns to the diff. Browser-tab
            // semantics; do NOT close the diff manually here —
            // scope.handleNavigateInTab takes care of swapping the
            // slot's ref while keeping the diff in the back stack.
            nav.navOpenFile(p);
          }}
        />
      ) : null,
    };
  },
  "duplicate-block": ({ ref, params, nav, scope }) => {
    const payload = duplicateBlockOf(params);
    if (!payload) return null;
    const leftBase = payload.leftPath.split("/").pop() ?? payload.leftPath;
    const rightBase = payload.rightPath.split("/").pop() ?? payload.rightPath;
    return {
      id: ref.id,
      label: `${leftBase} ↔ ${rightBase}`,
      closable: true,
      render: () => scope.stream ? (
        <DuplicateBlockPage
          stream={scope.stream}
          payload={payload}
          visible={scope.effectiveCenterActive === ref.id}
          onJumpToSource={(p, v) => {
            scope.handleNavigateInTab(nav.slotId, fileRef(p, v));
          }}
        />
      ) : null,
    };
  },
  settings: ({ ref, params, nav }) => {
    const area = settingsArea(params.area);
    const section = params.section ?? null;
    return {
      id: ref.id,
      label: area.id === "overview" ? "Settings" : `Settings: ${area.title}`,
      closable: true,
      render: () => <SettingsPage area={area.id} section={section} onOpenPage={nav.navOpen} />,
    };
  },
  "diff-view": ({ ref, params, nav, scope }) => diffViewTab(ref, endpointsOf(params), nav, scope.stream),
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
  "uncommitted-changes": ({ ref, nav, scope }) => {
    return {
      id: ref.id,
      label: "Uncommitted",
      closable: true,
      render: () => (
        <UncommittedChangesPage
          stream={scope.stream}
          onOpenPage={nav.navOpen}
          onOpenFile={nav.navOpenFile}
          onOpenDiff={nav.navOpenDiff}
          onOpenDiffInTab={nav.navOpenDiff}
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
  alerts: ({ ref, nav }) => {
    return {
      id: ref.id,
      label: "Alerts",
      closable: true,
      render: () => <AlertsPage onOpenPage={nav.navOpen} />,
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
  comments: ({ ref, nav, scope }) => {
    return {
      id: ref.id,
      label: "Comments Dashboard",
      closable: true,
      render: () => <CommentsInboxPage stream={scope.stream} onOpenPage={nav.navOpen} />,
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
  "external-url": ({ ref, params }) => {
    const externalUrl = params.url ?? "";
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
  "stream-settings": ({ ref, params, scope }) => {
    const targetStreamId = params.stream ?? "";
    const targetStream = scope.streams.find((s) => s.id === targetStreamId) ?? null;
    return {
      id: ref.id,
      label: targetStream ? `Settings · ${targetStream.title}` : "Stream Settings",
      closable: true,
      render: () => (
        <StreamSettingsPage
          stream={targetStream}
          onClose={() => scope.closePageTab(ref.id)}
          onSaved={(next) => scope.setStreams(next)}
        />
      ),
    };
  },
  "thread-settings": ({ ref, params, nav, scope }) => {
    const targetThreadId = params.thread ?? "";
    const targetThread = scope.threads.find((t) => t.id === targetThreadId) ?? null;
    return {
      id: ref.id,
      label: targetThread ? `Settings · ${targetThread.title}` : "Thread Settings",
      closable: true,
      render: () => (
        <ThreadSettingsPage
          streamId={scope.stream?.id ?? ""}
          thread={targetThread}
          onClose={() => scope.closePageTab(ref.id)}
          onSaved={(nextThreads) => {
            if (!scope.stream) return;
            scope.setThreadStates((prev) => ({
              ...prev,
              [scope.stream.id]: {
                ...(prev[scope.stream.id] ?? { selectedThreadId: null, threads: [] }),
                threads: nextThreads,
              },
            }));
          }}
        />
      ),
    };
  },
  "new-stream": ({ ref, scope }) => {
    return {
      id: ref.id,
      label: "New Stream",
      closable: true,
      render: () => (
        <NewStreamPage
          vcsEnabled={scope.vcsEnabled}
          defaultTitle={`Stream ${scope.streams.length + 1}`}
          onClose={() => scope.closePageTab(ref.id)}
          onCreated={(created) => {
            scope.handleStreamCreated(created);
            scope.closePageTab(ref.id);
          }}
        />
      ),
    };
  },
  "new-task": ({ ref, params, scope }) => {
    return {
      id: ref.id,
      label: "New item",
      closable: true,
      render: () => (
        <NewTaskPage
          defaults={{ parentRef: params.parent ?? null }}
          epics={scope.threadWork?.epics ?? []}
          onClose={() => scope.closePageTab(ref.id)}
          onSubmit={(input) => scope.handleCreateTask({ ...input, state: input.state ?? "todo" })}
        />
      ),
    };
  },
};
