import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { LensResultView } from "./LensResultView.js";

afterEach(cleanup);

const base: Lens = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "L",
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  copy: false,
  path: "",
};
const chart = { x: null, y: null, series: null, label: null, size: null, group: null };
const run = (lens: Partial<Lens>, columns: string[], rows: (string | number)[][]): LensRun => ({
  lens: { ...base, ...lens },
  params: {},
  result: { columns, rows, truncated: false },
});

test("bar lenses draw one bar per row", () => {
  const { container } = render(
    <LensResultView
      run={run({ viz: "bar", chart: { ...chart, x: "day", y: "n" } }, ["day", "n"], [["mon", 3], ["tue", 5]])}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelector('[data-testid="lens-bar"]')).not.toBeNull();
  expect(container.querySelector('[title="mon: 3"]')).not.toBeNull();
  expect(container.querySelector('[title="tue: 5"]')).not.toBeNull();
});

test("line lenses draw one chart per series", () => {
  const rows = [
    [1_000, 1, "a"],
    [2_000, 2, "a"],
    [1_000, 3, "b"],
    [2_000, 4, "b"],
  ];
  const { container } = render(
    <LensResultView
      run={run({ viz: "line", chart: { ...chart, x: "t", y: "v", series: "s" } }, ["t", "v", "s"], rows)}
      onOpenPage={() => {}}
    />,
  );
  expect(container.querySelectorAll('[data-testid="lens-line-series"]').length).toBe(2);
  expect(container.textContent).toContain("a");
});

test("treemap lenses draw a tile per positive item and link through the lens's column link", () => {
  const opened: string[] = [];
  const { container } = render(
    <LensResultView
      run={run(
        {
          viz: "treemap",
          chart: { ...chart, label: "path", size: "churn", group: "zone" },
          columns: [{ key: "path", label: null, link: { kind: "file", from: null, line: null } }],
        },
        ["path", "churn", "zone"],
        [
          ["a.rs", 5, "core"],
          ["b.rs", 2, "ui"],
          ["c.rs", 0, "ui"],
        ],
      )}
      onOpenPage={(ref) => opened.push(ref.id)}
    />,
  );
  const tiles = container.querySelectorAll('[data-testid="lens-treemap-tile"]');
  expect(tiles.length).toBe(2);
  (tiles[0] as SVGElement).dispatchEvent(new MouseEvent("click", { bubbles: true }));
  expect(opened).toEqual(["file:a.rs"]);
});

test("a markdown lens with copy: true copies its text", async () => {
  const written: string[] = [];
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText: async (t: string) => void written.push(t) },
    configurable: true,
  });
  const { getByTestId, container } = render(
    <LensResultView run={run({ viz: "markdown", copy: true }, ["prompt"], [["Review **this**"]])} onOpenPage={() => {}} />,
  );
  expect(container.textContent).toContain("Review");
  fireEvent.click(getByTestId("lens-copy"));
  await Promise.resolve();
  expect(written).toEqual(["Review **this**"]);
});

test("a markdown lens without copy has no button", () => {
  const { queryByTestId } = render(
    <LensResultView run={run({ viz: "markdown" }, ["t"], [["x"]])} onOpenPage={() => {}} />,
  );
  expect(queryByTestId("lens-copy")).toBeNull();
});
