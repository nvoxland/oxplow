import type { AcpAgentListing, HarnessListing } from "./tauri-bridge/generated/bindings.js";

/// The agent harnesses come from the backend (`listAgentHarnesses`): the
/// ones the project's extensions declare, in priority order, each with its
/// title, whether the project enables it, and whether it runs an ACP agent
/// in a chat. Nothing here names a harness.

/// A harness as a person reads it: its title, else its key.
export function harnessLabel(harnesses: HarnessListing[], id: string): string {
  return harnesses.find((h) => h.id === id)?.title ?? id;
}

/// An agent session as its tab names it: "ACP · gemini" for a session
/// running an ACP agent.
export function sessionLabel(harnesses: HarnessListing[], session: { harness: string; acpAgent: string | null }): string {
  const title = harnessLabel(harnesses, session.harness);
  return session.acpAgent ? `${title} · ${session.acpAgent}` : title;
}

export interface AgentChoice {
  /// `<harness>`, or `<harness>:<acp agent>` for a chat harness.
  value: string;
  label: string;
}

/// The session picker's choices: each enabled harness, in priority order,
/// and for a chat harness one per ACP agent, flagged when it can't start
/// yet.
export function agentChoices(harnesses: HarnessListing[], acpAgents: AcpAgentListing[]): AgentChoice[] {
  return harnesses
    .filter((h) => h.enabled)
    .flatMap((h): AgentChoice[] => {
      if (!h.chat) return [{ value: h.id, label: h.title }];
      return acpAgents.map((a) => {
        const note = !a.resolvedPath ? " (not installed)" : !a.approved ? " (needs approval)" : "";
        return { value: `${h.id}:${a.name}`, label: `${h.title} · ${a.name}${note}` };
      });
    });
}

/// A choice value back into what opening a session takes.
export function parseAgentChoice(value: string): { harness: string; acpAgent: string | null } {
  const at = value.indexOf(":");
  return at < 0 ? { harness: value, acpAgent: null } : { harness: value.slice(0, at), acpAgent: value.slice(at + 1) };
}
