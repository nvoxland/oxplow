import { describe, expect, test } from "bun:test";
import {
  gitCommitBacklinks,
  gitDashboardBacklinks,
  gitHistoryBacklinks,
  uncommittedChangesBacklinks,
  type AppBacklinkContext,
} from "./appPageBacklinks.js";

const baseCtx = (overrides: Partial<AppBacklinkContext> = {}): AppBacklinkContext => ({
  notes: [],
  workItems: [],
  findings: [],
  ...overrides,
});

const mkCommit = (sha: string, subject: string) => ({
  sha,
  parents: [],
  commit: { author: { name: "x", email: "y@z", date: "" }, message: subject },
  refs: [],
});

describe("gitDashboardBacklinks", () => {
  test("includes git-history + uncommitted always", () => {
    const out = gitDashboardBacklinks(null, baseCtx());
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("git-history");
    expect(ids).toContain("uncommitted-changes");
  });

  test("includes up to 5 recent commits as commit refs", () => {
    const ctx = baseCtx({ recentLog: Array.from({ length: 7 }, (_, i) => mkCommit(`sha${i}`, `subj ${i}`)) });
    const out = gitDashboardBacklinks(null, ctx);
    const commitIds = out.map((e) => e.ref.id).filter((id) => id.startsWith("git-commit:"));
    expect(commitIds).toHaveLength(5);
    expect(commitIds[0]).toBe("git-commit:sha0");
  });

  test("includes notes mentioning the current branch", () => {
    const ctx = baseCtx({
      currentBranch: "feature/x",
      notes: [{ slug: "n1", title: "Plan", body: "see feature/x for details" }],
    });
    const out = gitDashboardBacklinks(null, ctx);
    expect(out.some((e) => e.ref.id === "note:n1")).toBe(true);
  });
});

describe("uncommittedChangesBacklinks", () => {
  test("emits a file ref per uncommitted path", () => {
    const out = uncommittedChangesBacklinks(null, baseCtx({ uncommittedPaths: ["a.ts", "b/c.ts"] }));
    expect(out.map((e) => e.ref.id)).toEqual(["file:a.ts", "file:b/c.ts"]);
  });

  test("links work items whose touched_files overlap", () => {
    const ctx = baseCtx({
      uncommittedPaths: ["src/x.ts"],
      workItems: [
        { id: "wi-1", title: "Touched x", description: "", acceptance_criteria: null, touched_files: ["src/x.ts"] },
        { id: "wi-2", title: "Other", description: "", acceptance_criteria: null, touched_files: ["src/y.ts"] },
      ],
    });
    const out = uncommittedChangesBacklinks(null, ctx);
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("wi:wi-1");
    expect(ids).not.toContain("wi:wi-2");
  });

  test("links notes mentioning any uncommitted path", () => {
    const ctx = baseCtx({
      uncommittedPaths: ["src/foo.ts"],
      notes: [{ slug: "n1", title: "Foo notes", body: "see src/foo.ts" }],
    });
    const out = uncommittedChangesBacklinks(null, ctx);
    expect(out.some((e) => e.ref.id === "note:n1")).toBe(true);
  });
});

describe("gitCommitBacklinks", () => {
  test("links to git-history and notes mentioning the sha", () => {
    const ctx = baseCtx({
      notes: [{ slug: "n1", title: "Discussion", body: "fix landed in abc1234" }],
    });
    const out = gitCommitBacklinks({ sha: "abc1234567890" }, ctx);
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("git-history");
    expect(ids).toContain("note:n1");
  });

  test("links to previous and next commits when present in recentLog", () => {
    const ctx = baseCtx({
      recentLog: [mkCommit("c2", "newer"), mkCommit("c1", "current"), mkCommit("c0", "older")],
    });
    const out = gitCommitBacklinks({ sha: "c1" }, ctx);
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("git-commit:c2");
    expect(ids).toContain("git-commit:c0");
  });

  test("emits a file ref per touched file when commitDetail is supplied", () => {
    const ctx = baseCtx({
      commitDetail: {
        sha: "c1",
        parents: [],
        author: { name: "x", email: "", date: "" },
        committer: { name: "x", email: "", date: "" },
        subject: "x",
        body: "",
        files: [
          { path: "src/a.ts", status: "modified", additions: 1, deletions: 0 },
          { path: "old.ts → new.ts", status: "renamed", additions: 0, deletions: 0 },
        ],
      },
    });
    const out = gitCommitBacklinks({ sha: "c1" }, ctx);
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("file:src/a.ts");
    expect(ids).toContain("file:new.ts");
    expect(ids).not.toContain("file:old.ts → new.ts");
  });

  test("links work items whose touched_files overlap commitDetail.files", () => {
    const ctx = baseCtx({
      workItems: [
        { id: "wi-1", title: "Refactor a.ts", description: "", acceptance_criteria: null, touched_files: ["src/a.ts"] },
        { id: "wi-2", title: "Unrelated", description: "", acceptance_criteria: null, touched_files: ["src/b.ts"] },
      ],
      commitDetail: {
        sha: "c1",
        parents: [],
        author: { name: "x", email: "", date: "" },
        committer: { name: "x", email: "", date: "" },
        subject: "x",
        body: "",
        files: [{ path: "src/a.ts", status: "modified", additions: 1, deletions: 0 }],
      },
    });
    const out = gitCommitBacklinks({ sha: "c1" }, ctx);
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("wi:wi-1");
    expect(ids).not.toContain("wi:wi-2");
  });

  test("ignores commitDetail when its sha doesn't match the target", () => {
    const ctx = baseCtx({
      commitDetail: {
        sha: "different",
        parents: [],
        author: { name: "x", email: "", date: "" },
        committer: { name: "x", email: "", date: "" },
        subject: "x",
        body: "",
        files: [{ path: "src/a.ts", status: "modified", additions: 1, deletions: 0 }],
      },
    });
    const out = gitCommitBacklinks({ sha: "c1" }, ctx);
    expect(out.map((e) => e.ref.id)).not.toContain("file:src/a.ts");
  });
});

describe("gitHistoryBacklinks", () => {
  test("includes dashboard + uncommitted", () => {
    const out = gitHistoryBacklinks(null, baseCtx());
    const ids = out.map((e) => e.ref.id);
    expect(ids).toContain("git-dashboard");
    expect(ids).toContain("uncommitted-changes");
  });
});
