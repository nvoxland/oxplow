import { expect, test } from "bun:test";

import { renameHint, renameSession } from "./sessionRename.js";

// Rename sets oxplow's alias; the harness's own name is separate, and only
// renamed where the harness can. A partial failure says which half held.

test("the alias alone: one call, with an empty alias clearing it", async () => {
  const calls: string[] = [];
  await renameSession({
    alias: "review",
    native: false,
    harnessTitle: "Claude Code",
    setAlias: async (a) => void calls.push(`alias ${a}`),
    setNative: async (n) => void calls.push(`native ${n}`),
  });
  await renameSession({
    alias: "",
    native: true,
    harnessTitle: "Codex",
    setAlias: async (a) => void calls.push(`alias ${a}`),
    setNative: async (n) => void calls.push(`native ${n}`),
  });
  // Nothing to name natively when the alias was cleared.
  expect(calls).toEqual(["alias review", "alias "]);
});

test("the alias, then the native name when asked", async () => {
  const calls: string[] = [];
  await renameSession({
    alias: "Login fix",
    native: true,
    harnessTitle: "Codex",
    setAlias: async (a) => void calls.push(`alias ${a}`),
    setNative: async (n) => void calls.push(`native ${n}`),
  });
  expect(calls).toEqual(["alias Login fix", "native Login fix"]);
});

test("a failed alias stops there; a failed native name says the alias was saved", async () => {
  const calls: string[] = [];
  await expect(
    renameSession({
      alias: "x",
      native: true,
      harnessTitle: "Codex",
      setAlias: async () => {
        throw new Error("denied");
      },
      setNative: async (n) => void calls.push(n),
    }),
  ).rejects.toThrow("denied");
  expect(calls).toEqual([]);
  await expect(
    renameSession({
      alias: "x",
      native: true,
      harnessTitle: "Codex",
      setAlias: async () => {},
      setNative: async () => {
        throw new Error("the daemon is gone");
      },
    }),
  ).rejects.toThrow("Saved oxplow's name, but Codex didn't take it: the daemon is gone");
});

test("the strip says whose name it sets and what clearing it does", () => {
  const claude = renameHint({ title: "", nativeTitle: "", displayName: "claude 2026-10-08", harnessTitle: "Claude Code", nativeRename: false });
  expect(claude.placeholder).toBe("claude 2026-10-08");
  expect(claude.message).toContain("oxplow's name for this session");
  expect(claude.message).toContain("Clear it to use Claude Code's");
  const codex = renameHint({ title: "review", nativeTitle: "Login fix", displayName: "review", harnessTitle: "Codex", nativeRename: true });
  expect(codex.placeholder).toBe("Login fix");
  expect(codex.message).toContain("Clear it to use Login fix");
  expect(codex.toggleLabel).toBe("Also rename it in Codex");
  expect(claude.toggleLabel).toBeNull();
});
