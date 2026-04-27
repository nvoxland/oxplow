import { describe, expect, test } from "bun:test";
import { filterSubsystemDocs } from "./SubsystemDocsPage.js";

describe("filterSubsystemDocs", () => {
  test("keeps only .md files and sorts alphabetically", () => {
    const rows = [
      { name: "theming.md", path: ".context/theming.md", kind: "file" as const },
      { name: "agent-model.md", path: ".context/agent-model.md", kind: "file" as const },
      { name: "scratch", path: ".context/scratch", kind: "directory" as const },
      { name: "notes.txt", path: ".context/notes.txt", kind: "file" as const },
      { name: "architecture.md", path: ".context/architecture.md", kind: "file" as const },
    ];
    expect(filterSubsystemDocs(rows)).toEqual([
      { name: "agent-model.md", path: ".context/agent-model.md" },
      { name: "architecture.md", path: ".context/architecture.md" },
      { name: "theming.md", path: ".context/theming.md" },
    ]);
  });

  test("empty input returns empty array", () => {
    expect(filterSubsystemDocs([])).toEqual([]);
  });

  test("ignores non-markdown files even when they share a prefix", () => {
    const rows = [
      { name: "data-model.md", path: ".context/data-model.md", kind: "file" as const },
      { name: "data-model.md.bak", path: ".context/data-model.md.bak", kind: "file" as const },
    ];
    expect(filterSubsystemDocs(rows).map((d) => d.name)).toEqual(["data-model.md"]);
  });
});
