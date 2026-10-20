import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { MetricFilters } from "./MetricFilters.js";

afterEach(cleanup);

test("range, branch and slice are the rows a lens page has, and an edit applies its value", () => {
  const applied: [string, string][] = [];
  const view = render(
    <MetricFilters
      range="7d"
      branch=""
      branches={["main", "dev"]}
      slice=""
      dimensions={[{ key: "oxplow.language", label: "Language" }]}
      onApply={(name, value) => applied.push([name, value])}
    />,
  );
  expect(view.getByTestId("lens-param-range-value").textContent).toBe("Last 7 days");
  expect(view.getByTestId("lens-param-branch-value").textContent).toBe("Any");

  fireEvent.click(view.getByTestId("lens-param-branch-value"));
  fireEvent.change(view.getByTestId("lens-param-branch-edit"), { target: { value: "dev" } });
  fireEvent.click(view.getByTestId("lens-param-slice-value"));
  fireEvent.change(view.getByTestId("lens-param-slice-edit"), { target: { value: "oxplow.language" } });
  fireEvent.click(view.getByTestId("lens-param-range-value"));
  fireEvent.change(view.getByTestId("lens-param-range-edit"), { target: { value: "30d" } });
  expect(applied).toEqual([
    ["branch", "dev"],
    ["slice", "oxplow.language"],
    ["range", "30d"],
  ]);
});

test("a filter left out is not drawn, and slice hides while there is no dimension", () => {
  const view = render(<MetricFilters range="all" slice="" onApply={() => {}} />);
  expect(view.queryByTestId("lens-param-branch")).toBeNull();
  expect(view.queryByTestId("lens-param-slice")).toBeNull();
  expect(view.getByTestId("lens-param-range-value").textContent).toBe("All time");
});
