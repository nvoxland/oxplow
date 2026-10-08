import { expect, test } from "bun:test";
import { shouldReensure } from "./useChange.js";

const row = { id: 5 };

test("a v_change commit re-ensures the page's change", () => {
  expect(shouldReensure({ kind: "modelsChanged", models: ["v_change", "v_change_file"] }, row)).toBe(true);
  expect(shouldReensure({ kind: "modelsChanged", models: ["v_work_item"] }, row)).toBe(false);
  expect(shouldReensure({ kind: "snapshotTaken" }, row)).toBe(false);
});

test("nothing to re-ensure before the first answer", () => {
  expect(shouldReensure({ kind: "modelsChanged", models: ["v_change"] }, null)).toBe(false);
});
