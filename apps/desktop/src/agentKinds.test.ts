import { expect, test } from "bun:test";

import type { HarnessListing } from "./tauri-bridge/generated/bindings.js";
import { agentChoices, harnessLabel, sessionLabel } from "./agentKinds.js";

const harness = (id: string, title: string, over: Partial<HarnessListing> = {}): HarnessListing => ({
  id,
  title,
  enabled: true,
  settings: [],
  features: [],
  ...over,
});

const harnesses = [harness("claude", "Claude"), harness("opencode", "opencode"), harness("codex", "Codex", { enabled: false })];

test("each enabled harness is a choice, in order", () => {
  expect(agentChoices(harnesses).map((c) => [c.value, c.label])).toEqual([
    ["claude", "Claude"],
    ["opencode", "opencode"],
  ]);
});

test("a session is labelled by its harness's title, else its key", () => {
  expect(sessionLabel(harnesses, { harness: "claude" })).toBe("Claude");
  expect(harnessLabel(harnesses, "gone")).toBe("gone");
});
