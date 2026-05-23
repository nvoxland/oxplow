import { describe, expect, test } from "bun:test";

import { collectContextChain, contextNodeProps, nearestContextNode } from "./contextNodes.js";

/// Build a detached DOM tree from nested `[kind, id]` declarations
/// (outermost first) and return the innermost element so tests can walk
/// up from a realistic selection anchor. `null` entries are plain
/// wrappers carrying no `data-ref-*`.
function nest(levels: ([string, string] | null)[]): HTMLElement {
  let current: HTMLElement | null = null;
  let root: HTMLElement | null = null;
  for (const level of levels) {
    const el = document.createElement("div");
    if (level) {
      el.setAttribute("data-ref-kind", level[0]);
      el.setAttribute("data-ref-id", level[1]);
    }
    if (current) current.appendChild(el);
    else root = el;
    current = el;
  }
  // Anchor a text node inside the innermost element — that's what a real
  // selection hands back.
  const text = document.createTextNode("selected words");
  current!.appendChild(text);
  void root;
  return current!;
}

describe("collectContextChain", () => {
  test("collects nested context nodes innermost→outermost", () => {
    const inner = nest([
      ["git-dashboard", "git-dashboard"],
      ["git-commit", "abc1234"],
      ["file", "src/app.rs"],
    ]);
    expect(collectContextChain(inner)).toEqual([
      { kind: "file", id: "src/app.rs" },
      { kind: "git-commit", id: "abc1234" },
      { kind: "git-dashboard", id: "git-dashboard" },
    ]);
  });

  test("starts from a text node by climbing to its parent", () => {
    const inner = nest([["task", "42"]]);
    const textNode = inner.firstChild!; // the text node
    expect(textNode.nodeType).toBe(3);
    expect(collectContextChain(textNode)).toEqual([{ kind: "task", id: "42" }]);
  });

  test("skips wrappers with no data-ref attributes", () => {
    const inner = nest([["task", "42"], null, null]);
    expect(collectContextChain(inner)).toEqual([{ kind: "task", id: "42" }]);
  });

  test("collapses adjacent duplicate declarations", () => {
    // Same (kind,id) on a wrapper and its child collapses to one entry…
    const inner = nest([
      ["git-commit", "abc"],
      ["file", "x.rs"],
      ["file", "x.rs"],
    ]);
    expect(collectContextChain(inner)).toEqual([
      { kind: "file", id: "x.rs" },
      { kind: "git-commit", id: "abc" },
    ]);
  });

  test("does NOT collapse a duplicate that reappears non-adjacently", () => {
    const inner = nest([
      ["file", "x.rs"],
      ["git-commit", "abc"],
      ["file", "x.rs"],
    ]);
    expect(collectContextChain(inner)).toEqual([
      { kind: "file", id: "x.rs" },
      { kind: "git-commit", id: "abc" },
      { kind: "file", id: "x.rs" },
    ]);
  });

  test("ignores an element missing one of the two attributes", () => {
    const orphan = document.createElement("div");
    orphan.setAttribute("data-ref-kind", "file"); // no id
    expect(collectContextChain(orphan)).toEqual([]);
  });

  test("returns [] for a region with no context nodes", () => {
    const plain = nest([null, null]);
    expect(collectContextChain(plain)).toEqual([]);
  });

  test("returns [] for a null node", () => {
    expect(collectContextChain(null)).toEqual([]);
  });
});

describe("nearestContextNode", () => {
  test("is the innermost context node", () => {
    const inner = nest([["git-commit", "abc"], ["file", "x.rs"]]);
    expect(nearestContextNode(inner)).toEqual({ kind: "file", id: "x.rs" });
  });

  test("is null when nothing declares identity", () => {
    expect(nearestContextNode(nest([null]))).toBeNull();
  });
});

describe("contextNodeProps", () => {
  test("produces the data-ref-* attribute pair", () => {
    expect(contextNodeProps("task", "42")).toEqual({
      "data-ref-kind": "task",
      "data-ref-id": "42",
    });
  });
});
