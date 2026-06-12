import { describe, expect, test } from "bun:test";

import { wikiRowTooltip } from "./wikiRowLabel.js";

describe("wikiRowTooltip", () => {
  test("interpolates the real referenced-file count", () => {
    expect(
      wikiRowTooltip({ title: "Architecture Overview", slug: "architecture-overview", file_refs: ["a.rs", "b.rs"] }),
    ).toBe(
      "Architecture Overview\narchitecture-overview — 2 referenced files\nDrag onto agent to add to context",
    );
  });

  test("singular form for one ref", () => {
    expect(wikiRowTooltip({ title: "T", slug: "t", file_refs: ["a.rs"] })).toContain(
      "t — 1 referenced file\n",
    );
  });

  test("omits the clause when the count is absent — never 'undefined'", () => {
    const label = wikiRowTooltip({ title: "T", slug: "t" });
    expect(label).toBe("T\nt\nDrag onto agent to add to context");
    expect(label).not.toContain("undefined");
  });

  test("zero refs still reads as a count, not omitted", () => {
    expect(wikiRowTooltip({ title: "T", slug: "t", file_refs: [] })).toContain(
      "t — 0 referenced files",
    );
  });
});
