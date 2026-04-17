import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { FileChangeStore, type BatchFileChange } from "./file-change-store.js";
import { TurnStore } from "./turn-store.js";
import type { Stream } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-file-changes-"));
  const batchStore = new BatchStore(dir);
  const stream: Stream = {
    id: "s-1",
    title: "Demo",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: "/tmp/demo",
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    panes: { working: "newde-demo:working-s-1", talking: "newde-demo:talking-s-1" },
    resume: { working_session_id: "", talking_session_id: "" },
  };
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  return { dir, batchId, turns: new TurnStore(dir), changes: new FileChangeStore(dir) };
}

describe("FileChangeStore", () => {
  test("record persists and emits; listForBatch returns newest-first", () => {
    const { batchId, changes } = seed();
    const seen: BatchFileChange[] = [];
    changes.subscribe((c) => seen.push(c));

    changes.record({
      batchId,
      turnId: null,
      workItemId: null,
      path: "a.ts",
      changeKind: "updated",
      source: "fs-watch",
    });
    changes.record({
      batchId,
      turnId: null,
      workItemId: null,
      path: "b.ts",
      changeKind: "created",
      source: "hook",
      toolName: "Write",
    });

    expect(seen).toHaveLength(2);
    const list = changes.listForBatch(batchId);
    expect(list).toHaveLength(2);
    expect(list[0]!.path).toBe("b.ts");
    expect(list[0]!.tool_name).toBe("Write");
    expect(list[1]!.path).toBe("a.ts");
  });

  test("listForTurn filters by turn_id", () => {
    const { batchId, turns, changes } = seed();
    const turn = turns.openTurn({ batchId, prompt: "p" });
    changes.record({
      batchId,
      turnId: turn.id,
      workItemId: null,
      path: "in-turn.ts",
      changeKind: "updated",
      source: "fs-watch",
    });
    changes.record({
      batchId,
      turnId: null,
      workItemId: null,
      path: "out-of-turn.ts",
      changeKind: "updated",
      source: "fs-watch",
    });

    const turnList = changes.listForTurn(turn.id);
    expect(turnList).toHaveLength(1);
    expect(turnList[0]!.path).toBe("in-turn.ts");
  });
});
