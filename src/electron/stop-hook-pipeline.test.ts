import { describe, expect, test } from "bun:test";
import { decideStopDirective, findStaleEpicChildrenPairs, type ThreadSnapshot } from "./stop-hook-pipeline.js";
import type { Thread } from "../persistence/thread-store.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";

const builders = {
  buildInProgressAuditReason: (items: WorkItem[]) =>
    `audit: ${items.map((i) => i.id).join(",")}`,
  buildWikiCaptureReason: () => "wiki-capture",
  buildFiledButDidntShipReason: () => "filed but didn't ship",
  buildStaleEpicChildrenReason: (pairs: Array<{ epic: WorkItem; staleChildren: WorkItem[] }>) =>
    `stale-epics: ${pairs.map((p) => `${p.epic.id}=>${p.staleChildren.map((c) => c.id).join(",")}`).join("|")}`,
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

function snapshot(parts: Partial<ThreadSnapshot>): ThreadSnapshot {
  return {
    thread: thread(),
    workItems: [],
    ...parts,
  };
}

describe("decideStopDirective", () => {
  test("writer thread with ready work item and no in_progress: allow stop (queue progression is user-driven)", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(snapshot({ workItems: [ready] }), builders);
    expect(out.directive).toBeNull();
  });

  test("non-writer thread with ready work item: allow stop", () => {
    const ready = workItem("w1", 0);
    const out = decideStopDirective(
      snapshot({ thread: thread({ status: "queued" }), workItems: [ready] }),
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
    expect(out.directive).toEqual({ decision: "block", reason: "audit: w1" });
  });

  test("wiki-capture is skipped when buildWikiCaptureReason isn't wired (older callers)", () => {
    const out = decideStopDirective(
      snapshot({ turnHadActivity: false, turnWasExploration: true }),
      {
        buildInProgressAuditReason: builders.buildInProgressAuditReason,
      },
    );
    expect(out.directive).toBeNull();
  });

  test("Q&A turn (turnHadActivity=false) suppresses every directive", () => {
    const ready = workItem("w1", 0);
    const inProgress = workItem("w2", 1, "in_progress");
    const out = decideStopDirective(
      snapshot({
        workItems: [inProgress, ready],
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

  describe("in-progress audit branch", () => {
    test("fires when any work item is in_progress, listing each id", () => {
      const a = workItem("w-a", 0, "in_progress");
      const b = workItem("w-b", 1, "in_progress");
      const out = decideStopDirective(snapshot({ workItems: [a, b] }), builders);
      expect(out.directive).toEqual({ decision: "block", reason: "audit: w-a,w-b" });
    });

    test("fires even when ready work is also queued", () => {
      const inFlight = workItem("w-a", 0, "in_progress");
      const ready = workItem("w-b", 1, "ready");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight, ready] }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a");
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
      const out = decideStopDirective(snapshot({ workItems: [inFlight, ready] }), {});
      expect(out.directive).toBeNull();
    });
  });

  describe("subagent-in-flight suppression", () => {
    test("in-progress audit is suppressed while a subagent is in flight", () => {
      const inFlight = workItem("w-a", 0, "in_progress");
      const out = decideStopDirective(
        snapshot({ workItems: [inFlight], subagentInFlight: true }),
        builders,
      );
      expect(out.directive).toBeNull();
      expect(out.sideEffects).toEqual([]);
    });
  });

  describe("in-progress audit no-change suppression", () => {
    test("first fire emits the audit and a record-signature side effect", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-01T00:00:00Z" });
      const out = decideStopDirective(snapshot({ workItems: [a] }), builders);
      expect(out.directive?.reason).toBe("audit: w-a");
      const recorded = out.sideEffects.find((e) => e.kind === "record-audit-signature");
      expect(recorded).toBeDefined();
      expect((recorded as { signature: string }).signature).toContain("w-a");
    });

    test("second fire with identical signature suppresses the directive", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-01T00:00:00Z" });
      const sig = `w-a|2024-02-01T00:00:00Z|0`;
      const out = decideStopDirective(
        snapshot({ workItems: [a], lastInProgressAuditSignature: sig }),
        builders,
      );
      expect(out.directive).toBeNull();
      expect(out.sideEffects).toEqual([]);
    });

    test("changed updated_at on an in_progress item re-arms the audit", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-02T00:00:00Z" });
      const sig = `w-a|2024-02-01T00:00:00Z|0`;
      const out = decideStopDirective(
        snapshot({ workItems: [a], lastInProgressAuditSignature: sig }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a");
      expect(out.sideEffects.find((e) => e.kind === "record-audit-signature")).toBeDefined();
    });

    test("note added (note_count change) re-arms the audit", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-01T00:00:00Z", note_count: 1 });
      const sig = `w-a|2024-02-01T00:00:00Z|0`;
      const out = decideStopDirective(
        snapshot({ workItems: [a], lastInProgressAuditSignature: sig }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a");
    });

    test("growing the in_progress set re-arms the audit", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-01T00:00:00Z" });
      const b = workItem("w-b", 1, "in_progress", { updated_at: "2024-02-01T00:00:00Z" });
      const sig = `w-a|2024-02-01T00:00:00Z|0`;
      const out = decideStopDirective(
        snapshot({ workItems: [a, b], lastInProgressAuditSignature: sig }),
        builders,
      );
      expect(out.directive?.reason).toBe("audit: w-a,w-b");
    });

    test("shrinking the in_progress set: surviving signature still matches → suppressed", () => {
      const a = workItem("w-a", 0, "in_progress", { updated_at: "2024-02-01T00:00:00Z" });
      const sig = `w-a|2024-02-01T00:00:00Z|0`;
      const out = decideStopDirective(
        snapshot({ workItems: [a], lastInProgressAuditSignature: sig }),
        builders,
      );
      expect(out.directive).toBeNull();
    });
  });

  describe("awaitingUser branch", () => {
    test("awaitingUser=true allows stop even with ready items", () => {
      const ready = workItem("wi-r", 1, "ready");
      const out = decideStopDirective(
        snapshot({
          workItems: [ready],
          turnHadActivity: true,
          awaitingUser: true,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
      expect(out.sideEffects).toEqual([]);
    });

    test("awaitingUser=true suppresses in-progress audit", () => {
      const ip = workItem("wi-ip", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({
          workItems: [ip],
          turnHadActivity: true,
          awaitingUser: true,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });
  });

  describe("filing-enforcement no longer fires at Stop", () => {
    // Filing enforcement moved to the PreToolUse hook
    // (`buildFilingEnforcementPreToolDeny`). The Stop pipeline must
    // pass through writes-without-filing turns so we don't double-
    // block (PreToolUse blocked the edit; Stop blocking again is
    // post-hoc theatre).
    test("writes without filing and no in_progress: pass through", () => {
      const out = decideStopDirective(
        snapshot({
          workItems: [],
          turnHadActivity: true,
          turnHadWrites: true,
          turnHadFiling: false,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("writes under a pre-existing in_progress item: audit fires normally", () => {
      const ip = workItem("wi-ip", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({
          workItems: [ip],
          turnHadActivity: true,
          turnHadWrites: true,
          turnHadFiling: false,
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "audit: wi-ip" });
    });
  });

  describe("stale-epic-children advisory branch", () => {
    test("epic in human_check with a ready child: emit advisory", () => {
      const epic = workItem("wi-epic", 0, "human_check", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "ready", { parent_id: "wi-epic" });
      const out = decideStopDirective(
        snapshot({ workItems: [epic, child], turnHadActivity: true }),
        builders,
      );
      expect(out.directive).toEqual({
        decision: "block",
        reason: "stale-epics: wi-epic=>wi-child",
      });
    });

    test("epic in blocked with an in_progress child: emit advisory", () => {
      const epic = workItem("wi-epic", 0, "blocked", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "in_progress", { parent_id: "wi-epic" });
      const out = decideStopDirective(
        snapshot({ workItems: [epic, child], turnHadActivity: true }),
        builders,
      );
      expect(out.directive?.decision).toBe("block");
      expect(out.directive?.reason).toContain("wi-child");
    });

    test("epic in human_check with all-terminal children: no advisory", () => {
      const epic = workItem("wi-epic", 0, "human_check", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "human_check", { parent_id: "wi-epic" });
      const out = decideStopDirective(
        snapshot({ workItems: [epic, child], turnHadActivity: true }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("ready epic with ready children: not closed, no advisory", () => {
      const epic = workItem("wi-epic", 0, "ready", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "ready", { parent_id: "wi-epic" });
      const out = decideStopDirective(
        snapshot({ workItems: [epic, child], turnHadActivity: true }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("awaitingUser overrides stale-epic-children", () => {
      const epic = workItem("wi-epic", 0, "human_check", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "ready", { parent_id: "wi-epic" });
      const out = decideStopDirective(
        snapshot({ workItems: [epic, child], turnHadActivity: true, awaitingUser: true }),
        builders,
      );
      expect(out.directive).toBeNull();
    });
  });

  describe("findStaleEpicChildrenPairs", () => {
    test("returns empty array when no epics are closed", () => {
      const epic = workItem("wi-epic", 0, "ready", { kind: "epic" as WorkItemKind });
      const child = workItem("wi-child", 1, "ready", { parent_id: "wi-epic" });
      expect(findStaleEpicChildrenPairs([epic, child])).toEqual([]);
    });

    test("returns each closed epic with its non-terminal children", () => {
      const e1 = workItem("e1", 0, "human_check", { kind: "epic" as WorkItemKind });
      const c1a = workItem("c1a", 1, "ready", { parent_id: "e1" });
      const c1b = workItem("c1b", 2, "human_check", { parent_id: "e1" });
      const e2 = workItem("e2", 3, "blocked", { kind: "epic" as WorkItemKind });
      const c2a = workItem("c2a", 4, "in_progress", { parent_id: "e2" });
      const pairs = findStaleEpicChildrenPairs([e1, c1a, c1b, e2, c2a]);
      expect(pairs).toHaveLength(2);
      expect(pairs[0]!.epic.id).toBe("e1");
      expect(pairs[0]!.staleChildren.map((c) => c.id)).toEqual(["c1a"]);
      expect(pairs[1]!.epic.id).toBe("e2");
      expect(pairs[1]!.staleChildren.map((c) => c.id)).toEqual(["c2a"]);
    });
  });

  describe("filed-but-didn't-ship advisory branch", () => {
    test("filed a ready item, no writes, no in_progress: emit advisory", () => {
      const out = decideStopDirective(
        snapshot({
          workItems: [],
          turnHadActivity: true,
          turnHadWrites: false,
          turnHadFiling: true,
          turnFiledReadyItem: true,
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "filed but didn't ship" });
    });

    test("filed a ready item AND made writes: pass through (writes are the legitimate path)", () => {
      const out = decideStopDirective(
        snapshot({
          workItems: [],
          turnHadActivity: true,
          turnHadWrites: true,
          turnHadFiling: true,
          turnFiledReadyItem: true,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("filed a ready item but has an in_progress item: audit takes precedence", () => {
      const ip = workItem("wi-ip", 1, "in_progress");
      const out = decideStopDirective(
        snapshot({
          workItems: [ip],
          turnHadActivity: true,
          turnHadWrites: false,
          turnHadFiling: true,
          turnFiledReadyItem: true,
        }),
        builders,
      );
      expect(out.directive).toEqual({ decision: "block", reason: "audit: wi-ip" });
    });

    test("filing call but no ready row landed (e.g. transition only): no advisory", () => {
      const out = decideStopDirective(
        snapshot({
          workItems: [],
          turnHadActivity: true,
          turnHadWrites: false,
          turnHadFiling: true,
          turnFiledReadyItem: false,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });

    test("awaitingUser overrides filed-but-didn't-ship", () => {
      const out = decideStopDirective(
        snapshot({
          workItems: [],
          turnHadActivity: true,
          turnHadWrites: false,
          turnHadFiling: true,
          turnFiledReadyItem: true,
          awaitingUser: true,
        }),
        builders,
      );
      expect(out.directive).toBeNull();
    });
  });
});
