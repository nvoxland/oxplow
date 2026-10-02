import type { Grants, ProviderEffect } from "../tauri-bridge/generated/bindings.js";

/** One rendering of what a program's or provider's grants and
 *  declarations change, for the Extensions review (prefixed with the
 *  program) and the provider's approval row in Settings → Data. */

export const listed = (xs: string[]) => (xs.length > 0 ? xs.join(", ") : "none");

/** A program's grants: "runs x · reaches y · reads z". */
export function grantsLine(g: Grants): string {
  return `runs ${[g.entry, ...g.args].join(" ")} · reaches ${listed(g.hosts)} · reads ${listed(g.credentials)}`;
}

/** What a program's grants became, as phrases ("now reaches x (was y)"). */
export function grantChanges(before: Grants | null, after: Grants | null): string[] {
  if (!before || !after) return [];
  const out: string[] = [];
  if (before.entry !== after.entry || before.runtime !== after.runtime || listed(before.args) !== listed(after.args)) {
    out.push(`now runs ${[after.entry, ...after.args].join(" ")} (was ${[before.entry, ...before.args].join(" ")})`);
  }
  if (listed(before.hosts) !== listed(after.hosts)) out.push(`now reaches ${listed(after.hosts)} (was ${listed(before.hosts)})`);
  if (listed(before.credentials) !== listed(after.credentials)) out.push(`now reads ${listed(after.credentials)} (was ${listed(before.credentials)})`);
  if (listed(before.env) !== listed(after.env)) out.push(`now reads env ${listed(after.env)} (was ${listed(before.env)})`);
  return out;
}

const destructive = (c: { after: unknown }) =>
  (c.after as { confirm?: string } | null)?.confirm === "destructive" ? " (destructive)" : "";

/** A provider's changes as phrases: everything it declares when it's new,
 *  else its grants, commands, MCP tools and features that differ — and, when none of
 *  those shows a change, where its declarations first differ. */
export function providerChanges(e: ProviderEffect): string[] {
  if (e.change === "added") {
    const out = [
      `added — ${e.after ? grantsLine(e.after) : "no grants"}`,
      `commands: ${listed(e.commands.map((c) => `${c.name}${destructive(c)}`))}`,
    ];
    // Behind oxplow's MCP adapter: the server's tools, as pinned.
    if (e.tools.length > 0) out.push(`MCP tools: ${listed(e.tools.map((t) => t.name))}`);
    return out;
  }
  if (e.change === "removed") return ["removed"];
  const out = grantChanges(e.before, e.after);
  for (const c of e.commands.filter((c) => c.change !== "unchanged")) {
    out.push(`command \`${c.name}\` ${c.change}${c.change === "removed" ? "" : destructive(c)}`);
  }
  for (const t of e.tools.filter((t) => t.change !== "unchanged")) {
    out.push(`MCP tool \`${t.name}\` ${t.change}`);
  }
  if (JSON.stringify(e.featuresBefore) !== JSON.stringify(e.featuresAfter)) {
    out.push(`features now ${JSON.stringify(e.featuresAfter)} (were ${JSON.stringify(e.featuresBefore)})`);
  }
  if (out.length === 0 && e.change === "changed") out.push(e.firstDifference ?? "its declarations changed");
  return out;
}
