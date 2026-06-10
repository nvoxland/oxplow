import { beforeEach, describe, expect, test } from "bun:test";

import {
  DIFF_SPECS_STORAGE_KEY,
  THREAD_HISTORY_STORAGE_KEY,
  THREAD_TABS_STORAGE_KEY,
  readPersistedDiffSpecs,
  readPersistedThreadPageHistory,
  readPersistedThreadPageTabs,
  writePersistedThreadPageTabs,
  type ThreadHistory,
} from "./pageTabsPersistence.js";
import type { TabRef } from "./tabState.js";

const FILE_A: TabRef = { id: "file:src/a.ts", kind: "file", payload: { path: "src/a.ts" } };
const FILE_B: TabRef = { id: "file:src/b.ts", kind: "file", payload: { path: "src/b.ts" } };

beforeEach(() => {
  window.localStorage.clear();
});

describe("readPersistedThreadPageTabs", () => {
  test("absent key restores to an empty record", () => {
    expect(readPersistedThreadPageTabs()).toEqual({});
  });

  test("corrupt JSON restores to an empty record instead of throwing", () => {
    window.localStorage.setItem(THREAD_TABS_STORAGE_KEY, "{not json!");
    expect(readPersistedThreadPageTabs()).toEqual({});
  });

  test("non-object blob restores to an empty record", () => {
    window.localStorage.setItem(THREAD_TABS_STORAGE_KEY, JSON.stringify([1, 2, 3]));
    expect(readPersistedThreadPageTabs()).toEqual({});
  });

  test("malformed entries are dropped, valid refs survive", () => {
    window.localStorage.setItem(
      THREAD_TABS_STORAGE_KEY,
      JSON.stringify({
        thr1: [FILE_A, null, 42, { id: 7 }, { kind: "file" }],
        thr2: "not-an-array",
      }),
    );
    expect(readPersistedThreadPageTabs()).toEqual({ thr1: [FILE_A] });
  });

  test("duplicate tab ids self-heal on read, first occurrence wins", () => {
    window.localStorage.setItem(
      THREAD_TABS_STORAGE_KEY,
      JSON.stringify({ thr1: [FILE_A, FILE_B, FILE_A] }),
    );
    expect(readPersistedThreadPageTabs()).toEqual({ thr1: [FILE_A, FILE_B] });
  });

  test("round-trips through write, dropping empty thread entries", () => {
    writePersistedThreadPageTabs({ thr1: [FILE_A], thr2: [] });
    expect(readPersistedThreadPageTabs()).toEqual({ thr1: [FILE_A] });
  });
});

describe("readPersistedThreadPageHistory", () => {
  test("corrupt JSON restores to an empty record", () => {
    window.localStorage.setItem(THREAD_HISTORY_STORAGE_KEY, "][");
    expect(readPersistedThreadPageHistory()).toEqual({});
  });

  test("legacy shape (bare TabRef[] stacks) coerces to HistoryFrame[]", () => {
    window.localStorage.setItem(
      THREAD_HISTORY_STORAGE_KEY,
      JSON.stringify({
        thr1: { "file:src/b.ts": { back: [FILE_A], forward: [] } },
      }),
    );
    const restored = readPersistedThreadPageHistory();
    expect(restored.thr1["file:src/b.ts"]).toEqual({
      back: [{ ref: FILE_A, siblings: null }],
      forward: [],
      siblings: null,
    });
  });

  test("current shape round-trips unchanged", () => {
    const history: ThreadHistory = {
      thr1: {
        "file:src/b.ts": {
          back: [{ ref: FILE_A, siblings: null }],
          forward: [{ ref: FILE_B, siblings: null }],
          siblings: { entries: [{ ref: FILE_A, label: "a" }], index: 0 },
        },
      },
    };
    window.localStorage.setItem(THREAD_HISTORY_STORAGE_KEY, JSON.stringify(history));
    expect(readPersistedThreadPageHistory()).toEqual(history);
  });

  test("non-object per-tab entries are dropped", () => {
    window.localStorage.setItem(
      THREAD_HISTORY_STORAGE_KEY,
      JSON.stringify({ thr1: { tab: 17 }, thr2: 9 }),
    );
    const restored = readPersistedThreadPageHistory();
    expect(restored.thr1).toEqual({});
    expect(restored.thr2).toBeUndefined();
  });
});

describe("readPersistedDiffSpecs", () => {
  test("corrupt JSON restores to an empty list", () => {
    window.localStorage.setItem(DIFF_SPECS_STORAGE_KEY, "{{{");
    expect(readPersistedDiffSpecs()).toEqual([]);
  });

  test("legacy specs (leftRef + rightKind) coerce to versioned shape", () => {
    window.localStorage.setItem(
      DIFF_SPECS_STORAGE_KEY,
      JSON.stringify([
        { id: "d1", spec: { path: "src/a.ts", leftRef: "abc123", rightKind: "working", baseLabel: "HEAD" } },
        { id: "d2", spec: { path: "src/b.ts", rightKind: { ref: "def456" } } },
      ]),
    );
    const restored = readPersistedDiffSpecs();
    expect(restored).toHaveLength(2);
    expect(restored[0].spec.leftVersion).toEqual({ kind: "ref", ref: "abc123" });
    expect(restored[0].spec.rightVersion).toEqual({ kind: "disk" });
    expect(restored[1].spec.leftVersion).toEqual({ kind: "disk" });
    expect(restored[1].spec.rightVersion).toEqual({ kind: "ref", ref: "def456" });
  });

  test("entries without a string id or object spec are dropped", () => {
    window.localStorage.setItem(
      DIFF_SPECS_STORAGE_KEY,
      JSON.stringify([{ id: 5, spec: {} }, { id: "ok" }, null, { id: "d", spec: { path: "p" } }]),
    );
    const restored = readPersistedDiffSpecs();
    expect(restored).toHaveLength(1);
    expect(restored[0].id).toBe("d");
  });
});
