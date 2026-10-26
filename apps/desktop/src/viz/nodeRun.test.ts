import { expect, test } from "bun:test";

import { embedAt, nodeById, nodeRunOf } from "./nodeRun.js";
import { fakePage, fakeRun, loader, node, stackPage, tablePage } from "../pages/testing/fakeRun.js";

test("a viz node is a viz run over its loader's rows, with the run's params", () => {
  const run = { ...tablePage(), params: { q: "x" } };
  const viz = nodeById(run.page, "items")!;
  const vizRun = nodeRunOf(run, viz)!;
  expect(vizRun.config).toBe(viz.kind === "viz" ? viz.config : null);
  expect(vizRun.params).toEqual({ q: "x" });
  expect(vizRun.result.rows).toEqual([
    ["a", "Alpha"],
    ["b", "Beta"],
  ]);
});

test("a viz with no loader, or a loader that didn't run, has no rows", () => {
  const page = fakePage(node.stack("s", [node.viz("form", null), node.viz("gone", "missing")]));
  const run = fakeRun(page);
  expect(nodeRunOf(run, nodeById(page, "form")!)!.result).toMatchObject({ columns: [], rows: [] });
  expect(nodeRunOf(run, nodeById(page, "gone")!)!.result).toMatchObject({ columns: [], rows: [] });
});

test("only a viz node has a viz run", () => {
  const run = stackPage();
  expect(nodeRunOf(run, nodeById(run.page, "intro")!)).toBeNull();
});

test("nodeById finds a node anywhere in the view or the rail", () => {
  const page = fakePage(node.stack("s", [node.grid("g", [node.markdown("deep", "x")])]), {
    rail: node.stack("rail", [node.markdown("side", "y")]),
  });
  expect(nodeById(page, "deep")?.kind).toBe("markdown");
  expect(nodeById(page, "side")?.kind).toBe("markdown");
  expect(nodeById(page, "nope")).toBeNull();
});

test("embedAt follows `<embed>/<id>` paths to the run that holds the node", () => {
  const run = stackPage();
  expect(embedAt(run, "items")).toEqual({ run, id: "items" });
  const inner = embedAt(run, "notes/notes")!;
  expect(inner.id).toBe("notes");
  expect(inner.run.page.id).toBe("acme/notes");
  expect(inner.run.loaders.notes).toEqual(loader(["note"], [["inner note"]]));
  expect(embedAt(run, "nope/notes")).toBeNull();
});
