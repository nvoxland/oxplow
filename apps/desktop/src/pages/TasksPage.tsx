import { type ComponentProps, useCallback, useEffect, useMemo, useState } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { TasksList } from "../components/Plan/TasksList.js";
import { TaskDetailPane } from "../components/Plan/TaskDetailPane.js";
import { BacklogDrawer } from "../components/Plan/BacklogDrawer.js";
import { TasksScopePicker } from "../components/Plan/TasksScopePicker.js";
import {
  type TasksScope,
  loadScope,
  saveScope,
  mergeThreadWork,
  isReadOnlyScope,
} from "../components/Plan/tasks-scope.js";
import { cardLinkButton } from "../components/Card.js";
import { backlogRef, doneWorkRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import {
  getThreadWorkState,
  listThreads,
  type Stream,
  type Thread,
  type ThreadWorkState,
} from "../api.js";

export type TasksPageProps =
  Omit<
    ComponentProps<typeof PlanPane>,
    | "hideAuto"
    | "visibleSections"
    | "sectionItemLimit"
    | "sectionLabelOverrides"
    | "extraSectionLinks"
    | "hideBacklogChip"
    | "hideArchiveToggle"
    | "onlyStatuses"
    | "excludeStatuses"
  > & {
    onOpenPage(ref: TabRef): void;
    streams: Stream[];
    currentStreamId: string | null;
  };

const PREVIEW_LIMIT = 5;

const NOOP_ASYNC = async () => {};

/**
 * Tasks page with a scope switcher above the list. Scope modes:
 *   - currentThread: existing per-thread view (default).
 *   - thread: any (stream, thread) the user picks.
 *   - stream: every thread in a stream, merged.
 *   - all: every thread in every stream, merged.
 *
 * Cross-thread/stream views are read-only — mutation handlers in App.tsx
 * are scoped to the active (stream, thread), so the merged views replace
 * them with no-ops and hide the create/reorder affordances.
 */
export function TasksPage({
  onOpenPage,
  streams,
  currentStreamId,
  ...rest
}: TasksPageProps) {
  const [scope, setScopeState] = useState<TasksScope>(() => loadScope());
  const setScope = useCallback((next: TasksScope) => {
    setScopeState(next);
    saveScope(next);
  }, []);

  const [threadsByStream, setThreadsByStream] = useState<Record<string, Thread[]>>({});
  const [scopedWorkStates, setScopedWorkStates] = useState<Record<string, ThreadWorkState>>({});
  const [scopedError, setScopedError] = useState<string | null>(null);
  const [scopedLoading, setScopedLoading] = useState(false);

  const requestThreads = useCallback((streamId: string) => {
    if (!streamId) return;
    if (threadsByStream[streamId]) return;
    void listThreads(streamId)
      .then((ts) => setThreadsByStream((prev) => ({ ...prev, [streamId]: ts })))
      .catch((e) => setScopedError(String(e)));
  }, [threadsByStream]);

  // Pre-load thread lists used by the picker. The current stream is loaded
  // up-front so the "Specific thread" dropdown has options without a click.
  useEffect(() => {
    if (currentStreamId) requestThreads(currentStreamId);
  }, [currentStreamId, requestThreads]);

  // Fetch the work states needed by the active scope. For "stream" we load
  // every thread in that stream; for "all" we load every thread across
  // every stream. Results cache in scopedWorkStates by threadId.
  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        if (scope.kind === "currentThread") return;
        if (scope.kind === "thread") {
          if (!scope.streamId || !scope.threadId) return;
          if (scopedWorkStates[scope.threadId]) return;
          setScopedLoading(true);
          const work = await getThreadWorkState(scope.streamId, scope.threadId);
          if (cancelled) return;
          setScopedWorkStates((prev) => ({ ...prev, [scope.threadId]: work }));
        } else if (scope.kind === "stream") {
          if (!scope.streamId) return;
          let threads = threadsByStream[scope.streamId];
          if (!threads) {
            threads = await listThreads(scope.streamId);
            if (cancelled) return;
            setThreadsByStream((prev) => ({ ...prev, [scope.streamId]: threads! }));
          }
          setScopedLoading(true);
          const missing = threads.filter((t) => !scopedWorkStates[t.id]);
          const loaded = await Promise.all(
            missing.map(async (t) => [t.id, await getThreadWorkState(scope.streamId, t.id)] as const),
          );
          if (cancelled) return;
          if (loaded.length) {
            setScopedWorkStates((prev) => {
              const next = { ...prev };
              for (const [id, state] of loaded) next[id] = state;
              return next;
            });
          }
        } else if (scope.kind === "all") {
          setScopedLoading(true);
          const allThreadLists = await Promise.all(
            streams.map(async (s) => {
              if (threadsByStream[s.id]) return [s.id, threadsByStream[s.id]!] as const;
              const ts = await listThreads(s.id);
              return [s.id, ts] as const;
            }),
          );
          if (cancelled) return;
          setThreadsByStream((prev) => {
            const next = { ...prev };
            for (const [id, ts] of allThreadLists) next[id] = ts;
            return next;
          });
          const all = allThreadLists.flatMap(([streamId, ts]) =>
            ts.map((t) => ({ streamId, threadId: t.id })),
          );
          const missing = all.filter((p) => !scopedWorkStates[p.threadId]);
          const loaded = await Promise.all(
            missing.map(async (p) => [p.threadId, await getThreadWorkState(p.streamId, p.threadId)] as const),
          );
          if (cancelled) return;
          if (loaded.length) {
            setScopedWorkStates((prev) => {
              const next = { ...prev };
              for (const [id, state] of loaded) next[id] = state;
              return next;
            });
          }
        }
        if (!cancelled) setScopedError(null);
      } catch (e) {
        if (!cancelled) setScopedError(String(e));
      } finally {
        if (!cancelled) setScopedLoading(false);
      }
    }
    void load();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, streams]);

  const effectiveThreadWork: ThreadWorkState | null = useMemo(() => {
    if (scope.kind === "currentThread") return rest.threadWork ?? null;
    if (scope.kind === "thread") {
      return scopedWorkStates[scope.threadId] ?? null;
    }
    if (scope.kind === "stream") {
      const threads = threadsByStream[scope.streamId] ?? [];
      const states = threads
        .map((t) => scopedWorkStates[t.id])
        .filter((s): s is ThreadWorkState => Boolean(s));
      return mergeThreadWork(states);
    }
    if (scope.kind === "all") {
      const allIds = Object.values(threadsByStream).flat().map((t) => t.id);
      const states = allIds
        .map((id) => scopedWorkStates[id])
        .filter((s): s is ThreadWorkState => Boolean(s));
      return mergeThreadWork(states);
    }
    return null;
  }, [scope, rest.threadWork, scopedWorkStates, threadsByStream]);

  const readOnly = isReadOnlyScope(scope) || (scope.kind === "thread" && scope.threadId !== rest.activeThreadId);

  const viewAllDone = (
    <button
      type="button"
      data-testid="tasks-view-done"
      onClick={(event) => { event.stopPropagation(); onOpenPage(doneWorkRef()); }}
      style={cardLinkButton}
    >
      View all done →
    </button>
  );

  // When read-only (cross-thread/stream/all view), neutralize mutation
  // handlers — the existing wiring is bound to the active thread, and
  // calling it for items belonging to another thread would write to the
  // wrong place. The picker UI keeps the rest of the view interactive
  // (sorting, filters, expanding epics).
  const listProps = readOnly
    ? {
        ...rest,
        threadWork: effectiveThreadWork,
        onUpdateTask: NOOP_ASYNC as typeof rest.onUpdateTask,
        onDeleteTask: NOOP_ASYNC as typeof rest.onDeleteTask,
        onReorderTasks: NOOP_ASYNC as typeof rest.onReorderTasks,
        onMoveItemToBacklog: NOOP_ASYNC as typeof rest.onMoveItemToBacklog,
      }
    : { ...rest, threadWork: effectiveThreadWork };

  return (
    <Page testId="page-tasks" title="Tasks">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <TasksScopePicker
          scope={scope}
          onChange={setScope}
          streams={streams}
          threadsByStream={threadsByStream}
          onRequestThreads={requestThreads}
        />
        {scopedError ? (
          <div data-testid="tasks-scope-error" style={{ padding: "4px 10px", color: "var(--danger)", fontSize: "var(--text-xs)" }}>
            {scopedError}
          </div>
        ) : null}
        {scopedLoading && scope.kind !== "currentThread" ? (
          <div style={{ padding: "4px 10px", color: "var(--muted)", fontSize: "var(--text-xs)" }}>Loading…</div>
        ) : null}
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "row" }}>
          <TasksList
            {...listProps}
            visibleSections={["ready", "blocked", "done"]}
            sectionItemLimit={{ done: PREVIEW_LIMIT }}
            sectionLabelOverrides={{ done: "Recently Done" }}
            extraSectionLinks={{ done: viewAllDone }}
            hideBacklogChip
            hideArchiveToggle
          />
          <TaskDetailPane threadWork={effectiveThreadWork} />
        </div>
        <BacklogDrawer
          backlog={rest.backlog}
          onOpenBacklog={() => onOpenPage(backlogRef())}
        />
      </div>
    </Page>
  );
}
