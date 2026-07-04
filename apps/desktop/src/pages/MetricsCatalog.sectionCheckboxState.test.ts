import { describe, expect, it } from "bun:test";

import { sectionCheckboxState } from "./MetricsCatalog.js";

describe("sectionCheckboxState", () => {
  const rows = (...on: boolean[]) => on.map((enabled) => ({ enabled }));

  it("all enabled → checked, not indeterminate, clicking disables all", () => {
    expect(sectionCheckboxState(rows(true, true, true))).toEqual({
      checked: true,
      indeterminate: false,
      nextEnabled: false,
    });
  });

  it("all disabled → unchecked, not indeterminate, clicking enables all", () => {
    expect(sectionCheckboxState(rows(false, false))).toEqual({
      checked: false,
      indeterminate: false,
      nextEnabled: true,
    });
  });

  it("mixed → indeterminate, clicking enables all", () => {
    expect(sectionCheckboxState(rows(true, false, true))).toEqual({
      checked: false,
      indeterminate: true,
      nextEnabled: true,
    });
  });
});
