import { describe, expect, test } from "bun:test";

import { DocumentSyncTracker } from "./lsp-document-sync.js";

function collector() {
  const sent: { path: string; text: string; version: number }[] = [];
  const tracker = new DocumentSyncTracker(
    (path, text, version) => sent.push({ path, text, version }),
    5,
  );
  return { sent, tracker };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("DocumentSyncTracker", () => {
  test("open returns version 1 and changed sends debounced didChange with version 2", async () => {
    const { sent, tracker } = collector();
    expect(tracker.open("a.ts", "one")).toBe(1);
    tracker.changed("a.ts", "two");
    expect(sent).toHaveLength(0); // debounced, not immediate
    await sleep(20);
    expect(sent).toEqual([{ path: "a.ts", text: "two", version: 2 }]);
  });

  test("rapid changes coalesce into one send with the last text", async () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.changed("a.ts", "v1");
    tracker.changed("a.ts", "v2");
    tracker.changed("a.ts", "v3");
    await sleep(20);
    expect(sent).toEqual([{ path: "a.ts", text: "v3", version: 2 }]);
  });

  test("versions are monotonic across multiple flushes", async () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.changed("a.ts", "v1");
    tracker.flush("a.ts");
    tracker.changed("a.ts", "v2");
    tracker.flush("a.ts");
    expect(sent.map((s) => s.version)).toEqual([2, 3]);
  });

  test("flush is a no-op when nothing is pending or text reverted to last sent", () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.flush("a.ts");
    tracker.changed("a.ts", "v1");
    tracker.changed("a.ts", "v0"); // reverted before the debounce fired
    tracker.flush("a.ts");
    expect(sent).toHaveLength(0);
  });

  test("changed on an untracked path is ignored", async () => {
    const { sent, tracker } = collector();
    tracker.changed("nope.ts", "text");
    await sleep(20);
    expect(sent).toHaveLength(0);
  });

  test("close cancels pending sends and stops tracking", async () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.changed("a.ts", "v1");
    tracker.close("a.ts");
    await sleep(20);
    expect(sent).toHaveLength(0);
    expect(tracker.isTracking("a.ts")).toBe(false);
  });

  test("reopen restarts at version 1", async () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.changed("a.ts", "v1");
    tracker.flush("a.ts");
    tracker.close("a.ts");
    expect(tracker.open("a.ts", "fresh")).toBe(1);
    tracker.changed("a.ts", "fresh2");
    tracker.flush("a.ts");
    expect(sent.map((s) => s.version)).toEqual([2, 2]);
  });

  test("reset drops every doc", () => {
    const { sent, tracker } = collector();
    tracker.open("a.ts", "v0");
    tracker.open("b.ts", "v0");
    tracker.changed("a.ts", "v1");
    tracker.reset();
    tracker.flush("a.ts");
    expect(sent).toHaveLength(0);
    expect(tracker.isTracking("b.ts")).toBe(false);
  });
});
