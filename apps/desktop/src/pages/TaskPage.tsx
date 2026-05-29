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
import { useProseAudience } from "../tabs/useProseAudience.js";
import { availableVariants, type ProseVariants } from "../components/ProseAudience/selectVariant.js";
import type { ProseAudience } from "../tabs/proseAudience.js";

export interface TaskPageProps {
  stream: Stream | null;
  thread: Thread | null;
  itemId: number;
  /** Live snapshot of all tasks in the current thread (used to find this one). */
  items: Task[];
  threadWork: ThreadWorkState | null;
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
  onOpenPage,
  onOpenFile,
  onShowInHistory,
  onOpenDiff,
}: TaskPageProps) {
  const [fetchedItem, setFetchedItem] = useState<Task | null>(null);
  const item = items.find((i) => i.id === itemId) ?? fetchedItem;
  const nav = useOptionalPageNavigation();
  const { audience, setAudience } = useProseAudience(nav?.pageKey ?? null);
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
        tasksId: itemId,
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
      void getTask(itemId).then((row) => {
        if (!cancelled) setFetchedItem(row);
      });
    };
    if (!inThreadItems) refetch();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.type !== "task.changed") return;
      const targetId = (event as unknown as { itemId?: number }).itemId;
      if (targetId !== itemId) return;
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
      if (event.type !== "task.changed") return;
      const targetId = (event as unknown as { itemId?: number }).itemId;
      if (targetId !== item.id) return;
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
    targetId: number,
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

  // Availability spans the description AND every effort summary, since
  // one page-level selector governs all the page's prose.
  const descVariants: ProseVariants = item.description_variants ?? { developer: item.description };
  const available = availableVariants(descVariants);
  for (const e of efforts) {
    const sv = e.effort.summary_variants;
    if (sv?.executive) available.executive = true;
    if (sv?.terse) available.terse = true;
  }
  const audienceConfig: { value: ProseAudience; onChange: (a: ProseAudience) => void; available: Record<ProseAudience, boolean> } = {
    value: audience,
    onChange: setAudience,
    available,
  };

  const rail = (
    <TaskDetailRail
      item={item}
      onUpdateTask={handleUpdate}
      onRequestDelete={() => {}}
      extraMenuItems={
        scopeAction ? [{ label: scopeAction.label, onSelect: () => void scopeAction.run() }] : undefined
      }
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
      audience={audienceConfig}
      layout="details"
      rightRail={rail}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
        <TaskDetail
          item={item}
          onUpdateTask={handleUpdate}
          audience={audience}
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
            audience={audience}
          />
        </section>
      </div>
    </Page>
  );
}
