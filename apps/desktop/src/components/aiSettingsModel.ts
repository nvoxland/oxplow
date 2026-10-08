/// Pure view-model for the Settings → AI section (`AiSection.tsx`).
/// See `.context/ai-providers.md`.

import type { AiSettings, ProviderKindInfo, Role, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/// `kind` is a registered provider kind (`AiSettings.kinds`).
export type ProviderForm = { id: string; kind: string; baseUrl: string; key: string; editing: boolean };

/// A kind as a person reads it: its title, else its key.
export function kindLabel(kinds: ProviderKindInfo[], kind: string): string {
  return kinds.find((k) => k.kind === kind)?.title ?? kind;
}

/// A blank form, its kind the first one registered.
export function emptyProviderForm(kinds: ProviderKindInfo[]): ProviderForm {
  return { id: "", kind: kinds[0]?.kind ?? "", baseUrl: "", key: "", editing: false };
}

/// Why the form can't be saved yet, or null when it can.
export function providerFormError(form: ProviderForm, settings: AiSettings): string | null {
  const id = form.id.trim();
  if (!id) return "Name it (e.g. openrouter).";
  if (/\s/.test(id)) return "The name can't contain spaces.";
  if (!form.editing && settings.providers.some((p) => p.id === id)) return `There's already a provider named ${id}.`;
  const kind = settings.kinds.find((k) => k.kind === form.kind);
  if (!kind) return settings.kinds.length === 0 ? "No provider kinds are available." : "Pick a kind.";
  if (kind.defaultBaseUrl === null && !form.baseUrl.trim()) return `${kind.title} needs a base URL.`;
  return null;
}

const USED_FOR: Record<Role, string> = {
  main: "General reasoning",
  fast: "Cheap, quick generation",
  summarize: "Summaries of sessions, efforts and changes",
  embed: "Embeddings",
  decide: "Typed yes/no, choice and score questions (e.g. Jev)",
  review: "Second-opinion review by a different model",
};

export type RoleRow = {
  role: Role;
  usedFor: string;
  /// `provider · model`, or null when unassigned.
  assigned: string | null;
  problem: string | null;
  note: string | null;
  /// False when the project sets this role: editing the global value here
  /// wouldn't change what's used.
  editable: boolean;
  /// Hover text explaining why it's read-only, when it is.
  lockedReason: string | null;
};

export function roleRows(settings: AiSettings): RoleRow[] {
  const ids = new Set(settings.providers.map((p) => p.id));
  return settings.roles.map((r) => ({
    role: r.role,
    usedFor: USED_FOR[r.role],
    assigned: r.binding ? `${r.binding.provider} · ${r.binding.model}` : null,
    problem: r.binding && !ids.has(r.binding.provider) ? `Provider ${r.binding.provider} isn't set up.` : null,
    note: r.overridden ? "Set by this project" : null,
    editable: !r.overridden,
    lockedReason: r.overridden ? "This project sets this role in .oxplow/project.yaml (ai.roles); change it there." : null,
  }));
}

/// A model to test a provider with: one a role already uses on it, else "".
export function testModelFor(settings: AiSettings, providerId: string): string {
  return settings.roles.find((r) => r.binding?.provider === providerId)?.binding?.model ?? "";
}

/// Last 7 days of oxplow's own model calls, by role and caller.
export const USAGE_SQL = `SELECT role, caller, count(*) AS calls, sum(1 - ok) AS failed,
       sum(input_tokens) AS input_tokens, sum(output_tokens) AS output_tokens, max(at) AS last_at
FROM v_ai_call
WHERE at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-7 days')
GROUP BY role, caller
ORDER BY last_at DESC`;

export type UsageRow = {
  role: string;
  caller: string;
  calls: number;
  failed: number;
  inputTokens: number;
  outputTokens: number;
  lastAt: string;
};

export function usageRows(result: SqlQueryResult): UsageRow[] {
  return result.rows.map(([role, caller, calls, failed, inputTokens, outputTokens, lastAt]) => ({
    role: String(role),
    caller: String(caller),
    calls: Number(calls ?? 0),
    failed: Number(failed ?? 0),
    inputTokens: Number(inputTokens ?? 0),
    outputTokens: Number(outputTokens ?? 0),
    lastAt: String(lastAt ?? ""),
  }));
}
