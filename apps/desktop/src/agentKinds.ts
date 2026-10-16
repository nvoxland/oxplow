import type { HarnessListing } from "./tauri-bridge/generated/bindings.js";

/// The agent harnesses come from the backend (`listAgentHarnesses`): the
/// ones the project's extensions declare, in priority order, each with its
/// title and whether the project enables it. Nothing here names a harness.

/// A harness as a person reads it: its title, else its key.
export function harnessLabel(harnesses: HarnessListing[], id: string): string {
  return harnesses.find((h) => h.id === id)?.title ?? id;
}

/// An agent session as its tab names it: its harness's title.
export function sessionLabel(harnesses: HarnessListing[], session: { harness: string }): string {
  return harnessLabel(harnesses, session.harness);
}

export interface AgentChoice {
  /// The harness's key.
  value: string;
  label: string;
}

/// The session picker's choices: each enabled harness, in priority order.
export function agentChoices(harnesses: HarnessListing[]): AgentChoice[] {
  return harnesses.filter((h) => h.enabled).map((h) => ({ value: h.id, label: h.title }));
}
