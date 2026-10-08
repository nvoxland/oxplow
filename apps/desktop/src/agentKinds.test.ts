import { expect, test } from "bun:test";

import type { AcpAgentListing, HarnessListing } from "./tauri-bridge/generated/bindings.js";
import { agentChoices, harnessLabel, parseAgentChoice, sessionLabel } from "./agentKinds.js";

const acp = (name: string, over: Partial<AcpAgentListing> = {}): AcpAgentListing => ({
  name,
  command: name,
  args: [],
  source: "declared",
  approved: true,
  resolvedPath: `/bin/${name}`,
  ...over,
});

const harness = (id: string, title: string, over: Partial<HarnessListing> = {}): HarnessListing => ({
  id,
  title,
  chat: false,
  enabled: true,
  ...over,
});

const harnesses = [harness("claude", "Claude"), harness("acp", "ACP", { chat: true }), harness("codex", "Codex", { enabled: false })];

test("each enabled harness is a choice, in order; a chat harness is one per ACP agent, flagged when it can't start", () => {
  const choices = agentChoices(harnesses, [
    acp("claude"),
    acp("gemini", { resolvedPath: null }),
    acp("mine", { source: "project", approved: false }),
  ]);
  expect(choices.map((c) => [c.value, c.label])).toEqual([
    ["claude", "Claude"],
    ["acp:claude", "ACP · claude"],
    ["acp:gemini", "ACP · gemini (not installed)"],
    ["acp:mine", "ACP · mine (needs approval)"],
  ]);
});

test("a choice value parses back to the harness and ACP agent", () => {
  expect(parseAgentChoice("codex")).toEqual({ harness: "codex", acpAgent: null });
  expect(parseAgentChoice("acp:gemini")).toEqual({ harness: "acp", acpAgent: "gemini" });
});

test("a session is labelled by its harness's title, and a chat session by its ACP agent too", () => {
  expect(sessionLabel(harnesses, { harness: "acp", acpAgent: "gemini" })).toBe("ACP · gemini");
  expect(sessionLabel(harnesses, { harness: "claude", acpAgent: null })).toBe("Claude");
  expect(harnessLabel(harnesses, "gone")).toBe("gone");
});
