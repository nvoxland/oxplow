import { describe, expect, test } from "bun:test";
import { pickDefaultBranchEntry, validateNewStreamInput } from "./NewStreamPage.js";
import type { BranchRef } from "../api.js";

describe("validateNewStreamInput", () => {
  test("rejects an empty title regardless of mode", () => {
    expect(validateNewStreamInput({ mode: "existing", title: "  ", selectedRef: "refs/heads/main", newBranch: "", startPointRef: "", worktreePath: "" })).toEqual({
      ok: false,
      message: "Name is required",
    });
  });

  test("existing mode requires a selected branch", () => {
    expect(validateNewStreamInput({ mode: "existing", title: "ok", selectedRef: "", newBranch: "", startPointRef: "", worktreePath: "" })).toEqual({
      ok: false,
      message: "Select an existing branch",
    });
  });

  test("new-branch mode requires a branch name and start point", () => {
    expect(validateNewStreamInput({ mode: "new", title: "ok", selectedRef: "", newBranch: "", startPointRef: "ref", worktreePath: "" })).toEqual({
      ok: false,
      message: "Enter a new branch name",
    });
    expect(validateNewStreamInput({ mode: "new", title: "ok", selectedRef: "", newBranch: "feature/x", startPointRef: "", worktreePath: "" })).toEqual({
      ok: false,
      message: "Choose a starting branch",
    });
  });

  test("worktree mode requires a worktree path", () => {
    expect(validateNewStreamInput({ mode: "worktree", title: "ok", selectedRef: "", newBranch: "", startPointRef: "", worktreePath: "" })).toEqual({
      ok: false,
      message: "Select a worktree",
    });
  });

  test("returns ok when all required fields are present", () => {
    expect(validateNewStreamInput({ mode: "existing", title: "ok", selectedRef: "ref", newBranch: "", startPointRef: "", worktreePath: "" })).toEqual({ ok: true });
    expect(validateNewStreamInput({ mode: "new", title: "ok", selectedRef: "", newBranch: "feature", startPointRef: "ref", worktreePath: "" })).toEqual({ ok: true });
    expect(validateNewStreamInput({ mode: "worktree", title: "ok", selectedRef: "", newBranch: "", startPointRef: "", worktreePath: "/tmp/wt" })).toEqual({ ok: true });
  });
});

describe("pickDefaultBranchEntry", () => {
  const local = (name: string): BranchRef => ({ kind: "local", name, ref: `refs/heads/${name}` });
  const remote = (name: string): BranchRef => ({ kind: "remote", name, ref: `refs/remotes/${name}`, remote: name.split("/")[0] });

  test("returns null when no default branch is reported", () => {
    expect(pickDefaultBranchEntry([local("feature"), local("main")], null)).toBeNull();
  });

  test("prefers a local branch matching the default name", () => {
    const branches = [local("feature"), local("main"), remote("origin/main")];
    expect(pickDefaultBranchEntry(branches, "main")?.ref).toBe("refs/heads/main");
    expect(pickDefaultBranchEntry(branches, "origin/main")?.ref).toBe("refs/heads/main");
  });

  test("falls back to the matching remote branch when no local exists", () => {
    const branches = [local("feature"), remote("origin/main")];
    expect(pickDefaultBranchEntry(branches, "origin/main")?.ref).toBe("refs/remotes/origin/main");
    expect(pickDefaultBranchEntry(branches, "main")?.ref).toBe("refs/remotes/origin/main");
  });

  test("returns null when the default branch is not present in the list", () => {
    expect(pickDefaultBranchEntry([local("feature")], "main")).toBeNull();
  });
});
