import { describe, expect, test } from "bun:test";

import { branchRefOf } from "../vcsHistory.js";
import { pickedBranch } from "./BranchPicker.js";

describe("pickedBranch", () => {
  test("a remote branch checks out under its own name", () => {
    for (const name of ["main", "feature/login"]) {
      const branch = branchRefOf({ name, remote: "origin", head: null, isDefault: false, streamId: null });
      expect(pickedBranch(branch)).toEqual({ kind: "branch", name, branch });
    }
  });

  test("a local branch too", () => {
    const branch = branchRefOf({ name: "fix/x", remote: null, head: null, isDefault: false, streamId: null });
    expect(pickedBranch(branch).name).toBe("fix/x");
  });
});
