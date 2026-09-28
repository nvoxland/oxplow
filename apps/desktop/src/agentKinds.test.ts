import { expect, test } from "bun:test";

import type { AcpAgentListing } from "./tauri-bridge/generated/bindings.js";
import { agentChoices, parseAgentChoice, threadAgentLabel } from "./agentKinds.js";

const acp = (name: string, over: Partial<AcpAgentListing> = {}): AcpAgentListing => ({
  name,
  command: name,
  args: [],
  source: "preset",
  approved: true,
  resolvedPath: `/bin/${name}`,
  ...over,
});

test("the ACP kind expands into one choice per ACP agent, flagged when it can't start", () => {
  const choices = agentChoices(
    ["claude", "acp"],
    [acp("claude"), acp("gemini", { resolvedPath: null }), acp("mine", { source: "project", approved: false })],
  );
  expect(choices.map((c) => [c.value, c.label])).toEqual([
    ["claude", "Claude"],
    ["acp:claude", "ACP · claude"],
    ["acp:gemini", "ACP · gemini (not installed)"],
    ["acp:mine", "ACP · mine (needs approval)"],
  ]);
  // Without ACP enabled, no ACP choices.
  expect(agentChoices(["claude", "codex"], [acp("gemini")]).map((c) => c.value)).toEqual(["claude", "codex"]);
});

test("a choice value parses back to the agent and ACP agent", () => {
  expect(parseAgentChoice("codex")).toEqual({ agent: "codex", acpAgent: null });
  expect(parseAgentChoice("acp:gemini")).toEqual({ agent: "acp", acpAgent: "gemini" });
});

test("an ACP thread's label names its ACP agent", () => {
  expect(threadAgentLabel({ agent: "acp", acp_agent: "gemini" })).toBe("ACP · gemini");
  expect(threadAgentLabel({ agent: "claude", acp_agent: null })).toBe("Claude");
});
