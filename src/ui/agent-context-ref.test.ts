import { describe, expect, test } from "bun:test";
import { formatContextMention } from "./agent-context-ref.js";

describe("formatContextMention", () => {
  test("file → @<path> with trailing space", () => {
    expect(formatContextMention({ kind: "file", path: "src/foo.ts" })).toBe("@src/foo.ts ");
  });

  test("file with nested path", () => {
    expect(formatContextMention({ kind: "file", path: "src/ui/components/Notes/NotesPane.tsx" }))
      .toBe("@src/ui/components/Notes/NotesPane.tsx ");
  });

  test("note → @.oxplow/notes/<slug>.md with trailing space", () => {
    expect(formatContextMention({ kind: "note", slug: "auth-flow" })).toBe("@.oxplow/notes/auth-flow.md ");
  });

  test("work-item → bracketed reference with id, title, status, trailing space", () => {
    expect(formatContextMention({
      kind: "work-item", itemId: "wi-abc123", title: "Add to agent context", status: "in_progress",
    })).toBe('[oxplow work-item wi-abc123: "Add to agent context" (in_progress)] ');
  });

  test("work-item collapses whitespace in title", () => {
    expect(formatContextMention({
      kind: "work-item", itemId: "wi-1", title: "Multi\nline\ttitle  here", status: "ready",
    })).toBe('[oxplow work-item wi-1: "Multi line title here" (ready)] ');
  });

  test("work-item leaves quotes in title untouched (plain text reference)", () => {
    expect(formatContextMention({
      kind: "work-item", itemId: "wi-1", title: 'Fix "broken" thing', status: "ready",
    })).toBe('[oxplow work-item wi-1: "Fix "broken" thing" (ready)] ');
  });

  test("every output ends with a space so the user can keep typing", () => {
    expect(formatContextMention({ kind: "file", path: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({ kind: "note", slug: "x" }).endsWith(" ")).toBe(true);
    expect(formatContextMention({
      kind: "work-item", itemId: "x", title: "x", status: "x",
    }).endsWith(" ")).toBe(true);
  });
});
