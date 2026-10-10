import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { ImpactReport } from "../tauri-bridge/generated/bindings.js";
import { ImpactReportView } from "./ImpactReportView.js";

afterEach(cleanup);

const report = (lines: string[]): ImpactReport => ({
  lenses: [],
  models: [],
  collectors: [],
  providers: [],
  effects: [],
  contributions: [],
  outOfTime: [],
  lines,
});

// An input whose event couldn't be made arrives as an error on that input
// (`its event: …`); the review says why on the input's line, never drops it.
test("the review shows why an input's event couldn't be made, on that input's line", () => {
  const why = "its event: `anchors.turn_id`: not a turn id";
  const { getByTestId } = render(
    <ImpactReportView
      report={report([
        `Effect on-done on fixture basic: fails (${why})`,
        `Collector gh on fixture basic: fails (${why})`,
      ])}
      testId="impact"
    />,
  );
  const lines = [...getByTestId("impact").querySelectorAll("li")].map((li) => li.textContent);
  expect(lines).toEqual([
    `Effect on-done on fixture basic: fails (${why})`,
    `Collector gh on fixture basic: fails (${why})`,
  ]);
});

test("a report with nothing to say renders nothing", () => {
  const { container } = render(<ImpactReportView report={report([])} testId="impact" />);
  expect(container.textContent).toBe("");
});
