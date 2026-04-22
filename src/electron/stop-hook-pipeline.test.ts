import { describe, expect, test } from "bun:test";
import { decideStopDirective, type BatchSnapshot } from "./stop-hook-pipeline.js";
import type { Batch } from "../persistence/batch-store.js";
import type { CommitPoint, CommitPointStatus } from "../persistence/commit-point-store.js";
import type { WaitPoint, WaitPointStatus } from "../persistence/wait-point-store.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";

const builders = {
  buildCommitPointReason: (cp: CommitPoint) => `commit: ${cp.id}`,
  buildNextWorkItemReason: (item: WorkItem, context: { uiChangeNudge?: boolean }) =>
    `next: ${item.id}${context.uiChangeNudge ? " [ui-nudge]" : ""}`,
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
    auto_commit: false,
    custom_prompt: null,
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
    note_count: 0,
    ...overrides,
  };
}

function commitPoint(id: string, sort_index: number, status: CommitPointStatus = "pending", mode: import("../persistence/commit-point-store.js").CommitPointMode = "approve"): CommitPoint {
  return {
    id,
    batch_id: "b1",
    sort_index,
    mode,
    status,
    commit_sha: null,
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

  test("pending wait point fires once preceding items are human_check (agents never self-mark done)", () => {
    // Regression: previously we required preceding items to be `done` /
    // `canceled`, which never happened in practice because the agent leaves
    // finished work in `human_check` for the user to verify. The wait line
    // therefore never triggered and the agent kept marching past it.
    const items = [workItem("w1", 0, "human_check")];
    const wp = waitPoint("wp1", 1);
    const out = decideStopDirective(snapshot({ waitPoints: [wp], workItems: items }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([{ kind: "trigger-wait-point", id: "wp1" }]);
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

  test("writer batch with ready work item: block with next-item reason", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
  });

  test("allow stop when every ready item was filed by the agent during this turn (triage inbox)", () => {
    // Regression: /autoimprove and similar flows end with the agent FILING
    // items for the user to triage. Pre-fix, the Stop hook immediately
    // pushed the agent into implementing them — the exact opposite of the
    // intent. Now the hook ignores items created_by="agent" with
    // created_at >= the current turn's started_at.
    const justFiled = workItem("w1", 0, "ready", {
      created_by: "agent",
      created_at: "2024-06-01T12:00:10Z",
    });
    const out = decideStopDirective(
      snapshot({
        workItems: [justFiled],
        readyWorkItems: [justFiled],
        currentTurnStartedAt: "2024-06-01T12:00:00Z",
      }),
      builders,
    );
    expect(out.directive).toBeNull();
  });

  test("mixed queue: agent-filed-this-turn items are skipped, pre-existing ready items still fire the directive", () => {
    const preExisting = workItem("w1", 0, "ready", {
      created_by: "user",
      created_at: "2024-05-01T00:00:00Z",
    });
    const justFiled = workItem("w2", 1, "ready", {
      created_by: "agent",
      created_at: "2024-06-01T12:00:10Z",
    });
    const out = decideStopDirective(
      snapshot({
        workItems: [preExisting, justFiled],
        readyWorkItems: [preExisting, justFiled],
        currentTurnStartedAt: "2024-06-01T12:00:00Z",
      }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
  });

  test("user-filed items during this turn still fire the directive (only agent-filed ones are skipped)", () => {
    // Rationale: a user who filed something mid-turn WANTS the agent to
    // pick it up next; the skip is specifically about agents' own inboxes.
    const userFiled = workItem("w1", 0, "ready", {
      created_by: "user",
      created_at: "2024-06-01T12:00:10Z",
    });
    const out = decideStopDirective(
      snapshot({
        workItems: [userFiled],
        readyWorkItems: [userFiled],
        currentTurnStartedAt: "2024-06-01T12:00:00Z",
      }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
  });

  test("next-item directive carries the UI-change nudge when currentTurnFilePaths touches src/ui", () => {
    // Acceptance: a turn that wrote to any src/ui/** path produces a Stop
    // hook response whose next-item directive carries the visual-
    // verification nudge.
    const ready = workItem("w1", 0, "ready", { created_by: "user" });
    const out = decideStopDirective(
      snapshot({
        workItems: [ready],
        readyWorkItems: [ready],
        currentTurnFilePaths: ["src/ui/components/Plan/PlanPane.tsx"],
      }),
      builders,
    );
    expect(out.directive?.reason).toContain("[ui-nudge]");
  });

  test("next-item directive skips the UI nudge when only server / persistence paths changed", () => {
    const ready = workItem("w1", 0, "ready", { created_by: "user" });
    const out = decideStopDirective(
      snapshot({
        workItems: [ready],
        readyWorkItems: [ready],
        currentTurnFilePaths: ["src/electron/runtime.ts", "src/persistence/batch-store.ts"],
      }),
      builders,
    );
    expect(out.directive?.reason).not.toContain("[ui-nudge]");
  });

  test("next-item directive skips the UI nudge when currentTurnFilePaths is absent", () => {
    const ready = workItem("w1", 0, "ready", { created_by: "user" });
    const out = decideStopDirective(
      snapshot({ workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive?.reason).not.toContain("[ui-nudge]");
  });

  test("without a currentTurnStartedAt snapshot field, behaviour matches the pre-fix baseline", () => {
    // Backwards compat: builds that pre-date the runtime plumbing still
    // behave as before (no filtering).
    const justFiled = workItem("w1", 0, "ready", {
      created_by: "agent",
      created_at: "2024-06-01T12:00:10Z",
    });
    const out = decideStopDirective(
      snapshot({
        workItems: [justFiled],
        readyWorkItems: [justFiled],
      }),
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

  test("auto-mode commit point, done after runtime execution: pipeline allows stop (no double-block)", () => {
    // Contract: the runtime calls executeAutoCommitPoint before decideStopDirective,
    // so by the time the pipeline runs the auto-mode commit point is already "done".
    // This test confirms the pipeline does not re-block on a done auto-mode commit.
    const cp = commitPoint("cp1", 0, "done", "auto");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("auto-mode commit point pending (runtime did not process it): pipeline blocks as a safety net", () => {
    // If the runtime fails to execute an auto-mode commit, the pipeline catches it.
    // The fix (passing mode='auto' from the synthesis call) ensures findActiveAutoCommitPoint
    // picks it up; before the fix, mode='approve' would be synthesized and would also
    // block here — but now the intent is clear.
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
  });

  test("auto_commit=true snapshot with no commit points falls through to ready-work handling", () => {
    // Contract: the runtime auto-commit pre-pass runs BEFORE decideStopDirective
    // and never creates a commit_point row. So the pipeline sees the batch with
    // `autoCommit: true` but an empty commitPoints list and must not invent any
    // commit directive from the flag itself — it should just handle whatever
    // work remains. Here: no ready items, no commits, so allow stop.
    const settled = workItem("w1", 0, "human_check");
    const out = decideStopDirective(
      snapshot({
        batch: batch({ auto_commit: true }),
        workItems: [settled],
        autoCommit: true,
      }),
      builders,
    );
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("auto_commit=true with a ready work item: pipeline still fires the next-item directive", () => {
    // Pre-pass already ran (or had nothing settled), but more work remains.
    // The pipeline must keep auto-progressing through the queue.
    const ready = workItem("w2", 1, "ready");
    const out = decideStopDirective(
      snapshot({
        batch: batch({ auto_commit: true }),
        workItems: [ready],
        readyWorkItems: [ready],
        autoCommit: true,
      }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w2" });
  });

  describe("human_check nudge", () => {
    const nudgeBuilders = {
      ...builders,
      buildHumanCheckNudgeReason: (item: WorkItem) => `nudge: ${item.id}`,
    };

    test("fires when an in_progress item wasn't touched this turn and no blocker was raised", () => {
      const stale = workItem("w-stale", 0, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [stale],
          readyWorkItems: [],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        nudgeBuilders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "nudge: w-stale" });
    });

    test("suppresses when the in_progress item was updated during this turn", () => {
      const fresh = workItem("w-fresh", 0, "in_progress", { updated_at: "2024-06-01T00:05:30Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [fresh],
          readyWorkItems: [],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        nudgeBuilders,
      );
      expect(out.directive).toBeNull();
    });

    test("suppresses when another item was flagged `blocked` during this turn", () => {
      const stale = workItem("w-stale", 0, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const blocker = workItem("w-block", 1, "blocked", { updated_at: "2024-06-01T00:05:30Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [stale, blocker],
          readyWorkItems: [],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        nudgeBuilders,
      );
      expect(out.directive).toBeNull();
    });

    test("suppresses when there's no current turn (no anchor to compare against)", () => {
      const stale = workItem("w-stale", 0, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [stale],
          readyWorkItems: [],
          currentTurnStartedAt: null,
        }),
        nudgeBuilders,
      );
      expect(out.directive).toBeNull();
    });

    test("suppresses when two or more items are in_progress (convention: one at a time)", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const b = workItem("w-b", 1, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [a, b],
          readyWorkItems: [],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        nudgeBuilders,
      );
      expect(out.directive).toBeNull();
    });

    test("is opt-in — omitting the builder never fires the nudge", () => {
      const stale = workItem("w-stale", 0, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [stale],
          readyWorkItems: [],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("commit point still takes priority over the nudge", () => {
      // Commit point sits at the front of the queue with no preceding
      // items (so it's "active"), followed by the stale in_progress item.
      // The pipeline's commit-point branch should fire first and the
      // nudge branch should never be evaluated.
      const cp = commitPoint("cp1", 0);
      const stale = workItem("w-stale", 1, "in_progress", { updated_at: "2024-06-01T00:00:00Z" });
      const out = decideStopDirective(
        snapshot({
          workItems: [stale],
          readyWorkItems: [],
          commitPoints: [cp],
          currentTurnStartedAt: "2024-06-01T00:05:00Z",
        }),
        nudgeBuilders,
      );
      expect(out.directive?.reason).toBe("commit: cp1");
    });
  });

});
