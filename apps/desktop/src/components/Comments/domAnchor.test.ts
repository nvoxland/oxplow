import { describe, expect, test } from "bun:test";

import { refFromHref, refsInRange, textContentRange } from "./domAnchor.js";

/// Build a detached element with the given innerHTML for range tests.
function frag(html: string): HTMLElement {
  const el = document.createElement("div");
  el.innerHTML = html;
  return el;
}

describe("textContentRange", () => {
  test("locates a span spanning multiple text nodes", () => {
    // textContent is "Fix the flaky test" across two spans.
    const el = frag("<span>Fix the </span><span>flaky test</span>");
    expect(el.textContent).toBe("Fix the flaky test");
    const start = el.textContent!.indexOf("flaky");
    const range = textContentRange(el, start, "flaky".length);
    expect(range).not.toBeNull();
    expect(range!.toString()).toBe("flaky");
  });

  test("locates a span within a single text node", () => {
    const el = frag("<div>the quick brown fox</div>");
    const start = el.textContent!.indexOf("brown");
    const range = textContentRange(el, start, "brown".length);
    expect(range!.toString()).toBe("brown");
  });

  test("returns null for an out-of-bounds span", () => {
    const el = frag("<div>short</div>");
    expect(textContentRange(el, 100, 5)).toBeNull();
    expect(textContentRange(el, 0, 0)).toBeNull();
  });
});

describe("refFromHref", () => {
  test("maps typed hrefs to canonical refs", () => {
    expect(refFromHref("file:src/app.rs")).toEqual({ kind: "file", id: "src/app.rs" });
    expect(refFromHref("dir:src/components")).toEqual({
      kind: "directory",
      id: "src/components",
    });
    expect(refFromHref("gitcommit:abc1234")).toEqual({ kind: "git-commit", id: "abc1234" });
  });

  test("returns null for external links and anchors", () => {
    expect(refFromHref("https://example.com")).toBeNull();
    expect(refFromHref("#section")).toBeNull();
    expect(refFromHref("")).toBeNull();
  });
});

describe("refsInRange", () => {
  test("collects deduped typed refs from links inside the selection", () => {
    const el = frag(
      'see <a href="file:src/app.rs">app</a> and <a href="file:src/app.rs">app again</a> and <a href="gitcommit:deadbee">that commit</a>',
    );
    document.body.appendChild(el);
    const range = document.createRange();
    range.selectNodeContents(el);
    const refs = refsInRange(range);
    expect(refs).toEqual([
      { kind: "file", id: "src/app.rs" },
      { kind: "git-commit", id: "deadbee" },
    ]);
    el.remove();
  });

  test("empty when the selection has no links", () => {
    const el = frag("<div>just plain text</div>");
    document.body.appendChild(el);
    const range = document.createRange();
    range.selectNodeContents(el);
    expect(refsInRange(range)).toEqual([]);
    el.remove();
  });
});
