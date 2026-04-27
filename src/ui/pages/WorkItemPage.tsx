import { useEffect, useMemo, useState } from "react";
import type { EffortDetail, Stream, Thread, ThreadWorkState, WorkItem, WorkItemPriority, WorkItemStatus, WorkNote } from "../api.js";
import {
  getWorkNotes,
  listWorkItemEfforts,
  subscribeOxplowEvents,
  updateWorkItem,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { workItemRef } from "../tabs/pageRefs.js";
import { ActivityTimeline, WorkItemDetail } from "../components/Plan/WorkItemDetail.js";
import { BacklinksList, type SnapshotBacklinkEntry } from "../tabs/BacklinksList.js";
import { useBacklinks } from "../tabs/useBacklinks.js";
import { SnapshotDetailSlideover } from "../components/Snapshots/SnapshotDetailSlideover.js";
import { CommitDetailSlideover } from "../components/History/CommitDetailSlideover.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { DiffRequest } from "../components/Diff/diff-request.js";

export interface WorkItemPageProps {
  stream: Stream | null;
  thread: Thread | null;
  itemId: string;
  /** Live snapshot of all work items in the current thread (used to find this one). */
  items: WorkItem[];
  threadWork: ThreadWorkState | null;
  onOpenPage(ref: TabRef): void;
  onOpenFile?(path: string): void;
  onShowInHistory?(snapshotId: string): void;
  /** Forwarded to the embedded SnapshotDetailSlideover so its file rows
   *  can ask the host to open a diff editor. */
  onOpenDiff?(spec: DiffSpec): void;
  /** Forwarded to the embedded CommitDetailSlideover so its file rows
   *  can route diffs into the host's diff editor. */
  onOpenCommitDiff?(request: DiffRequest): void;
}

/**
 * Single-record page for a work item. Shows the full editable detail
 * (title, description, acceptance, status, priority) plus the merged
 * activity timeline (notes + efforts). Phase 4 entry point — replaces
 * the modal-only edit flow when callers route via `onOpenPage`.
 *
 * Read-only fallback: if the item isn't in the loaded thread state
 * (e.g. it lives in another thread), the page renders just the title
 * row and a hint to open it from its owning thread.
 */
export function WorkItemPage({
  stream,
  thread,
  itemId,
  items,
  threadWork,
  onOpenPage,
  onOpenFile,
  onShowInHistory,
  onOpenDiff,
  onOpenCommitDiff,
}: WorkItemPageProps) {
  const item = items.find((i) => i.id === itemId) ?? null;
  const backlinkEntries = useBacklinks(workItemRef(itemId), stream, threadWork);
  const [notes, setNotes] = useState<WorkNote[]>([]);
  const [efforts, setEfforts] = useState<EffortDetail[]>([]);
  // Slideover state lives on this host page (the brief calls for it).
  // Single instance — opening another snapshot replaces the current one.
  const [slideoverSnapshot, setSlideoverSnapshot] = useState<{
    snapshotId: string;
    label: string | null;
    source: string;
    workItemId: string | null;
  } | null>(null);
  const [slideoverCommit, setSlideoverCommit] = useState<{
    sha: string;
    subject: string;
  } | null>(null);

  // Synthesize snapshot backlinks from this item's efforts. Each completed
  // effort's `end_snapshot_id` becomes a clickable row that opens the
  // SnapshotDetailSlideover. Skipped when no end snapshot (effort still
  // in progress) so the row never lands without a target.
  const snapshotBacklinks = useMemo<SnapshotBacklinkEntry[]>(() => {
    return efforts
      .filter((d) => !!d.effort.end_snapshot_id)
      .map((d, i) => ({
        kind: "snapshot" as const,
        snapshotId: d.effort.end_snapshot_id!,
        label: `Effort ${i + 1} end snapshot`,
        source: "task-end",
        snapshotLabel: null,
        workItemId: itemId,
        subtitle: `${d.changed_paths.length} file${d.changed_paths.length === 1 ? "" : "s"}`,
      }));
  }, [efforts, itemId]);

  const backlinks = (
    <BacklinksList
      entries={backlinkEntries}
      snapshotEntries={snapshotBacklinks}
      onOpenPage={onOpenPage}
      onOpenSnapshot={(payload) => setSlideoverSnapshot({
        snapshotId: payload.snapshotId,
        label: payload.label ?? null,
        source: payload.source ?? "",
        workItemId: payload.workItemId ?? null,
      })}
      onOpenCommit={(payload) => setSlideoverCommit({
        sha: payload.sha,
        subject: payload.subject ?? "",
      })}
    />
  );

  useEffect(() => {
    if (!item) return;
    let cancelled = false;
    void getWorkNotes(item.id).then((rows) => {
      if (!cancelled) setNotes(rows);
    });
    void listWorkItemEfforts(item.id).then((rows) => {
      if (!cancelled) setEfforts(rows);
    });
    const unsub = subscribeOxplowEvents((event) => {
      if (event.type !== "work-item.changed") return;
      const targetId = (event as unknown as { itemId?: string }).itemId;
      if (targetId !== item.id) return;
      void getWorkNotes(item.id).then((rows) => {
        if (!cancelled) setNotes(rows);
      });
      void listWorkItemEfforts(item.id).then((rows) => {
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
    changes: { title?: string; description?: string; acceptanceCriteria?: string | null; status?: WorkItemStatus; priority?: WorkItemPriority },
  ) => {
    if (!stream || !thread) return;
    await updateWorkItem(stream.id, thread.id, targetId, changes);
  };

  const slideover = (
    <>
      <SnapshotDetailSlideover
        open={!!slideoverSnapshot}
        onClose={() => setSlideoverSnapshot(null)}
        stream={stream}
        snapshotId={slideoverSnapshot?.snapshotId ?? null}
        snapshotLabel={slideoverSnapshot?.label ?? null}
        snapshotSource={slideoverSnapshot?.source ?? ""}
        workItemId={slideoverSnapshot?.workItemId ?? null}
        onOpenDiff={onOpenDiff}
        onOpenWorkItem={(targetId) => onOpenPage(workItemRef(targetId))}
      />
      <CommitDetailSlideover
        open={!!slideoverCommit}
        onClose={() => setSlideoverCommit(null)}
        stream={stream}
        sha={slideoverCommit?.sha ?? null}
        subject={slideoverCommit?.subject ?? ""}
        onOpenDiff={onOpenCommitDiff}
      />
    </>
  );

  if (!item) {
    return (
      <Page testId="page-work-item" title={itemId} kind="work item" backlinks={backlinks}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: 13 }}>
          This work item is not loaded in the current thread. Open the thread that owns it to edit, or use the rail to navigate.
        </div>
        {slideover}
      </Page>
    );
  }

  const chips = [
    { label: item.status },
    { label: `${item.priority} priority` },
  ];

  return (
    <Page testId="page-work-item" title={item.title} kind="work item" chips={chips} backlinks={backlinks}>
      <div style={{ padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
        <WorkItemDetail
          item={item}
          onUpdateWorkItem={handleUpdate}
          onRequestDelete={() => {}}
        />
        <div>
          <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4, marginBottom: 6 }}>
            Activity
          </div>
          <ActivityTimeline
            notes={notes}
            efforts={efforts}
            formatTimestamp={(iso) => new Date(iso).toLocaleString()}
            onOpenFile={onOpenFile}
            onShowInHistory={onShowInHistory}
          />
        </div>
      </div>
      {slideover}
    </Page>
  );
}
