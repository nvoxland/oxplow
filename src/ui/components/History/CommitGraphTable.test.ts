import { describe, expect, test } from "bun:test";
import type { GitLogResult } from "../../api.js";
import { formatTimestamp, indexRefsBySha } from "./CommitGraphTable.js";

function emptyLog(): GitLogResult {
  return { commits: [], branchHeads: [], tags: [], currentBranch: null };
}

describe("formatTimestamp", () => {
  test("renders an ISO date as YYYY-MM-DD HH:MM in local time", () => {
    // Construct a Date and round-trip its ISO so the assertion is timezone-agnostic.
    const date = new Date(2025, 5, 7, 14, 32); // June 7 2025, 14:32 local
    expect(formatTimestamp(date.toISOString())).toBe("2025-06-07 14:32");
  });

  test("zero-pads month, day, hour, and minute", () => {
    const date = new Date(2024, 0, 3, 5, 9); // Jan 3 2024, 05:09 local
    expect(formatTimestamp(date.toISOString())).toBe("2024-01-03 05:09");
  });

  test("empty input is empty string", () => {
    expect(formatTimestamp("")).toBe("");
  });

  test("non-parseable input passes through unchanged", () => {
    expect(formatTimestamp("not-a-date")).toBe("not-a-date");
  });
});

describe("indexRefsBySha", () => {
  test("returns empty maps when log is null", () => {
    const { branchHeadsBySha, tagsBySha } = indexRefsBySha(null);
    expect(branchHeadsBySha.size).toBe(0);
    expect(tagsBySha.size).toBe(0);
  });

  test("groups branch heads and tags by their commit sha", () => {
    const log: GitLogResult = {
      ...emptyLog(),
      branchHeads: [
        { name: "main", commit: { sha: "aaa" } },
        { name: "feature", commit: { sha: "bbb" } },
        { name: "release", commit: { sha: "aaa" } },
      ],
      tags: [
        { name: "v1.0", commit: { sha: "aaa" } },
        { name: "rc1", commit: { sha: "ccc" } },
      ],
    };
    const { branchHeadsBySha, tagsBySha } = indexRefsBySha(log);
    expect(branchHeadsBySha.get("aaa")?.sort()).toEqual(["main", "release"]);
    expect(branchHeadsBySha.get("bbb")).toEqual(["feature"]);
    expect(branchHeadsBySha.get("ccc")).toBeUndefined();
    expect(tagsBySha.get("aaa")).toEqual(["v1.0"]);
    expect(tagsBySha.get("ccc")).toEqual(["rc1"]);
  });
});
