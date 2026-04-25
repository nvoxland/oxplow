import { describe, expect, test } from "bun:test";
import { decideStopDirective, type ThreadSnapshot } from "./stop-hook-pipeline.js";
import type { Thread } from "../persistence/thread-store.js";
import type { CommitPoint, CommitPointStatus } from "../persistence/commit-point-store.js";
import type { WaitPoint, WaitPointStatus } from "../persistence/wait-point-store.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";

const builders = {
  buildCommitPointReason: (cp: CommitPoint) => `commit: ${cp.id}`,
  buildNextWorkItemReason: (item: WorkItem) => `next: ${item.id}`,
  buildInProgressAuditReason: (items: WorkItem[]) =>
    `audit: ${items.map((i) => i.id).join(",")}`,
  buildWikiCaptureReason: () => "wiki-capture",
};

function thread(overrides: Partial<Thread> = {}): Thread {
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
    thread_id: "b1",
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
    author: null,
    ...overrides,
  };
}

function commitPoint(id: string, sort_index: number, status: CommitPointStatus = "pending", mode: import("../persistence/commit-point-store.js").CommitPointMode = "approve"): CommitPoint {
  return {
    id,
    thread_id: "b1",
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
    thread_id: "b1",
    sort_index,
    status,
    note: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  };
}

function snapshot(parts: Partial<ThreadSnapshot>): ThreadSnapshot {
  return {
    thread: thread(),
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
    // The audit branch fires for the in_progress item — commit waits behind it.
    expect(out.directive).toEqual({ decision: "block", reason: "audit: w1" });
  });

  test("pending wait point fires once preceding items are human_check (agents never self-mark done)", () => {
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

  test("writer thread with ready work item and no in_progress: block with next-item reason", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
  });

  test("non-writer thread with ready work item: allow stop (no auto-progression)", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ thread: thread({ status: "queued" }), workItems: [ready], readyWorkItems: [ready] }),
      builders,
    );
    expect(out.directive).toBeNull();
  });

  test("nothing pending and no ready items: allow stop", () => {
    const out = decideStopDirective(snapshot({}), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("read-heavy Q&A turn (turnWasExploration=true) emits the wiki-capture directive", () => {
    const out = decideStopDirective(
      snapshot({ turnHadActivity: false, turnWasExploration: true }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "wiki-capture" });
  });

  test("wiki-capture directive is suppressed when justEmittedWikiCapture is true", () => {
    const out = decideStopDirective(
      snapshot({
        turnHadActivity: false,
        turnWasExploration: true,
        justEmittedWikiCapture: true,
      }),
      builders,
    );
    expect(out.directive).toBeNull();
  });

  test("wiki-capture directive only fires on active threads", () => {
    const out = decideStopDirective(
      snapshot({
        thread: thread({ status: "queued" }),
        turnHadActivity: false,
        turnWasExploration: true,
      }),
      builders,
    );
    expect(out.directive).toBeNull();
  });

  test("wiki-capture directive only fires when turnHadActivity is false (real-work turns take precedence)", () => {
    const inProgress = workItem("w1", 0, "in_progress");
    const out = decideStopDirective(
      snapshot({
        workItems: [inProgress],
        turnHadActivity: true,
        turnWasExploration: true,
      }),
      builders,
    );
    // Real-work turn → audit branch wins, capture is ignored.
    expect(out.directive).toEqual({ decision: "block", reason: "audit: w1" });
  });

  test("wiki-capture is skipped when buildWikiCaptureReason isn't wired (older callers)", () => {
    const out = decideStopDirective(
      snapshot({ turnHadActivity: false, turnWasExploration: true }),
      {
        buildCommitPointReason: builders.buildCommitPointReason,
        buildNextWorkItemReason: builders.buildNextWorkItemReason,
        buildInProgressAuditReason: builders.buildInProgressAuditReason,
        // no buildWikiCaptureReason
      },
    );
    expect(out.directive).toBeNull();
  });

  test("Q&A turn (turnHadActivity=false) suppresses every directive", () => {
    const ready = workItem("w1", 0);
    const inProgress = workItem("w2", 1, "in_progress");
    const cp = commitPoint("cp1", 2);
    const wp = waitPoint("wp1", 3);
    const out = decideStopDirective(
      snapshot({
        commitPoints: [cp],
        waitPoints: [wp],
        workItems: [inProgress, ready],
        readyWorkItems: [ready],
        autoCommit: true,
        turnHadActivity: false,
      }),
      builders,
    );
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("turnHadActivity=true behaves the same as undefined (no suppression)", () => {
    const inProgress = workItem("w2", 1, "in_progress");
    const out = decideStopDirective(
      snapshot({ workItems: [inProgress], turnHadActivity: true }),
      builders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "audit: w2" });
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

  test("non-writer thread with pending commit point: allow stop (only the active thread commits)", () => {
    const cp = commitPoint("cp1", 1);
    const out = decideStopDirective(
      snapshot({ thread: thread({ status: "queued" }), commitPoints: [cp] }),
      builders,
    );
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("auto-mode commit point done: pipeline allows stop (no double-block)", () => {
    const cp = commitPoint("cp1", 0, "done", "auto");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toBeNull();
    expect(out.sideEffects).toEqual([]);
  });

  test("auto-mode commit point pending with buildAutoCommitReason wired: emits the auto-commit directive", () => {
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), autoBuilders);
    expect(out.directive).toEqual({ decision: "block", reason: "auto: cp1" });
  });

  test("auto-mode commit point pending without buildAutoCommitReason: falls back to the approve-mode directive", () => {
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
  });

  test("auto_commit=true with settled work and no commit point: emits the ad-hoc auto-commit directive", () => {
    const settled = workItem("w1", 0, "human_check");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true }),
        workItems: [settled],
        autoCommit: true,
      }),
      autoBuilders,
    );
    expect(out.directive).toEqual({ decision: "block", reason: "auto: ad-hoc" });
  });

  test("auto_commit=true with settled work but a clean worktree: suppress directive (wi-ec4c8e6f44fd)", () => {
    const settled = workItem("w1", 0, "human_check");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true }),
        workItems: [settled],
        autoCommit: true,
        worktreeClean: true,
      }),
      autoBuilders,
    );
    expect(out.directive).toBeNull();
  });

  test("auto-mode commit point pending but a clean worktree: suppress directive (wi-ec4c8e6f44fd)", () => {
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({ commitPoints: [cp], worktreeClean: true }),
      autoBuilders,
    );
    expect(out.directive).toBeNull();
  });

  test("auto_commit=true with no settled work: allow stop (nothing to commit)", () => {
    // No settled items, no commit_point → auto-commit branch passes through.
    // Ready/in_progress branches handle the rest. Here neither: allow stop.
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true }),
        workItems: [],
        autoCommit: true,
      }),
      autoBuilders,
    );
    expect(out.directive).toBeNull();
  });

  test("auto_commit=true with a ready work item: ad-hoc auto-commit wins over ready-work directive", () => {
    const settled = workItem("w1", 0, "human_check");
    const ready = workItem("w2", 1, "ready");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true }),
        workItems: [settled, ready],
        readyWorkItems: [ready],
        autoCommit: true,
      }),
      autoBuilders,
    );
    expect(out.directive?.reason).toBe("auto: ad-hoc");
  });

  test("auto_commit=true on a non-active thread: no auto-commit directive (only writers commit)", () => {
    const settled = workItem("w1", 0, "human_check");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true, status: "queued" }),
        workItems: [settled],
        autoCommit: true,
      }),
      autoBuilders,
    );
    expect(out.directive).toBeNull();
  });

  describe("in-progress audit branch", () => {
    test("fires when any work item is in_progress, listing each id", () => {
      const a = workItem("w-a", 0, "in_progress");
      const b = workItem("w-b", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({ workItems: [a, b] }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "audit: w-a,w-b" });
    });

    test("fires even when ready work is also queued — audit takes priority", () => {
      // The audit ensures stale in_progress rows are reconciled before the
      // agent picks anything new up.
      const inFlight = workItem("w-a", 0, "in_progress");
      const ready = workItem("w-b", 1, "ready");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight, ready], readyWorkItems: [ready] }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a");
    });

    test("commit point still takes priority over the audit", () => {
      // Commit point at sort_index 0 with no preceding items beats the audit.
      const cp = commitPoint("cp1", 0);
      const inFlight = workItem("w-a", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight], commitPoints: [cp] }),
        builders,
      );
      expect(out.directive?.reason).toBe("commit: cp1");
    });

    test("wait point still takes priority over the audit", () => {
      // Wait point with no preceding items emits side effect and allows stop.
      const wp = waitPoint("wp1", 0);
      const inFlight = workItem("w-a", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight], waitPoints: [wp] }),
        builders,
      );
      expect(out.directive).toBeNull();
      expect(out.sideEffects).toEqual([{ kind: "trigger-wait-point", id: "wp1" }]);
    });

    test("non-writer thread does not emit the audit (read-only thread)", () => {
      const inFlight = workItem("w-a", 0, "in_progress");
      const out = decideStopDirective(
        snapshot({ thread: thread({ status: "queued" }), workItems: [inFlight] }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("is opt-in — omitting buildInProgressAuditReason skips the branch entirely", () => {
      const inFlight = workItem("w-a", 0, "in_progress");
      const ready = workItem("w-b", 1, "ready");
      const noAuditBuilders = {
        buildCommitPointReason: builders.buildCommitPointReason,
        buildNextWorkItemReason: builders.buildNextWorkItemReason,
      };
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight, ready], readyWorkItems: [ready] }),
        noAuditBuilders,
      );
      // Falls through to ready-work.
      expect(out.directive).toEqual({ decision: "block", reason: "next: w-b" });
    });
  });

  describe("subagent-in-flight suppression", () => {
    test("in-progress audit is suppressed while a subagent is in flight", () => {
      // The orchestrator dispatched a Task subagent that owns the
      // in_progress item. Re-firing the audit nudge mid-flight produces
      // the visual loop the user complained about.
      const inFlight = workItem("w-a", 0, "in_progress");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight], subagentInFlight: true }),
        builders,
      );
      expect(out.directive).toBeNull();
      expect(out.sideEffects).toEqual([]);
    });

    test("ready-work directive is suppressed while a subagent is in flight", () => {
      // The parent shouldn't pick up a new item while a subagent is
      // still working — same noise loop, different branch.
      const ready = workItem("w-b", 0, "ready");
      const out = decideStopDirective(
        snapshot({ workItems: [ready], readyWorkItems: [ready], subagentInFlight: true }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("commit/wait points still fire even with a subagent in flight", () => {
      // Markers are user-placed and represent explicit work, not the
      // queue-management nudges this suppression targets.
      const cp = commitPoint("cp1", 1);
      const out = decideStopDirective(
        snapshot({ commitPoints: [cp], subagentInFlight: true }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
    });
  });

  describe("ready-work suppression rules", () => {
    test("just-read-ready: ready set matches last read, suppressed", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          justReadReadySet: ["w1"],
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("just-read-ready: ready set differs, directive fires", () => {
      const ready = workItem("w2", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          justReadReadySet: ["w1"],
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "next: w2" });
    });

    test("just-read-ready: no prior read recorded, directive fires", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
    });

    test("just-read-ready does NOT suppress the in-progress audit branch", () => {
      // The audit is independent of ready-work suppression.
      const inFlight = workItem("w-a", 0, "in_progress");
      const out = decideStopDirective(
        snapshot({
          workItems: [inFlight],
          readyWorkItems: [],
          justReadReadySet: ["w-a"],
        }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a");
    });
  });
});
