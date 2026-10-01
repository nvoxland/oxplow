import { expect, test } from "bun:test";

import type { ProviderEffect } from "../tauri-bridge/generated/bindings.js";
import { providerChanges } from "./providerEffectText.js";

const grants = (hosts: string[]) => ({ entry: "bin/p", runtime: "exec", args: [], hosts, credentials: ["token"], env: [] });
const base: ProviderEffect = {
  id: "fake",
  capability: "work_items",
  change: "changed",
  before: grants([]),
  after: grants([]),
  commands: [],
  featuresBefore: null,
  featuresAfter: null,
  firstDifference: null,
};

test("a new provider reads as everything it declares", () => {
  const added: ProviderEffect = {
    ...base,
    change: "added",
    before: null,
    after: grants(["api.example.com"]),
    commands: [
      { name: "create", change: "added", before: null, after: { confirm: "never" } },
      { name: "delete", change: "added", before: null, after: { confirm: "destructive" } },
    ],
  };
  expect(providerChanges(added)).toEqual([
    "added — runs bin/p · reaches api.example.com · reads token",
    "commands: create, delete (destructive)",
  ]);
});

test("a changed provider reads as what differs", () => {
  expect(
    providerChanges({
      ...base,
      after: grants(["api.example.com"]),
      commands: [
        { name: "create", change: "unchanged", before: {}, after: {} },
        { name: "archive", change: "added", before: null, after: { confirm: "destructive" } },
      ],
    }),
  ).toEqual(["now reaches api.example.com (was none)", "command `archive` added (destructive)"]);
});

test("a change no grant, command or feature shows names its first difference", () => {
  expect(providerChanges({ ...base, firstDifference: "`/declarations/version` was 1, now 2" })).toEqual([
    "`/declarations/version` was 1, now 2",
  ]);
  expect(providerChanges({ ...base, change: "unchanged" })).toEqual([]);
});
