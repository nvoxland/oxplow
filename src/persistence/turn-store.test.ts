import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "./thread-store.js";
import { SnapshotStore } from "./snapshot-store.js";
import { StreamStore } from "./stream-store.js";
import { TurnStore, type TurnChange } from "./turn-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-turns-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "Demo",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const threadStore = new ThreadStore(dir);
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  const turns = new TurnStore(dir);
  const snapshots = new SnapshotStore(dir);
  return { turns, threadId, snapshots, worktreePath: dir, streamId: stream.id };
}

describe("TurnStore", () => {
  test("openTurn + closeTurn roundtrip and emit change events", () => {
    const { turns, threadId } = seed();
    const changes: TurnChange[] = [];
    turns.subscribe((change) => changes.push(change));

    const open = turns.openTurn({ threadId, prompt: "Do the thing", sessionId: "sess-1" });
    expect(open.prompt).toBe("Do the thing");
    expect(open.ended_at).toBeNull();
    expect(turns.currentOpenTurn(threadId)?.id).toBe(open.id);

    const closed = turns.closeTurn(open.id, { answer: "Did the thing" });
    expect(closed?.answer).toBe("Did the thing");
    expect(closed?.ended_at).not.toBeNull();
    expect(turns.currentOpenTurn(threadId)).toBeNull();

    expect(changes.map((c) => c.kind)).toEqual(["opened", "closed"]);
  });

  test("setStartSnapshot and setEndSnapshot populate the turn", () => {
    const { turns, threadId, snapshots, worktreePath, streamId } = seed();
    writeFileSync(join(worktreePath, "a.txt"), "v1");
    const startSnap = snapshots.flushSnapshot({
      source: "turn-start",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    writeFileSync(join(worktreePath, "a.txt"), "v2");
    const endSnap = snapshots.flushSnapshot({
      source: "turn-end",
      streamId,
      worktreePath,
      dirtyPaths: ["a.txt"],
    });
    const open = turns.openTurn({ threadId, prompt: "P" });
    turns.setStartSnapshot(open.id, startSnap.id);
    turns.setEndSnapshot(open.id, endSnap.id);
    const read = turns.getById(open.id);
    expect(read?.start_snapshot_id).toBe(startSnap.id);
    expect(read?.end_snapshot_id).toBe(endSnap.id);
  });

  test("closeTurn on an already-closed turn returns existing without re-emitting", () => {
    const { turns, threadId } = seed();
    const open = turns.openTurn({ threadId, prompt: "P" });
    turns.closeTurn(open.id, { answer: "A" });

    const events: TurnChange[] = [];
    turns.subscribe((c) => events.push(c));
    const again = turns.closeTurn(open.id, { answer: "B" });
    expect(again?.answer).toBe("A");
    expect(events).toHaveLength(0);
  });

  test("listForThread returns newest-first", () => {
    const { turns, threadId } = seed();
    const a = turns.openTurn({ threadId, prompt: "first" });
    turns.closeTurn(a.id, { answer: "a-done" });
    const b = turns.openTurn({ threadId, prompt: "second" });
    const list = turns.listForThread(threadId);
    expect(list).toHaveLength(2);
    expect(list[0]!.id).toBe(b.id);
    expect(list[1]!.id).toBe(a.id);
  });

  test("prompt longer than cap is truncated with ellipsis", () => {
    const { turns, threadId } = seed();
    const huge = "x".repeat(25_000);
    const open = turns.openTurn({ threadId, prompt: huge });
    expect(open.prompt.length).toBe(20_000);
    expect(open.prompt.endsWith("…")).toBe(true);
  });

  test("getLastClosedTurnCacheRead returns null when no turn has closed", () => {
    const { turns, threadId } = seed();
    expect(turns.getLastClosedTurnCacheRead(threadId)).toBeNull();
    // Open-but-not-closed turns don't count either.
    turns.openTurn({ threadId, prompt: "still running" });
    expect(turns.getLastClosedTurnCacheRead(threadId)).toBeNull();
  });

  test("getLastClosedTurnCacheRead returns the cache_read_input_tokens of the most recent closed turn", () => {
    const { turns, threadId } = seed();
    const a = turns.openTurn({ threadId, prompt: "first" });
    turns.setTurnUsage(a.id, { inputTokens: 10, outputTokens: 5, cacheReadInputTokens: 1_200_000 });
    turns.closeTurn(a.id, { answer: "ok" });

    expect(turns.getLastClosedTurnCacheRead(threadId)).toBe(1_200_000);

    // A newer closed turn supersedes it.
    const b = turns.openTurn({ threadId, prompt: "second" });
    turns.setTurnUsage(b.id, { inputTokens: 20, outputTokens: 10, cacheReadInputTokens: 5_000_000 });
    turns.closeTurn(b.id, { answer: "ok" });
    expect(turns.getLastClosedTurnCacheRead(threadId)).toBe(5_000_000);
  });

  test("getLastClosedTurnCacheRead returns null when the latest closed turn has no usage set", () => {
    const { turns, threadId } = seed();
    const a = turns.openTurn({ threadId, prompt: "no usage" });
    turns.closeTurn(a.id, { answer: "ok" });
    // No setTurnUsage call — cache_read_input_tokens stays null.
    expect(turns.getLastClosedTurnCacheRead(threadId)).toBeNull();
  });

  test("getCumulativeCacheRead returns 0 for a thread with no closed turns", () => {
    const { turns, threadId } = seed();
    expect(turns.getCumulativeCacheRead(threadId)).toBe(0);
    turns.openTurn({ threadId, prompt: "still running" });
    expect(turns.getCumulativeCacheRead(threadId)).toBe(0);
  });

  test("getCumulativeCacheRead sums cache_read_input_tokens across closed turns", () => {
    const { turns, threadId } = seed();
    const a = turns.openTurn({ threadId, prompt: "a" });
    turns.setTurnUsage(a.id, { inputTokens: 1, outputTokens: 1, cacheReadInputTokens: 1_000_000 });
    turns.closeTurn(a.id, { answer: "a" });

    const b = turns.openTurn({ threadId, prompt: "b" });
    turns.setTurnUsage(b.id, { inputTokens: 1, outputTokens: 1, cacheReadInputTokens: 3_500_000 });
    turns.closeTurn(b.id, { answer: "b" });

    // Null-usage turn contributes 0 (COALESCE).
    const c = turns.openTurn({ threadId, prompt: "c" });
    turns.closeTurn(c.id, { answer: "c" });

    expect(turns.getCumulativeCacheRead(threadId)).toBe(4_500_000);
  });
});
