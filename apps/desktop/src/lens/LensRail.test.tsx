import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { SqlCell } from "../api.js";
import type { Lens, LensParam } from "../tauri-bridge/generated/bindings.js";
import type { TabRef } from "../tabs/tabState.js";
import { fileRef, settingsRef } from "../tabs/pageRefs.js";
import { LensRail, type PickedRow } from "./LensRail.js";

afterEach(cleanup);

const p = (over: Partial<LensParam> & { name: string }): LensParam =>
  ({ label: null, default: null, options: [], presets: [], type: "text", role: "filter", ...over }) as LensParam;

const lens = {
  id: "commit-history/history",
  params: [
    p({ name: "range", label: "When", type: "date_range", default: "30d" }),
    p({ name: "q", label: "Search subjects", type: "search" }),
    p({ name: "commit", label: "Commit", role: "selection" }),
  ],
  origin: { extension: "commit-history", source: ".oxplow/extensions/commit-history/lenses/history.yaml", running: "x", deployment: "abc", bundled: false },
} as unknown as Lens;

function setup(values: Record<string, SqlCell>, picked: Record<string, PickedRow> = {}, over: { lens?: Lens; outOfDate?: boolean; fixed?: string[] } = {}) {
  const applied: [string, SqlCell][] = [];
  const opened: TabRef[] = [];
  const view = render(
    <LensRail
      lens={over.lens ?? lens}
      values={values}
      picked={picked}
      outOfDate={over.outOfDate ?? false}
      fixed={over.fixed}
      onApply={(n, v) => applied.push([n, v])}
      onOpenPage={(r) => opened.push(r)}
    />,
  );
  return { applied, opened, view };
}

// Filters, the selection and where the page comes from: one place each.
test("the rail puts filters under Filters and the selection under Selection, never as an input", () => {
  const { view } = setup({ range: "30d", q: null, commit: null });
  const filters = view.getByTestId("lens-rail-filters");
  expect(filters.querySelector("[data-testid=lens-param-range]")).not.toBeNull();
  expect(filters.querySelector("[data-testid=lens-param-q]")).not.toBeNull();
  expect(view.queryByTestId("lens-param-commit")).toBeNull();
  expect(view.getByTestId("lens-rail-selection").textContent).toContain("Click a row to select it.");
});

test("a picked row shows as its facts, and Clear sets the selection to nothing", () => {
  const picked = { commit: { value: "ba74ce57", facts: [{ label: "Commit", text: "ba74ce57" }, { label: "Subject", text: "Add the numbat" }] } };
  const { view, applied } = setup({ range: "30d", commit: "ba74ce57" }, picked);
  const selection = view.getByTestId("lens-rail-selection");
  expect(selection.textContent).toContain("Subject");
  expect(selection.textContent).toContain("Add the numbat");
  fireEvent.click(view.getByTestId("lens-selection-commit-clear"));
  expect(applied).toEqual([["commit", null]]);
});

// A reopened page has the value but not the row it came from.
test("a selection with no row shows its value", () => {
  const { view } = setup({ range: "30d", commit: "ba74ce57" });
  expect(view.getByTestId("lens-rail-selection").textContent).toContain("ba74ce57");
  expect(view.getByTestId("lens-selection-commit-clear")).not.toBeNull();
});

test("Reset puts every changed filter back to its default, and shows only when one changed", () => {
  expect(setup({ range: "30d", q: null }).view.queryByTestId("action-lens-rail-reset")).toBeNull();
  cleanup();
  const { view, applied } = setup({ range: "7d", q: "numbat" });
  fireEvent.click(view.getByTestId("action-lens-rail-reset"));
  expect(applied).toEqual([
    ["range", "30d"],
    ["q", null],
  ]);
});

test("About this page names the extension and the source file, which open", () => {
  const { view, opened } = setup({ range: "30d" });
  fireEvent.click(view.getByTestId("lens-about-extension"));
  fireEvent.click(view.getByTestId("lens-about-source"));
  expect(view.getByTestId("lens-about-source").textContent).toBe(".oxplow/extensions/commit-history/lenses/history.yaml");
  expect(opened.map((r) => r.id)).toEqual([settingsRef("extensions").id, fileRef(".oxplow/extensions/commit-history/lenses/history.yaml").id]);
  expect(view.queryByTestId("lens-about-out-of-date")).toBeNull();
});

test("About says when the source changed since its deploy, and when the page ships with oxplow", () => {
  expect(setup({}, {}, { outOfDate: true }).view.getByTestId("lens-about-out-of-date")).not.toBeNull();
  cleanup();
  const bundled = { ...lens, origin: { ...lens.origin, source: null, bundled: true } } as unknown as Lens;
  const { view } = setup({}, {}, { lens: bundled });
  expect(view.queryByTestId("lens-about-source")).toBeNull();
  expect(view.getByTestId("lens-rail-about").textContent).toContain("Ships with oxplow");
});

test("a lens with no filters has no Filters group, and none selected no Selection", () => {
  const bare = { ...lens, params: [] } as unknown as Lens;
  const { view } = setup({}, {}, { lens: bare });
  expect(view.queryByTestId("lens-rail-filters")).toBeNull();
  expect(view.queryByTestId("lens-rail-selection")).toBeNull();
  expect(view.getByTestId("lens-rail-about")).not.toBeNull();
});

// What the page is (a renderer's URI) isn't a control.
test("a fixed param isn't drawn", () => {
  const { view } = setup({ range: "30d", q: null, commit: null }, {}, { fixed: ["q"] });
  expect(view.queryByTestId("lens-param-q")).toBeNull();
  expect(view.queryByTestId("lens-param-range")).not.toBeNull();
});
