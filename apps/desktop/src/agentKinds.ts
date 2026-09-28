import type { AgentKind } from "./api.js";
import type { AcpAgentListing } from "./tauri-bridge/generated/bindings.js";

/// Every agent oxplow can launch, in default display order. Adding an
/// agent here (plus the Rust AgentKind variant) is what surfaces it in
/// the Settings picker and the new-thread dialog.
export const ALL_AGENT_KINDS: AgentKind[] = ["claude", "codex", "opencode", "acp"];

const LABELS: Record<AgentKind, string> = {
  claude: "Claude",
  codex: "Codex",
  // The brand styles itself lowercase, but next to "Claude" / "Codex"
  // a lowercase entry reads as a bug — match the picker's casing.
  opencode: "OpenCode",
  // Agent Client Protocol: which agent is the thread's `acp_agent`.
  acp: "ACP",
};

export function agentLabel(agent: AgentKind): string {
  return LABELS[agent] ?? agent;
}

/// A thread's agent as the agent tab names it: "ACP · gemini" for an ACP
/// thread.
export function threadAgentLabel(thread: { agent: AgentKind; acp_agent?: string | null }): string {
  return thread.agent === "acp" && thread.acp_agent ? `ACP · ${thread.acp_agent}` : agentLabel(thread.agent);
}

export interface AgentChoice {
  /// `claude` for a terminal agent, `acp:<name>` for an ACP agent.
  value: string;
  label: string;
}

/// The new-thread picker's choices: each enabled terminal agent, and when
/// ACP is enabled one per ACP agent, flagged when it can't start yet.
export function agentChoices(enabled: AgentKind[], acpAgents: AcpAgentListing[]): AgentChoice[] {
  return enabled.flatMap((kind): AgentChoice[] => {
    if (kind !== "acp") return [{ value: kind, label: agentLabel(kind) }];
    return acpAgents.map((a) => {
      const note = !a.resolvedPath ? " (not installed)" : !a.approved ? " (needs approval)" : "";
      return { value: `acp:${a.name}`, label: `ACP · ${a.name}${note}` };
    });
  });
}

/// A choice value back into what thread creation takes.
export function parseAgentChoice(value: string): { agent: AgentKind; acpAgent: string | null } {
  return value.startsWith("acp:")
    ? { agent: "acp", acpAgent: value.slice("acp:".length) }
    : { agent: value as AgentKind, acpAgent: null };
}
