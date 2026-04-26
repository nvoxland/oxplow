import { describe, expect, test } from "bun:test";
import { computeBacklinks, type BacklinkContext, type BacklinkNoteEntry, type BacklinkWorkItemEntry, type BacklinkFindingEntry } from "./backlinksIndex.js";
import { fileRef, findingRef, noteRef, workItemRef } from "./pageRefs.js";

const note = (slug: string, body: string): BacklinkNoteEntry => ({ slug, title: slug, body });

const wi = (id: string, title: string, opts: Partial<BacklinkWorkItemEntry> = {}): BacklinkWorkItemEntry => ({
  id,
  title,
  description: opts.description ?? "",
  acceptance_criteria: opts.acceptance_criteria ?? null,
  touched_files: opts.touched_files ?? [],
});

const finding = (id: string, path: string, opts: Partial<BacklinkFindingEntry> = {}): BacklinkFindingEntry => ({
  id,
  path,
  startLine: opts.startLine ?? 1,
  endLine: opts.endLine ?? 10,
  kind: opts.kind ?? "complexity",
  metricValue: opts.metricValue ?? 0,
});

describe("computeBacklinks", () => {
  test("file ref → notes mentioning the path, work items touching it, findings located there", () => {
    const ctx: BacklinkContext = {
      notes: [
        note("uses-file", "See `src/a.ts` for details."),
        note("unrelated", "Nothing here."),
      ],
      workItems: [
        wi("wi-1", "Fix bug in a.ts", { touched_files: ["src/a.ts"] }),
        wi("wi-2", "Other work", { touched_files: ["src/b.ts"] }),
      ],
      findings: [finding("1", "src/a.ts"), finding("2", "src/b.ts")],
    };

    const result = computeBacklinks(fileRef("src/a.ts"), ctx);
    const ids = result.map((r) => r.ref.id);
    expect(ids).toContain("note:uses-file");
    expect(ids).toContain("wi:wi-1");
    expect(ids).toContain("finding:1");
    expect(ids).not.toContain("note:unrelated");
    expect(ids).not.toContain("wi:wi-2");
    expect(ids).not.toContain("finding:2");
  });

  test("work-item ref → notes mentioning [[wi-id]] and findings on touched files", () => {
    const ctx: BacklinkContext = {
      notes: [
        note("links-wi", "Tracking [[wi-42]] for this."),
        note("mentions-id", "Per wi-42, we should…"),
        note("unrelated", "Nothing here."),
      ],
      workItems: [
        wi("wi-42", "Refactor", { touched_files: ["src/a.ts"] }),
      ],
      findings: [finding("1", "src/a.ts"), finding("2", "src/b.ts")],
    };

    const result = computeBacklinks(workItemRef("wi-42"), ctx);
    const ids = result.map((r) => r.ref.id);
    expect(ids).toContain("note:links-wi");
    expect(ids).toContain("note:mentions-id");
    expect(ids).toContain("finding:1");
    expect(ids).not.toContain("finding:2");
    expect(ids).not.toContain("note:unrelated");
  });

  test("finding ref → work items touching the finding's file, notes mentioning the file or finding id", () => {
    const ctx: BacklinkContext = {
      notes: [
        note("mentions-file", "Look at src/a.ts."),
        note("mentions-id", "See finding:7 for context."),
        note("unrelated", "no refs here"),
      ],
      workItems: [
        wi("wi-1", "On a.ts", { touched_files: ["src/a.ts"] }),
        wi("wi-2", "On b.ts", { touched_files: ["src/b.ts"] }),
      ],
      findings: [finding("7", "src/a.ts")],
    };

    const result = computeBacklinks(findingRef("7"), ctx);
    const ids = result.map((r) => r.ref.id);
    expect(ids).toContain("wi:wi-1");
    expect(ids).toContain("note:mentions-file");
    expect(ids).toContain("note:mentions-id");
    expect(ids).not.toContain("wi:wi-2");
    expect(ids).not.toContain("note:unrelated");
  });

  test("note ref → work items, files, findings the note text mentions", () => {
    const ctx: BacklinkContext = {
      notes: [
        note(
          "rich",
          "Working on [[wi-99]] which touches `src/a.ts`. Related: finding:5.",
        ),
      ],
      workItems: [
        wi("wi-99", "Cool thing"),
        wi("wi-100", "Other"),
      ],
      findings: [finding("5", "src/a.ts"), finding("6", "src/b.ts")],
    };

    const result = computeBacklinks(noteRef("rich"), ctx);
    const ids = result.map((r) => r.ref.id);
    expect(ids).toContain("wi:wi-99");
    expect(ids).toContain("file:src/a.ts");
    expect(ids).toContain("finding:5");
    expect(ids).not.toContain("wi:wi-100");
    expect(ids).not.toContain("finding:6");
  });

  test("does not include the target itself in its own backlinks", () => {
    const ctx: BacklinkContext = {
      notes: [note("self", "I mention [[wi-1]] and `src/a.ts` and finding:1")],
      workItems: [wi("wi-1", "x", { touched_files: ["src/a.ts"] })],
      findings: [finding("1", "src/a.ts")],
    };

    const fileResult = computeBacklinks(fileRef("src/a.ts"), ctx);
    expect(fileResult.find((r) => r.ref.id === "file:src/a.ts")).toBeUndefined();

    const wiResult = computeBacklinks(workItemRef("wi-1"), ctx);
    expect(wiResult.find((r) => r.ref.id === "wi:wi-1")).toBeUndefined();
  });

  test("returns empty array for unknown kinds gracefully", () => {
    const ctx: BacklinkContext = { notes: [], workItems: [], findings: [] };
    const result = computeBacklinks({ id: "agent", kind: "agent", payload: null }, ctx);
    expect(result).toEqual([]);
  });

  test("dedupes entries when multiple signals point to the same target", () => {
    const ctx: BacklinkContext = {
      notes: [note("dup", "src/a.ts mentioned. src/a.ts again. `src/a.ts`.")],
      workItems: [],
      findings: [],
    };
    const result = computeBacklinks(fileRef("src/a.ts"), ctx);
    const noteHits = result.filter((r) => r.ref.id === "note:dup");
    expect(noteHits.length).toBe(1);
  });

  test("scans description and acceptance for note→workitem references", () => {
    const ctx: BacklinkContext = {
      notes: [note("n", "talks about wi-7 only")],
      workItems: [
        wi("wi-7", "Title", { description: "x", acceptance_criteria: "y" }),
      ],
      findings: [],
    };
    const result = computeBacklinks(workItemRef("wi-7"), ctx);
    expect(result.map((r) => r.ref.id)).toContain("note:n");
  });
});
