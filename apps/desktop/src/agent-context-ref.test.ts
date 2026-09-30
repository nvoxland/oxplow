import { describe, expect, test } from "bun:test";
import { askAboutSelection, formatContextMention } from "./agent-context-ref.js";

describe("askAboutSelection", () => {
  test("the selected lines, dropping a trailing line the selection only touches", () => {
    expect(askAboutSelection("a.rs", { startLineNumber: 4, endLineNumber: 6, endColumn: 9 })).toBe("[oxplow ref file:a.rs#L4-6] ");
    expect(askAboutSelection("a.rs", { startLineNumber: 4, endLineNumber: 7, endColumn: 1 })).toBe("[oxplow ref file:a.rs#L4-6] ");
    expect(askAboutSelection("a.rs", { startLineNumber: 2, endLineNumber: 2, endColumn: 5 }, "git:abc")).toBe("[oxplow ref file:a.rs@git:abc#L2] ");
  });
});

describe("formatContextMention", () => {
  test("ref → [oxplow ref <ref>] with trailing space: Ask About This", () => {
    expect(formatContextMention({ kind: "ref", ref: "commit:abc123" })).toBe("[oxplow ref commit:abc123] ");
  });

  test("file → @<path> with trailing space", () => {
    expect(formatContextMention({ kind: "file", path: "src/foo.ts" })).toBe("@src/foo.ts ");
  });

  test("file with nested path", () => {
    expect(formatContextMention({ kind: "file", path: "src/ui/components/Wiki/WikiPane.tsx" }))
      .toBe("@src/ui/components/Wiki/WikiPane.tsx ");
  });

  test("wiki → @.oxplow/wiki/<slug>.md with trailing space", () => {
    expect(formatContextMention({ kind: "wiki", slug: "auth-flow" })).toBe("@.oxplow/wiki/auth-flow.md ");
  });

  test("tasks → bracketed reference with id, title, status, trailing space", () => {
    expect(formatContextMention({
      kind: "task", itemId: "wi-abc123", title: "Add to agent context", status: "in_progress",
    })).toBe('[oxplow task wi-abc123: "Add to agent context" (in_progress)] ');
  });

  test("tasks collapses whitespace in title", () => {
    expect(formatContextMention({
      kind: "task", itemId: "1", title: "Multi\nline\ttitle  here", status: "ready",
    })).toBe('[oxplow task 1: "Multi line title here" (ready)] ');
  });

  test("tasks leaves quotes in title untouched (plain text reference)", () => {
    expect(formatContextMention({
      kind: "task", itemId: "1", title: 'Fix "broken" thing', status: "ready",
    })).toBe('[oxplow task 1: "Fix "broken" thing" (ready)] ');
  });

  test("every output ends with a space so the user can keep typing", () => {
    expect(formatContextMention({ kind: "file", path: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({ kind: "wiki", slug: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({
      kind: "task", itemId: "x", title: "x", status: "x",
    }).endsWith(" ")).toBe(true);
  });

  test("lens → bracketed reference with id and non-default params", () => {
    expect(formatContextMention({ kind: "lens", lensId: "review/waiting", params: { stream: 1, kind: "primary" } }))
      .toBe('[oxplow lens review/waiting kind="primary" stream=1] ');
    expect(formatContextMention({ kind: "lens", lensId: "review/waiting", params: {} }))
      .toBe("[oxplow lens review/waiting] ");
  });
});
