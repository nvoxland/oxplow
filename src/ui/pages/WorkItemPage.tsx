import { useEffect, useState } from "react";
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
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks } from "../tabs/useBacklinks.js";

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
}: WorkItemPageProps) {
  const item = items.find((i) => i.id === itemId) ?? null;
  const backlinkEntries = useBacklinks(workItemRef(itemId), stream, threadWork);
  const backlinks = <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />;
  const [notes, setNotes] = useState<WorkNote[]>([]);
  const [efforts, setEfforts] = useState<EffortDetail[]>([]);

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

  if (!item) {
    return (
      <Page testId="page-work-item" title={itemId} kind="work item" backlinks={backlinks}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: 13 }}>
          This work item is not loaded in the current thread. Open the thread that owns it to edit, or use the rail to navigate.
        </div>
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
    </Page>
  );
}
