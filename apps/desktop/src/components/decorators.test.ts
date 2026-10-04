import { expect, test } from "bun:test";

import type { Extension, SqlQueryResult, UiDecorator } from "../tauri-bridge/generated/bindings.js";
import {
  chipsFor,
  decorationQueries,
  decorationsFromResult,
  decoratorsFor,
  MAX_DECORATIONS_PER_REF,
  MAX_LABEL,
  REFS_PER_QUERY,
  safeColor,
} from "./decorators.js";

const decorator = (over: Partial<UiDecorator>): UiDecorator => ({
  id: "flags/0",
  extension: "flags",
  view: "v_flags_flags",
  kind: "work_item",
  placement: "ref-chip",
  label: "label",
  color: "color",
  ...over,
});

// P6b.C5: decorations are labels from an extension's model on core refs.
test("the enabled extensions' decorators for a placement", () => {
  const exts = [
    { name: "flags", enabled: true, ui: { slots: [], commands: [], decorators: [decorator({}), decorator({ id: "flags/1", placement: "row-badge" })] } },
    { name: "off", enabled: false, ui: { slots: [], commands: [], decorators: [decorator({ id: "off/0" })] } },
  ] as unknown as Extension[];
  expect(decoratorsFor(exts, "ref-chip").map((d) => d.id)).toEqual(["flags/0"]);
  expect(decoratorsFor(exts, "row-badge").map((d) => d.id)).toEqual(["flags/1"]);
});

test("one query per decorator, over the refs of its kind", () => {
  expect(decorationQueries(decorator({}), ["work_item:oxplow:tsk1", "commit:abc1234", "work_item:fake:W-1"])).toEqual([
    {
      sql: 'SELECT ref, "label" AS label, "color" AS color FROM v_flags_flags WHERE ref IN (?1, ?2)',
      params: ["work_item:oxplow:tsk1", "work_item:fake:W-1"],
      limit: 2 * MAX_DECORATIONS_PER_REF,
    },
  ]);
  expect(decorationQueries(decorator({ color: null }), ["work_item:oxplow:tsk1"])[0]?.sql).toBe(
    'SELECT ref, "label" AS label FROM v_flags_flags WHERE ref IN (?1)',
  );
  expect(decorationQueries(decorator({}), ["commit:abc1234"])).toEqual([]);
});

// tsk934: a large table's row badges aren't cut off by one row limit —
// the refs go in chunks, each with room for every ref's decorations.
test("many refs are asked in chunks, each with room for all its decorations", () => {
  const refs = Array.from({ length: 2 * REFS_PER_QUERY + 50 }, (_, i) => `work_item:oxplow:tsk${i}`);
  const queries = decorationQueries(decorator({}), refs);
  expect(queries.map((q) => q.params.length)).toEqual([REFS_PER_QUERY, REFS_PER_QUERY, 50]);
  expect(queries.flatMap((q) => q.params)).toEqual(refs);
  expect(queries[0].limit).toBe(REFS_PER_QUERY * MAX_DECORATIONS_PER_REF);
});

// tsk934: one extension adds at most a few short labels to a ref.
test("an extension's decorations on one ref are few and short", () => {
  const res = {
    columns: ["ref", "label"],
    rows: Array.from({ length: 10 }, (_, i) => ["work_item:oxplow:tsk1", `${i}${"x".repeat(100)}`]),
    truncated: false,
  } as unknown as SqlQueryResult;
  const decorations = decorationsFromResult(res, "flags");
  expect(decorations.length).toBe(MAX_DECORATIONS_PER_REF);
  expect(decorations.every((d) => [...d.label].length <= MAX_LABEL)).toBe(true);
  expect(decorations[0].label.endsWith("…")).toBe(true);
});

// The loader refuses a column or view that isn't a plain identifier; the
// one place that builds the SQL refuses it too, so no decorator reaches a
// query with anything else in it.
test("a decorator naming anything but identifiers builds no query", () => {
  for (const bad of [{ label: 'x" ; DROP TABLE task; --' }, { color: "c)" }, { view: "v_x; DELETE FROM task" }]) {
    expect(decorationQueries(decorator(bad), ["work_item:oxplow:tsk1"])).toEqual([]);
  }
});

test("decorations become chips for their ref; a color that isn't a plain color is dropped", () => {
  const res = {
    columns: ["ref", "label", "color"],
    rows: [
      ["work_item:oxplow:tsk1", "urgent", "#ff0000"],
      ["work_item:oxplow:tsk1", "flaky", "url(https://x)"],
      ["work_item:oxplow:tsk2", "other", null],
    ],
    truncated: false,
  } as unknown as SqlQueryResult;
  const decorations = decorationsFromResult(res, "flags");
  expect(chipsFor(decorations, "work_item:oxplow:tsk1")).toEqual([
    { label: "urgent", color: "#ff0000", title: "from flags" },
    { label: "flaky", title: "from flags" },
  ]);
  expect(safeColor("rebeccapurple")).toBe("rebeccapurple");
  expect(safeColor("red; background: url(x)")).toBeNull();
});
