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

  test("a work item → bracketed reference with its ref, title, state, trailing space", () => {
    expect(formatContextMention({
      kind: "work_item", ref: "work_item:issues:ENG-12", title: "Add to agent context", state: "in_progress",
    })).toBe('[oxplow work_item:issues:ENG-12: "Add to agent context" (in_progress)] ');
  });

  test("a work item's title collapses whitespace", () => {
    expect(formatContextMention({
      kind: "work_item", ref: "work_item:oxplow:tsk1", title: "Multi\nline\ttitle  here", state: "todo",
    })).toBe('[oxplow work_item:oxplow:tsk1: "Multi line title here" (todo)] ');
  });

  test("a work item's title keeps its quotes (plain text reference)", () => {
    expect(formatContextMention({
      kind: "work_item", ref: "work_item:oxplow:tsk1", title: 'Fix "broken" thing', state: "todo",
    })).toBe('[oxplow work_item:oxplow:tsk1: "Fix "broken" thing" (todo)] ');
  });

  test("every output ends with a space so the user can keep typing", () => {
    expect(formatContextMention({ kind: "file", path: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({ kind: "wiki", slug: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({
      kind: "work_item", ref: "work_item:oxplow:tsk1", title: "x", state: "x",
    }).endsWith(" ")).toBe(true);
  });

  test("lens → bracketed reference with id and non-default params", () => {
    expect(formatContextMention({ kind: "lens", lensId: "review/waiting", params: { stream: 1, kind: "primary" } }))
      .toBe('[oxplow lens review/waiting kind="primary" stream=1] ');
    expect(formatContextMention({ kind: "lens", lensId: "review/waiting", params: {} }))
      .toBe("[oxplow lens review/waiting] ");
  });
});
