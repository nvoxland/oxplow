import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

// A lazy tree's open branches each run the lens again with the lazy param
// set to the branch's id: only the branches reachable through open parents
// run, one opening keeps the others' rows, closing drops a run, and a late
// answer never lands over a newer one.

const calls: { params: Record<string, SqlCell>; stream: string | null }[] = [];
let answer: (params: Record<string, SqlCell>) => Promise<LensRun> = async (params) => level(params);
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  runLens: async (_id: string, params: Record<string, SqlCell>, stream: string | null) => {
    calls.push({ params, stream });
    return answer(params);
  },
}));

const { useLazyBranches } = await import("./useLazyBranches.js");

const lens = {
  id: "x/files",
  viz: "tree",
  tree: { id: "id", parent: "p", label: "n", lazy: "folder", branch: "d" },
  params: [{ name: "folder", label: null, default: "", options: [] }],
  columns: [],
  actions: [],
} as unknown as Lens;
const rows = (folder: string): SqlCell[][] =>
  folder === "" ? [["src", "", "src", 1], ["a.rs", "", "a.rs", 0]] : [[`${folder}/lib`, folder, "lib", 1], [`${folder}/m.rs`, folder, "m.rs", 0]];
function level(params: Record<string, SqlCell>): LensRun {
  return { lens, params, result: { columns: ["id", "p", "n", "d"], rows: rows(String(params.folder ?? "")), truncated: false } } as unknown as LensRun;
}
const root = level({ folder: "" });

let seen: ReadonlyMap<string, { run: LensRun | null; error: string | null }> = new Map();
function Probe({ open }: { open: readonly string[] }) {
  seen = useLazyBranches(root, open, "str1");
  return <div data-testid="n">{[...seen.keys()].sort().join(",")}</div>;
}

beforeEach(() => {
  calls.length = 0;
  answer = async (params) => level(params);
});
afterEach(cleanup);

test("an open branch runs the lens with the lazy param set to its id, in the stream", async () => {
  const view = render(<Probe open={["src"]} />);
  await waitFor(() => expect(view.getByTestId("n").textContent).toBe("src"));
  expect(calls).toEqual([{ params: { folder: "src" }, stream: "str1" }]);
  expect(seen.get("src")?.run?.result.rows.map((r) => r[0])).toEqual(["src/lib", "src/m.rs"]);
});

test("a branch under a closed parent waits; opening the parent runs it, and closing drops it", async () => {
  const view = render(<Probe open={["src/lib"]} />);
  await new Promise((r) => setTimeout(r, 20));
  expect(calls).toEqual([]);
  view.rerender(<Probe open={["src/lib", "src"]} />);
  await waitFor(() => expect(view.getByTestId("n").textContent).toBe("src,src/lib"));
  expect(calls.map((c) => c.params.folder)).toEqual(["src", "src/lib"]);
  view.rerender(<Probe open={["src"]} />);
  await waitFor(() => expect(view.getByTestId("n").textContent).toBe("src"));
  // The kept branch did not run again.
  expect(calls).toHaveLength(2);
});

test("a run that fails is kept as its error", async () => {
  answer = async () => {
    throw new Error("no such folder");
  };
  const view = render(<Probe open={["src"]} />);
  await waitFor(() => expect(seen.get("src")?.error).toBe("no such folder"));
  expect(view.getByTestId("n").textContent).toBe("src");
});
