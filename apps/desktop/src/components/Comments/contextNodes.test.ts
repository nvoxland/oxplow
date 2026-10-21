import { describe, expect, test } from "bun:test";

import { commitUri, fileUri, pageUri, workItemUri } from "../../uri/uri.js";
import { collectContextChain, contextNodeProps, nearestContextNode } from "./contextNodes.js";

const GIT = pageUri("git-dashboard");
const COMMIT = commitUri("abc1234");
const FILE = fileUri("src/app.rs");
const X = fileUri("x.rs");
const TASK = workItemUri("oxplow", "tsk42");

/// Build a detached DOM tree from nested URI declarations (outermost
/// first) and return the innermost element so tests can walk up from a
/// realistic selection anchor. `null` entries are plain wrappers carrying
/// no `data-ref`.
function nest(levels: (string | null)[]): HTMLElement {
  let current: HTMLElement | null = null;
  let root: HTMLElement | null = null;
  for (const level of levels) {
    const el = document.createElement("div");
    if (level) {
      el.setAttribute("data-ref", level);
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
    const inner = nest([GIT, COMMIT, FILE]);
    expect(collectContextChain(inner)).toEqual([FILE, COMMIT, GIT]);
  });

  test("starts from a text node by climbing to its parent", () => {
    const inner = nest([TASK]);
    const textNode = inner.firstChild!; // the text node
    expect(textNode.nodeType).toBe(3);
    expect(collectContextChain(textNode)).toEqual([TASK]);
  });

  test("skips wrappers with no data-ref attribute", () => {
    const inner = nest([TASK, null, null]);
    expect(collectContextChain(inner)).toEqual([TASK]);
  });

  test("collapses adjacent duplicate declarations", () => {
    // The same URI on a wrapper and its child collapses to one entry…
    const inner = nest([COMMIT, X, X]);
    expect(collectContextChain(inner)).toEqual([X, COMMIT]);
  });

  test("does NOT collapse a duplicate that reappears non-adjacently", () => {
    const inner = nest([X, COMMIT, X]);
    expect(collectContextChain(inner)).toEqual([X, COMMIT, X]);
  });

  test("ignores an element with an empty data-ref", () => {
    const orphan = document.createElement("div");
    orphan.setAttribute("data-ref", "");
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
    const inner = nest([COMMIT, X]);
    expect(nearestContextNode(inner)).toBe(X);
  });

  test("is null when nothing declares identity", () => {
    expect(nearestContextNode(nest([null]))).toBeNull();
  });
});

describe("contextNodeProps", () => {
  test("produces the data-ref attribute", () => {
    expect(contextNodeProps(TASK)).toEqual({ "data-ref": TASK });
  });
});
