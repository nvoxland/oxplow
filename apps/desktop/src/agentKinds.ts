import type { AgentKind } from "./api.js";

/// Every agent oxplow can launch, in default display order. Adding an
/// agent here (plus the Rust AgentKind variant) is what surfaces it in
/// the Settings picker and the new-thread dialog.
export const ALL_AGENT_KINDS: AgentKind[] = ["claude", "codex", "opencode"];

const LABELS: Record<AgentKind, string> = {
  claude: "Claude",
  codex: "Codex",
  opencode: "opencode",
};

export function agentLabel(agent: AgentKind): string {
  return LABELS[agent] ?? agent;
}
