import { describe, expect, test } from "bun:test";
import {
  DEFAULT_TERMINAL_ID,
  addTerminal,
  closeTerminal,
  commentTargetFor,
  defaultTerminalList,
  normalizeTerminalList,
  paneTargetFor,
  renameTerminal,
} from "./terminalTabs.js";

describe("defaultTerminalList", () => {
  test("seeds one default terminal", () => {
    expect(defaultTerminalList()).toEqual([{ id: DEFAULT_TERMINAL_ID, title: "Terminal 1" }]);
  });
});

describe("paneTargetFor / commentTargetFor", () => {
  test("default terminal keeps the legacy bare shell + stream targets", () => {
    expect(paneTargetFor(DEFAULT_TERMINAL_ID)).toBe("shell");
    expect(commentTargetFor("s-1", DEFAULT_TERMINAL_ID)).toBe("s-1");
  });

  test("additional terminals get id-scoped targets", () => {
    expect(paneTargetFor("t2")).toBe("shell:t2");
    expect(commentTargetFor("s-1", "t2")).toBe("s-1:t2");
  });
});

describe("addTerminal", () => {
  test("appends an auto-numbered terminal and activates it", () => {
    const { list, activeId } = addTerminal(defaultTerminalList(), "t2");
    expect(list).toEqual([
      { id: DEFAULT_TERMINAL_ID, title: "Terminal 1" },
      { id: "t2", title: "Terminal 2" },
    ]);
    expect(activeId).toBe("t2");
  });

  test("does not collide with an existing numbered title after renames", () => {
    // "Terminal 2" already exists (a renamed tab) — the next number must
    // jump past it rather than duplicate.
    const list = [
      { id: DEFAULT_TERMINAL_ID, title: "logs" },
      { id: "t2", title: "Terminal 2" },
    ];
    const { list: next } = addTerminal(list, "t3");
    expect(next[2].title).toBe("Terminal 3");
  });
});

describe("closeTerminal", () => {
  const list = [
    { id: "a", title: "Terminal 1" },
    { id: "b", title: "Terminal 2" },
    { id: "c", title: "Terminal 3" },
  ];

  test("closing the active terminal activates the previous sibling", () => {
    const res = closeTerminal(list, "b", "b");
    expect(res.list.map((t) => t.id)).toEqual(["a", "c"]);
    expect(res.activeId).toBe("a");
  });

  test("closing the first active terminal falls forward to the next", () => {
    const res = closeTerminal(list, "a", "a");
    expect(res.list.map((t) => t.id)).toEqual(["b", "c"]);
    expect(res.activeId).toBe("b");
  });

  test("closing a non-active terminal leaves the active unchanged", () => {
    const res = closeTerminal(list, "a", "c");
    expect(res.activeId).toBe("a");
    expect(res.list.map((t) => t.id)).toEqual(["a", "b"]);
  });

  test("closing the last terminal re-seeds a fresh default", () => {
    const res = closeTerminal([{ id: "x", title: "only" }], "x", "x");
    expect(res.list).toEqual(defaultTerminalList());
    expect(res.activeId).toBe(DEFAULT_TERMINAL_ID);
  });

  test("closing an unknown id is a no-op", () => {
    const res = closeTerminal(list, "b", "zzz");
    expect(res.list).toBe(list);
    expect(res.activeId).toBe("b");
  });
});

describe("renameTerminal", () => {
  test("updates the matching terminal's title (trimmed)", () => {
    const list = [{ id: "a", title: "Terminal 1" }];
    expect(renameTerminal(list, "a", "  logs  ")).toEqual([{ id: "a", title: "logs" }]);
  });

  test("ignores an empty title", () => {
    const list = [{ id: "a", title: "Terminal 1" }];
    expect(renameTerminal(list, "a", "   ")).toEqual(list);
  });
});

describe("normalizeTerminalList", () => {
  test("keeps valid entries and drops malformed / duplicate / empty", () => {
    const raw = [
      { id: "a", title: "Terminal 1" },
      { id: "a", title: "dup id" },
      { id: "b", title: "" },
      { id: 3, title: "bad id" },
      null,
      { id: "c", title: "logs" },
    ];
    expect(normalizeTerminalList(raw)).toEqual([
      { id: "a", title: "Terminal 1" },
      { id: "c", title: "logs" },
    ]);
  });

  test("falls back to a default when nothing survives", () => {
    expect(normalizeTerminalList("nonsense")).toEqual(defaultTerminalList());
    expect(normalizeTerminalList([])).toEqual(defaultTerminalList());
  });
});
