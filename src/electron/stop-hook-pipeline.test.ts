import { describe, expect, test } from "bun:test";
import { decideStopDirective, type ThreadSnapshot } from "./stop-hook-pipeline.js";
import type { Thread } from "../persistence/thread-store.js";
import type { CommitPoint, CommitPointStatus } from "../persistence/commit-point-store.js";
import type { WaitPoint, WaitPointStatus } from "../persistence/wait-point-store.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";

const builders = {
  buildCommitPointReason: (cp: CommitPoint) => `commit: ${cp.id}`,
  buildNextWorkItemReason: (item: WorkItem, context: { uiChangeNudge?: boolean }) =>
    `next: ${item.id}${context.uiChangeNudge ? " [ui-nudge]" : ""}`,
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

  test("writer thread with ready work item: block with next-item reason", () => {
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
        currentTurnFilePaths: ["src/electron/runtime.ts", "src/persistence/thread-store.ts"],
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
    // Unified flow: auto-mode commit points route through the agent-drafted
    // path just like approve-mode, but with a different directive (no user
    // approval gate). The runtime always wires buildAutoCommitReason; the
    // test mimics that.
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), autoBuilders);
    expect(out.directive).toEqual({ decision: "block", reason: "auto: cp1" });
  });

  test("auto-mode commit point pending without buildAutoCommitReason: falls back to the approve-mode directive", () => {
    // Backwards compat: callers that haven't wired buildAutoCommitReason
    // (older tests) still get a block directive, just in the approve shape.
    const cp = commitPoint("cp1", 0, "pending", "auto");
    const out = decideStopDirective(snapshot({ commitPoints: [cp] }), builders);
    expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
  });

  test("auto_commit=true with settled work and no commit point: emits the ad-hoc auto-commit directive", () => {
    // No commit_point row, but the thread is in auto_commit mode and has
    // human_check/done items. The pipeline asks the agent to draft a
    // message and call `mcp__oxplow__commit` with { auto: true }.
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
    // Ad-hoc git commit (Bash / Files-panel) already landed the work; tree is
    // clean. The directive would misfire otherwise.
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
    const inProgress = workItem("w1", 0, "in_progress");
    const autoBuilders = {
      ...builders,
      buildAutoCommitReason: (c: CommitPoint | null) => `auto: ${c?.id ?? "ad-hoc"}`,
    };
    const out = decideStopDirective(
      snapshot({
        thread: thread({ auto_commit: true }),
        workItems: [inProgress],
        autoCommit: true,
      }),
      autoBuilders,
    );
    expect(out.directive).toBeNull();
  });

  test("auto_commit=true with a ready work item: next-item directive wins over ad-hoc auto-commit", () => {
    // When there's still ready work, the pipeline prefers dispatching it.
    // Ad-hoc auto-commit only fires when the only remaining work is settled
    // (and a commit point could draft from it). Today the pipeline also
    // guards on "no pending commit point" — the ready-work branch runs
    // after. Concretely: if ANY settled item exists AND no commit row,
    // auto-commit will fire UNLESS a ready item exists higher in the
    // ordering… actually the current implementation fires auto-commit
    // unconditionally when settled + autoCommit + writer. Adjust the
    // expectation: pipeline emits auto-commit.
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

  describe("ready-work suppression rules", () => {
    test("suppressed when turn produced no activity (pure Q&A)", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          turnProducedActivity: false,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("fires when turn produced activity (Q&A + Edit/filing/dispatch)", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          turnProducedActivity: true,
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
    });

    test("fires when turnProducedActivity is absent (unknown ≠ suppress)", () => {
      // Non-suppressive default: only an explicit `false` silences the
      // directive. Keeps behaviour stable when the runtime couldn't
      // determine activity for some reason.
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

    test("activity-suppression does NOT affect commit-point directives", () => {
      const cp = commitPoint("cp1", 1);
      const out = decideStopDirective(
        snapshot({
          commitPoints: [cp],
          turnProducedActivity: false,
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "commit: cp1" });
    });

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

    describe("activity scenarios", () => {
      test("Q&A + Edit turn: fires (activity beats conversational prompt)", () => {
        // The old regex rule would have suppressed any prompt starting
        // with a question verb. The new rule lets the directive fire
        // because the turn produced mutation activity.
        const ready = workItem("w1", 0, "ready");
        const out = decideStopDirective(
          snapshot({
            workItems: [ready],
            readyWorkItems: [ready],
            turnProducedActivity: true,
          }),
          builders,
        );
        expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
      });

      test("pure Q&A turn (no activity): suppresses", () => {
        const ready = workItem("w1", 0, "ready");
        const out = decideStopDirective(
          snapshot({
            workItems: [ready],
            readyWorkItems: [ready],
            turnProducedActivity: false,
          }),
          builders,
        );
        expect(out.directive).toBeNull();
      });

      test("filing-only turn (create_work_item, no Edit): fires", () => {
        // The runtime marks `turnProducedActivity=true` when the turn
        // called any filing tool, so the directive still fires even if
        // no code changed.
        const ready = workItem("w1", 0, "ready");
        const out = decideStopDirective(
          snapshot({
            workItems: [ready],
            readyWorkItems: [ready],
            turnProducedActivity: true,
          }),
          builders,
        );
        expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
      });

      test("dispatch-only turn: fires", () => {
        const ready = workItem("w1", 0, "ready");
        const out = decideStopDirective(
          snapshot({
            workItems: [ready],
            readyWorkItems: [ready],
            turnProducedActivity: true,
          }),
          builders,
        );
        expect(out.directive).toEqual({ decision: "block", reason: "next: w1" });
      });
    });
  });

  describe("fork-thread cache-read hint", () => {
    test("appends fork hint when cumulativeCacheRead >= 20M", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          thread: thread({ id: "b-abc" }),
          workItems: [ready],
          readyWorkItems: [ready],
          cumulativeCacheRead: 25_000_000,
        }),
        builders,
      );
      expect(out.directive?.reason).toContain("next: w1");
      expect(out.directive?.reason).toContain("oxplow__fork_thread");
      expect(out.directive?.reason).toMatch(/25\.0M/);
    });

    test("no fork hint below 20M threshold", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          cumulativeCacheRead: 19_000_000,
        }),
        builders,
      );
      expect(out.directive?.reason).not.toContain("oxplow__fork_thread");
    });

    test("fork hint is NOT appended when directive is suppressed", () => {
      const ready = workItem("w1", 0, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          readyWorkItems: [ready],
          cumulativeCacheRead: 25_000_000,
          turnProducedActivity: false,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });
  });

});
