import { describe, expect, test } from "bun:test";
import { decideStopDirective, type BatchSnapshot } from "./stop-hook-pipeline.js";
import type { Batch } from "../persistence/batch-store.js";
import type { CommitPoint, CommitPointMode, CommitPointStatus } from "../persistence/commit-point-store.js";
import type { WaitPoint, WaitPointStatus } from "../persistence/wait-point-store.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";

const builders = {
  buildCommitPointReason: (cp: CommitPoint) => `commit: ${cp.id}`,
  buildNextWorkItemReason: (item: WorkItem) => `next: ${item.id}`,
};

function batch(overrides: Partial<Batch> = {}): Batch {
  return {
    id: "b1",
    stream_id: "s1",
    title: "B",
    status: "active",
    sort_index: 0,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    pane_target: "p",
    resume_session_id: "",
    summary: "",
    summary_updated_at: null,
    ...overrides,
  };
}

function workItem(id: string, sort_index: number, status: WorkItemStatus = "ready", overrides: Partial<WorkItem> = {}): WorkItem {
  return {
    id,
    batch_id: "b1",
    parent_id: null,
    kind: "task" as WorkItemKind,
    title: id,
    description: "",
    acceptance_criteria: null,
    status,
    priority: "medium" as WorkItemPriority,
    sort_index,
    created_by: "user",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
    deleted_at: null,
    ...overrides,
  };
}

function commitPoint(id: string, sort_index: number, status: CommitPointStatus = "pending", mode: CommitPointMode = "approval"): CommitPoint {
  return {
    id,
    batch_id: "b1",
    sort_index,
    mode,
    status,
    proposed_message: null,
    approved_message: null,
    commit_sha: null,
    rejection_note: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  };
}

function waitPoint(id: string, sort_index: number, status: WaitPointStatus = "pending"): WaitPoint {
  return {
    id,
    batch_id: "b1",
    sort_index,
    status,
    note: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  };
}

function snapshot(parts: Partial<BatchSnapshot>): BatchSnapshot {
  return {
    batch: batch(),
    commitPoints: [],
    waitPoints: [],
    workItems: [],
    readyWorkItems: [],
    ...parts,
  };
}

describe("decideStopDirective", () => {
  test("pending commit point with no preceding items: block with commit reason", () => {
    const cp = commitPoint("cp1", 1);
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
    expect(out.sideEffects).toEqual([]);
  });

  test("pending commit point waits until preceding work items are done", () => {
    const items = [workItem("w1", 0, "in_progress")];
    const cp = commitPoint("cp1", 1);
    const out = decideStopDirective(snapshot({ commitPoints: [cp], workItems: items }), builders);
    expect(out.directive).toBeNull();
  });

  test("pending wait point: emits trigger side effect, allows stop", () => {
    const wp = waitPoint("wp1", 1);
    const out = decideStopDirective(snapshot({ waitPoints: [wp] }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([{ kind: "trigger-wait-point", id: "wp1" }]);
  });

  test("triggered wait point is treated as consumed", () => {
    const wp = waitPoint("wp1", 1, "triggered");
    const out = decideStopDirective(snapshot({ waitPoints: [wp] }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("approval-mode commit at proposed: allow stop, no side effects", () => {
    const cp = commitPoint("cp1", 1, "proposed", "approval");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("writer batch with ready work item: block with next-item reason", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
  });

  test("non-writer batch with ready work item: allow stop (no auto-progression)", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ batch: batch({ status: "queued" }), workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive).toBeNull();
  });

  test("nothing pending and no ready items: allow stop", () => {
    const out = decideStopDirective(snapshot({}), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("commit point takes priority over wait point at same position", () => {
    const cp = commitPoint("cp1", 1);
    const wp = waitPoint("wp1", 1);
    const out = decideStopDirective(snapshot({ commitPoints: [cp], waitPoints: [wp] }), builders);
    expect(out.directive?.reason).toContain("commit: cp1");
  });

  test("done commit points are skipped, the next pending one wins", () => {
    const done = commitPoint("cp0", 0, "done");
    const pending = commitPoint("cp1", 2);
    const out = decideStopDirective(
      snapshot({ commitPoints: [done, pending], workItems: [workItem("w1", 1, "done")] }),
      builders,
    );
    expect(out.directive?.reason).toContain("commit: cp1");
  });

  test("non-writer batch with pending commit point: allow stop (only the active batch commits)", () => {
    const cp = commitPoint("cp1", 1);
    const out = decideStopDirective(
      snapshot({ batch: batch({ status: "queued" }), commitPoints: [cp] }),
      builders,
    );
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("rejected commit point still active until pending again (so user can retry)", () => {
    // findActiveMarker uses `cp.status !== "done"`, so rejected counts as
    // not-done. But the directive only fires for status === "pending", so a
    // rejected point holds up the queue without re-prompting the agent.
    const rejected = commitPoint("cp1", 1, "rejected");
    const out = decideStopDirective(snapshot({ commitPoints: [rejected] }), builders);
    expect(out.directive).toBeNull();
  });
});
