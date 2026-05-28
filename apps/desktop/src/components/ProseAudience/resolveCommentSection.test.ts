import { describe, expect, test } from "bun:test";
import { resolveCommentSection } from "./resolveCommentSection.js";

describe("resolveCommentSection", () => {
  const headings = ["storage", "why"];

  test("quote present in the displayed body → precise quote mode", () => {
    const r = resolveCommentSection(
      { quote: "canonical text", sectionAnchor: "storage" },
      "## Storage\n\ncanonical text here",
      headings,
    );
    expect(r).toEqual({ mode: "quote" });
  });

  test("quote absent but section anchor present → section mode", () => {
    const r = resolveCommentSection(
      { quote: "developer-only phrasing", sectionAnchor: "storage" },
      "## Storage\n\nthe gist", // executive variant — different prose, same heading
      headings,
    );
    expect(r).toEqual({ mode: "section", slug: "storage" });
  });

  test("quote absent and section missing in this variant → orphaned", () => {
    const r = resolveCommentSection(
      { quote: "gone", sectionAnchor: "removed-section" },
      "## Storage\n\nthe gist",
      headings,
    );
    expect(r).toEqual({ mode: "orphaned" });
  });

  test("no section anchor and no quote match → orphaned", () => {
    const r = resolveCommentSection({ quote: "gone", sectionAnchor: null }, "## Storage", headings);
    expect(r).toEqual({ mode: "orphaned" });
  });
});
