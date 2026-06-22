import { expect, test } from "bun:test";

import {
  type ExplorerPreset,
  type PresetStore,
  loadPresets,
  removePreset,
  savePreset,
} from "./metricsPresets.js";

function fakeStore(): PresetStore {
  const map = new Map<string, string>();
  return {
    getItem: (k) => map.get(k) ?? null,
    setItem: (k, v) => {
      map.set(k, v);
    },
  };
}

const preset = (o: Partial<ExplorerPreset>): ExplorerPreset => ({
  name: "v",
  selected: [],
  groupBy: "none",
  viz: "line",
  ...o,
});

test("save then load round-trips a preset", () => {
  const store = fakeStore();
  savePreset(preset({ name: "cov-vs-cx", selected: ["a", "b"], groupBy: "subject" }), store);
  const got = loadPresets(store);
  expect(got).toHaveLength(1);
  expect(got[0]!.name).toBe("cov-vs-cx");
  expect(got[0]!.selected).toEqual(["a", "b"]);
});

test("saving the same name replaces, not duplicates; list stays sorted", () => {
  const store = fakeStore();
  savePreset(preset({ name: "b" }), store);
  savePreset(preset({ name: "a" }), store);
  savePreset(preset({ name: "a", selected: ["x"] }), store);
  const got = loadPresets(store);
  expect(got.map((p) => p.name)).toEqual(["a", "b"]);
  expect(got.find((p) => p.name === "a")!.selected).toEqual(["x"]);
});

test("blank name is ignored", () => {
  const store = fakeStore();
  savePreset(preset({ name: "   " }), store);
  expect(loadPresets(store)).toHaveLength(0);
});

test("removePreset drops by name", () => {
  const store = fakeStore();
  savePreset(preset({ name: "a" }), store);
  savePreset(preset({ name: "b" }), store);
  const after = removePreset("a", store);
  expect(after.map((p) => p.name)).toEqual(["b"]);
  expect(loadPresets(store).map((p) => p.name)).toEqual(["b"]);
});

test("malformed storage yields empty list", () => {
  const store = fakeStore();
  store.setItem("oxplow.metrics.explorerPresets", "{not an array}");
  expect(loadPresets(store)).toEqual([]);
});
