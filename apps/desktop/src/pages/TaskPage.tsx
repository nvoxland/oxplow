import { useEffect, useMemo, useState } from "react";
import type { EffortDetail, Stream, Thread, ThreadWorkState, Task, TaskPriority, TaskStatus } from "../api.js";
import {
  getTask,
  listTaskEfforts,
  moveBacklogItemToThread,
  moveTaskToBacklog,
  subscribeOxplowEvents,
  updateTask,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { gitCommitRef, snapshotRef, taskRef } from "../tabs/pageRefs.js";
import { ActivityTimeline, TaskDetail, TaskDetailRail } from "../components/Plan/TaskDetail.js";
import { CommentNavigator } from "../components/Comments/CommentNavigator.js";
import { BacklinksList, type SnapshotBacklinkEntry } from "../tabs/BacklinksList.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { logUi } from "../logger.js";

export interface TaskPageProps {
  stream: Stream | null;
  thread: Thread | null;
  itemId: string;
  /** Live snapshot of all tasks in the current thread (used to find this one). */
  items: Task[];
  threadWork: ThreadWorkState | null;
  /** Delete this task. The host handles confirmation fallout (closing /
   *  going back in the tab's history). */
  onDelete?(itemId: string): void;
  onOpenPage(ref: TabRef): void;
  onOpenFile?(path: string): void;
  onShowInHistory?(snapshotId: string): void;
  onOpenDiff?(spec: import("../components/Diff/DiffPane.js").DiffSpec): void;
}

/**
 * Single-record page for a task. Adopts `layout="details"`: title /
 * description / acceptance / activity live in the center column;
 * status / priority / category / tags / timestamps / overflow menu
 * (Send to backlog, Delete) live in the right rail. Activity timeline
 * sits below the editable body.
 */
export function TaskPage({
  stream,
  thread,
  itemId,
  items,
  onDelete,
  onOpenPage,
  onOpenFile,
  onShowInHistory,
  onOpenDiff,
}: TaskPageProps) {
  const [fetchedItem, setFetchedItem] = useState<Task | null>(null);
  const item = items.find((i) => i.id === itemId) ?? fetchedItem;
  const nav = useOptionalPageNavigation();
  const refForGraph = taskRef(itemId);
  const backlinkEntries = useBacklinks(refForGraph);
  const outboundEntries = usePageOutbound(refForGraph);
  const [efforts, setEfforts] = useState<EffortDetail[]>([]);
  const snapshotBacklinks = useMemo<SnapshotBacklinkEntry[]>(() => {
    return efforts
      .filter((d) => !!d.effort.end_snapshot_id)
      .map((d, i) => ({
        kind: "snapshot" as const,
        snapshotId: d.effort.end_snapshot_id!,
        label: `Effort ${i + 1} end snapshot`,
        source: "effort-end",
        snapshotLabel: null,
        subtitle: `${d.changed_paths.length} file${d.changed_paths.length === 1 ? "" : "s"}`,
      }));
  }, [efforts, itemId]);

  const backlinks = {
    count: backlinkEntries.length + snapshotBacklinks.length,
    body: (
      <BacklinksList
        entries={backlinkEntries}
        snapshotEntries={snapshotBacklinks}
        onOpenPage={onOpenPage}
        onOpenSnapshot={(payload) => {
          const id = Number(payload.snapshotId);
          if (Number.isFinite(id)) onOpenPage(snapshotRef(id));
        }}
        onOpenCommit={(payload) => onOpenPage(gitCommitRef(payload.sha))}
      />
    ),
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} />,
        }
      : undefined;

  const inThreadItems = items.some((i) => i.id === itemId);
  useEffect(() => {
    let cancelled = false;
    const refetch = () => {
      // Swallow + log rather than letting a rejected fetch (e.g. a
      // malformed task id) bubble to `window.unhandledrejection`, which
      // reads as a silent failure with no surfaced error.
      void getTask(itemId)
        .then((row) => {
          if (!cancelled) setFetchedItem(row);
        })
        .catch((err) => {
          logUi("warn", "task fetch failed", { itemId, error: String(err) });
        });
    };
    if (!inThreadItems) refetch();
    const unsub = subscribeOxplowEvents((event) => {
      // Oxplow events are `kind`-tagged (`{ kind: "tasksChanged", threadId }`);
      // there is no `type` field. Only the out-of-thread task needs this — the
      // in-thread case is driven by the live `items` prop.
      if (event.kind !== "tasksChanged") return;
      refetch();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [itemId, inThreadItems]);

  useEffect(() => {
    if (!item) return;
    let cancelled = false;
    void listTaskEfforts(item.id).then((rows) => {
      if (!cancelled) setEfforts(rows);
    });
    const unsub = subscribeOxplowEvents((event) => {
      // `tasksChanged` is thread-scoped and fires when an effort opens/closes
      // as part of a status transition — refetch this task's efforts so the
      // Activity timeline reflects the close without a remount.
      if (event.kind !== "tasksChanged") return;
      const threadId = (event as { threadId?: string | null }).threadId ?? null;
      if (item.thread_id != null && threadId !== item.thread_id) return;
      void listTaskEfforts(item.id).then((rows) => {
        if (!cancelled) setEfforts(rows);
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [item?.id]);

  const handleUpdate = async (
    targetId: string,
    changes: { title?: string; description?: string; status?: TaskStatus; priority?: TaskPriority; category?: string | null; tags?: string | null },
  ) => {
    if (!stream || !thread) return;
    await updateTask(stream.id, thread.id, targetId, changes);
  };

  const itemThreadId = item?.thread_id ?? null;
  const scopeAction: { label: string; run: () => Promise<void> } | null = (() => {
    if (!item || !stream) return null;
    if (itemThreadId === null && thread) {
      return {
        label: "Bring to this thread",
        run: async () => {
          await moveBacklogItemToThread(stream.id, item.id, thread.id);
        },
      };
    }
    if (thread && itemThreadId === thread.id) {
      return {
        label: "Send to backlog",
        run: async () => {
          await moveTaskToBacklog(stream.id, thread.id, item.id);
        },
      };
    }
    return null;
  })();

  if (!item) {
    return (
      <Page testId="page-tasks" title={`task:${itemId}`} kind="task" backlinks={backlinks} outbound={outbound}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
          Loading tasks…
        </div>
      </Page>
    );
  }

  const requestDelete = onDelete
    ? () => {
        if (window.confirm(`Delete task "${item.title}"? This can't be undone.`)) {
          onDelete(item.id);
        }
      }
    : undefined;
  const rail = (
    <TaskDetailRail
      item={item}
      onUpdateTask={handleUpdate}
      onDelete={requestDelete}
      scopeAction={scopeAction ? { label: scopeAction.label, run: () => void scopeAction.run() } : undefined}
    />
  );

  return (
    <Page
      testId="page-tasks"
      title={item.title}
      kind="task"
      backlinks={backlinks}
      outbound={outbound}
      commentsNav={stream ? <CommentNavigator targetKind="task" targetId={String(item.id)} /> : undefined}
      layout="details"
      rightRail={rail}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
        <TaskDetail
          item={item}
          onUpdateTask={handleUpdate}
          comments={
            stream
              ? {
                  streamId: stream.id,
                  threadId: item.thread_id ?? null,
                  targetKind: "task",
                  targetId: String(item.id),
                }
              : undefined
          }
        />
        <section>
          <h2 className="task-activity-heading">Activity</h2>
          <ActivityTimeline
            efforts={efforts}
            formatTimestamp={(iso) => new Date(iso).toLocaleString()}
            onOpenFile={onOpenFile}
            onShowInHistory={onShowInHistory}
            onOpenDiff={onOpenDiff}
          />
        </section>
      </div>
    </Page>
  );
}
