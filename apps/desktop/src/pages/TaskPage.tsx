import { LensSlots } from "../lens/LensSlots.js";
import { workItemRef } from "../workItemRef.js";
import { numericRowId } from "../lens/lensModel.js";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { EffortDetail, Stream, Thread, ThreadWorkState, Task, TaskPriority, TaskStatus } from "../api.js";
import { moveTask, readTask, readTaskEfforts, updateTask } from "../workItems.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
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
  /** Open the diff view for a completed effort (start→end snapshots). */
  onShowEffortDiff?(effortId: string): void;
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
  onShowEffortDiff,
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
  // What the page's own reads read; a change to one of those models
  // re-runs them (the one rerun rule). The in-thread task itself is
  // driven by the live `items` prop.
  const [taskReads, setTaskReads] = useState<Reads>(NO_READS);
  const [effortReads, setEffortReads] = useState<Reads>(NO_READS);
  const refetchTask = useCallback(() => {
    if (inThreadItems) return;
    // Swallow + log rather than letting a rejected fetch (e.g. a
    // malformed task id) bubble to `window.unhandledrejection`, which
    // reads as a silent failure with no surfaced error.
    void readTask(itemId)
      .then(({ task, reads }) => {
        setFetchedItem(task);
        setTaskReads(reads);
      })
      .catch((err) => {
        logUi("warn", "task fetch failed", { itemId, error: String(err) });
      });
  }, [itemId, inThreadItems]);
  useEffect(() => refetchTask(), [refetchTask]);

  const effortTaskId = item?.id ?? null;
  const loadEfforts = useCallback(() => {
    if (!effortTaskId) return;
    void readTaskEfforts(effortTaskId)
      .then(({ efforts, reads }) => {
        setEfforts(efforts);
        setEffortReads(reads);
      })
      .catch((err) => logUi("warn", "task efforts fetch failed", { itemId: effortTaskId, error: String(err) }));
  }, [effortTaskId]);
  useEffect(() => loadEfforts(), [loadEfforts]);
  // An effort opens, closes or links as the effort policy reacts to a
  // status move: the Activity timeline re-reads without a remount.
  useRerunOnChange(unionReads([taskReads, effortReads]), () => {
    refetchTask();
    loadEfforts();
  });

  const handleUpdate = async (
    targetId: string,
    changes: { title?: string; description?: string; status?: TaskStatus; priority?: TaskPriority; category?: string | null; tags?: string | null },
  ) => {
    await updateTask(targetId, changes);
  };

  const itemThreadId = item?.thread_id ?? null;
  const scopeAction: { label: string; run: () => Promise<void> } | null = (() => {
    if (!item || !stream) return null;
    if (itemThreadId === null && thread) {
      return {
        label: "Bring to this thread",
        run: async () => {
          await moveTask(item.id, thread.id);
        },
      };
    }
    if (thread && itemThreadId === thread.id) {
      return {
        label: "Send to backlog",
        run: async () => {
          await moveTask(item.id, null);
        },
      };
    }
    return null;
  })();

  if (!item) {
    return (
      <Page testId="page-tasks" title={itemId} kind="work_item" backlinks={backlinks} outbound={outbound}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
          Loading tasks…
        </div>
      </Page>
    );
  }

  // The rail's Delete asks inline before it calls this.
  const requestDelete = onDelete ? () => onDelete(item.id) : undefined;
  // What the item's two `work_item.detail.*` slots bind.
  const taskRow = numericRowId(String(item.id));
  // The newest effort (they come newest first) is the one to review.
  const latestEffort = efforts[0]?.effort.id;
  const review = onShowEffortDiff && latestEffort ? () => onShowEffortDiff(latestEffort) : undefined;
  const slotParams = taskRow === null ? null : { ref: workItemRef(String(item.id)) };
  const rail = (
    <>
      <TaskDetailRail
        item={item}
        onUpdateTask={handleUpdate}
        onDelete={requestDelete}
        onReview={review}
        scopeAction={scopeAction ? { label: scopeAction.label, run: () => void scopeAction.run() } : undefined}
      />
      <LensSlots
        slot="work_item.detail.sidebar"
        params={slotParams}
        streamId={stream?.id ?? null}
        onOpenPage={onOpenPage}
      />
    </>
  );

  return (
    <Page
      testId="page-tasks"
      title={item.title}
      kind="work_item"
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
        <LensSlots
          slot="work_item.detail.body"
          params={slotParams}
          streamId={stream?.id ?? null}
          onOpenPage={onOpenPage}
          h2ClassName="task-activity-heading"
        />
        <section>
          <h2 className="task-activity-heading">Activity</h2>
          <ActivityTimeline
            efforts={efforts}
            formatTimestamp={(iso) => new Date(iso).toLocaleString()}
            onOpenFile={onOpenFile}
            onShowEffortDiff={onShowEffortDiff}
            onOpenDiff={onOpenDiff}
          />
        </section>
      </div>
    </Page>
  );
}
