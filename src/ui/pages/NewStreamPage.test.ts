import { describe, expect, test } from "bun:test";
import { validateNewStreamInput } from "./NewStreamPage.js";

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
