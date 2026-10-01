import { expect, test } from "bun:test";

import type { Extension, SqlQueryResult, UiDecorator } from "../tauri-bridge/generated/bindings.js";
import { chipsFor, decorationQuery, decorationsFromResult, decoratorsFor, safeColor } from "./decorators.js";

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
  expect(decorationQuery(decorator({}), ["work_item:oxplow:tsk1", "commit:abc1234", "work_item:fake:W-1"])).toEqual({
    sql: 'SELECT ref, "label" AS label, "color" AS color FROM v_flags_flags WHERE ref IN (?1, ?2)',
    params: ["work_item:oxplow:tsk1", "work_item:fake:W-1"],
  });
  expect(decorationQuery(decorator({ color: null }), ["work_item:oxplow:tsk1"])?.sql).toBe(
    'SELECT ref, "label" AS label FROM v_flags_flags WHERE ref IN (?1)',
  );
  expect(decorationQuery(decorator({}), ["commit:abc1234"])).toBeNull();
});

// The loader refuses a column or view that isn't a plain identifier; the
// one place that builds the SQL refuses it too, so no decorator reaches a
// query with anything else in it.
test("a decorator naming anything but identifiers builds no query", () => {
  for (const bad of [{ label: 'x" ; DROP TABLE task; --' }, { color: "c)" }, { view: "v_x; DELETE FROM task" }]) {
    expect(decorationQuery(decorator(bad), ["work_item:oxplow:tsk1"])).toBeNull();
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
