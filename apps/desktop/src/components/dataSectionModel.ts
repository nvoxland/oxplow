/// Pure view model for Settings → Data (DataSection.tsx): what data the
/// semantic layer holds, who provides it, and how much. See
/// `.context/semantic-layer.md`.
import type { DataEntity, ProjectProgram, ProviderImpact } from "../tauri-bridge/generated/bindings.js";

export interface EntityRowModel {
  name: string;
  owner: string;
  description: string;
  /** Formatted row count, or a status when there is none. */
  rows: string;
  /** Why there's no count, on hover. */
  rowsTitle?: string;
  available: boolean;
}

/** One model's row count as the Data section holds it: absent while it's
 *  still being counted. */
export type EntityCount = { rows: number } | { error: string };

/// One row per entity: core first, then by owner and name. An entity
/// whose source hasn't synced (`declared`) shows "Not synced yet". Counts
/// arrive one model at a time after the list shows (tsk1065): a model too
/// big to count in time reads as a dash with why, not a failed list.
export function entityRows(entities: DataEntity[], counts: Record<string, EntityCount>): EntityRowModel[] {
  const fmt = new Intl.NumberFormat();
  return entities
    .map((e) => {
      const available = e.kind !== "declared";
      const count = counts[e.name];
      return {
        name: e.name,
        owner: e.owner,
        description: e.description,
        rows: !available ? "Not synced yet" : !count ? "Counting…" : "rows" in count ? fmt.format(count.rows) : "—",
        rowsTitle: available && count && "error" in count ? `Couldn't count it: ${count.error}` : undefined,
        available,
      };
    })
    .sort((a, b) => {
      const core = Number(b.owner === "core") - Number(a.owner === "core");
      return core || a.owner.localeCompare(b.owner) || a.name.localeCompare(b.name);
    });
}

/// "12 entities · 3 from extensions" for the section header.
export function entitySummary(rows: EntityRowModel[]): string {
  const ext = rows.filter((r) => r.owner !== "core").length;
  const noun = rows.length === 1 ? "entity" : "entities";
  return ext > 0 ? `${rows.length} ${noun} · ${ext} from extensions` : `${rows.length} ${noun}`;
}

export interface ProgramRowModel {
  key: string;
  label: string;
  /** What runs, as a command line. */
  command: string;
  status: string;
  approved: boolean;
  /** The Approve button's hover: what consenting means. */
  approveTitle: string;
  /** The row offers to show its entry: a bundled program's files aren't
   *  in the project, and an AI provider's script is what's approved. */
  showsSource: boolean;
}

/// A program the project's config would run (an `exec` collector, an ACP
/// agent, an extension's provider, effect or component): unapproved ones don't run until a person approves them here.
/// A shared extension's advisories are shown as what they'd say.
export function programRow(p: ProjectProgram): ProgramRowModel {
  if (p.kind === "advisories") {
    const command = p.args.join("\n");
    return {
      key: `${p.kind}:${p.name}`,
      label: `Advisories from ${p.name}`,
      command,
      status: p.approved ? "Approved on this machine" : "Not approved: they won't reach your agent",
      approved: p.approved,
      approveTitle: `Lets these queries' results into your agent's context (${p.program}). Approve only if you trust this extension; any change needs approval again.`,
      showsSource: false,
    };
  }
  if (p.kind === "provider") {
    // Its grants are part of what's approved: env names, keychain
    // credentials, hosts, and every file of its extension.
    // A server reached by url (`remote`) runs nothing of its own here:
    // the url, then the adapter's mapping, pinned tools and bearer.
    // An agent harness launches the person's agent sessions: what its
    // launch answers runs in their terminal, outside its own grants.
    const harness = p.capability === "agent_harness";
    const command = [
      ...(p.remote ? [`MCP server at ${p.program}`, `with: ${p.args.join(" ")}`] : [[p.program, ...p.args].join(" ")]),
      ...(p.env.length > 0 ? [`env: ${p.env.join(", ")}`] : []),
      ...(p.credentials.length > 0 ? [`credentials: ${p.credentials.join(", ")}`] : []),
      ...(p.network.length > 0 ? [`reaches: ${p.network.join(", ")}`] : []),
      ...(p.scopes.length > 0 ? [`calls oxplow's: ${p.scopes.join(", ")}`] : []),
      ...(harness ? ["launches agent sessions: the command it answers runs in your terminal"] : []),
    ].join("\n");
    return {
      key: `${p.kind}:${p.name}`,
      label: harness ? `Agent harness ${p.name}` : `External provider ${p.name}`,
      command,
      status: p.approved ? "Approved on this machine" : "Not approved: it won't run",
      approved: p.approved,
      approveTitle: harness
        ? `Runs ${p.program} as an agent harness with these grants, approving every file in ${p.tree ?? "its extension"}. It launches your agent sessions: the command it answers runs in your terminal with your rights, and it's given each session's bearer and system prompt. Approve only if you trust this extension; any change needs approval again.`
        : p.remote
        ? `Lets oxplow's MCP adapter talk to ${p.program} as an external provider with these grants, approving that address and every file in ${p.tree ?? "its extension"} (its mapping, its pinned tools, its declarations). The server runs elsewhere: its code isn't part of this approval, and oxplow refuses it when its tools stop matching the pinned ones. Approve only if you trust this extension and that server; any change here needs approval again.`
        : `Runs ${p.program} as a long-lived external provider with these grants, approving every file in ${p.tree ?? "its extension"} (its declarations included). Approve only if you trust this extension; any change needs approval again.`,
      showsSource: false,
    };
  }
  if (p.kind === "effect") {
    // A bundled extension's files come with oxplow: named by where they
    // sit in it, and approved again when a new oxplow changes them.
    const bundled = p.tree?.startsWith("bundled:") ?? false;
    const ext = bundled ? p.tree!.slice("bundled:".length) : null;
    const entry = ext && p.program.startsWith(`bundled:${ext}/`) ? p.program.slice(`bundled:${ext}/`.length) : p.program;
    return {
      key: `${p.kind}:${p.name}`,
      label: `Effect ${p.name}`,
      command: ext ? `${entry}, part of ${ext} (comes with oxplow)` : p.program,
      status: p.approved ? "Approved on this machine" : "Not approved: it won't run",
      approved: p.approved,
      approveTitle: ext
        ? `Runs ${entry} from ${ext}, which comes with oxplow, on events logged after you approve, composing commands with an agent's rights (a command that asks becomes a proposal for you), approving every file of ${ext}. A new oxplow that changes it asks again.`
        : `Runs ${p.program} on events logged after you approve, composing commands with an agent's rights (a command that asks becomes a proposal for you), approving every file in ${p.tree ?? "its extension"}. Approve only if you trust this extension; any change needs approval again.`,
      showsSource: bundled,
    };
  }
  if (p.kind === "ai-provider") {
    // Its calls carry the person's key and prompts to where it sends them;
    // a shipped one comes with oxplow and asks again when a new oxplow
    // changes it.
    const bundled = p.tree?.startsWith("bundled:") ?? false;
    const ext = bundled ? p.tree!.slice("bundled:".length) : null;
    const entry = ext && p.program.startsWith(`bundled:${ext}/`) ? p.program.slice(`bundled:${ext}/`.length) : p.program;
    const sendsTo = p.network.length > 0 ? p.network.join(", ") : "the base URL each provider you configure names";
    return {
      key: `${p.kind}:${p.name}`,
      label: `AI provider ${p.name}`,
      command: [ext ? `${entry}, part of ${ext} (comes with oxplow)` : p.program, `sends to: ${sendsTo}`].join("\n"),
      status: p.approved ? "Approved on this machine" : "Not approved: calls through it fail",
      approved: p.approved,
      approveTitle: ext
        ? `Lets ${entry}, which comes with oxplow, shape the calls to ${sendsTo} that carry your key and prompts for the AI roles that use it. A new oxplow that changes it asks again.`
        : `Lets ${p.program} shape the calls to ${sendsTo} that carry your key and prompts for the AI roles that use it. Approve only if you trust this extension; any change to the script needs approval again.`,
      showsSource: true,
    };
  }
  if (p.kind === "effort-policy") {
    // It reacts to core's events by composing effort commands; its
    // approval covers every file of its extension (the manifest's `needs`
    // say what it reads).
    return {
      key: `${p.kind}:${p.name}`,
      label: `Effort policy ${p.name}`,
      command: [p.program, ...(p.scopes.length > 0 ? [`reads oxplow's: ${p.scopes.join(", ")}`] : [])].join("\n"),
      status: p.approved ? "Approved on this machine" : "Not approved: choosing it opens and closes nothing",
      approved: p.approved,
      approveTitle: `Runs ${p.program} on the events an effort policy is offered while it's the project's effort policy, composing commands with an agent's rights (a command that asks becomes a proposal for you), approving every file in ${p.tree ?? "its extension"}. Approve only if you trust this extension; any change needs approval again.`,
      showsSource: true,
    };
  }
  if (p.kind === "component") {
    // Unapproved it still renders and queries its lenses; only acting —
    // running its commands with the viewer's rights — waits (tsk960).
    const commands = p.commands.join(", ");
    return {
      key: `${p.kind}:${p.name}`,
      label: `Component ${p.name}`,
      command: `${p.program}\nmay run: ${commands}`,
      status: p.approved ? "Approved on this machine" : "Not approved: it shows and reads, but can't act",
      approved: p.approved,
      approveTitle: `Lets the custom component ${p.name} run ${commands} with your rights when you use it (a command that asks still asks you), approving every file of its bundle (${p.program}). Approve only if you trust this extension; a changed bundle or command list needs approval again.`,
      showsSource: false,
    };
  }
  const command = [...(p.env ?? []), p.program, ...p.args].join(" ");
  const what = p.kind === "collector" ? "Collector" : "ACP agent";
  return {
    key: `${p.kind}:${p.name}`,
    label: `${what} ${p.name}`,
    command,
    status: p.approved ? "Approved on this machine" : "Not approved: it won't run",
    approved: p.approved,
    approveTitle: `Runs ${command} from this project's config on this machine. Approve only if you trust this repo; a changed program or arguments need approval again.`,
    showsSource: false,
  };
}

/** What approving a provider would change, as lines: the
 *  server's wording (`extension_impact::approval_lines`). */
export function providerImpactLines(e: ProviderImpact): string[] {
  return e.lines;
}

/** A provider's Approve waits until its declaration diff has loaded: a
 *  person approves what they saw change. Other programs approve as listed. */
export function canApprove(p: ProjectProgram, impact: ProviderImpactState | undefined): boolean {
  return p.kind !== "provider" || (impact !== undefined && impact !== "loading" && !("error" in impact));
}

/** A provider's declaration diff as the Data section holds it. */
export type ProviderImpactState = ProviderImpact | "loading" | { error: string };

/** What `oxplow.effect.backfill` answers (P9.D5). */
export interface BackfillResult {
  planned: number;
  ran: number;
  skipped: number;
  proposed: number;
  failed: number;
  remaining: number;
  stopped?: string;
}

/** What a person is told before a backfill runs: how many past events the
 *  effect never reacted to (`oxplow.effect.backfill_plan`), and what running it
 *  means. */
export function backfillAsk(effect: string, planned: number): string {
  if (planned === 0) return `${effect} has reacted to every matching event: nothing to backfill.`;
  return planned === 1
    ? `${effect} never reacted to 1 matching event. Backfilling runs it on that event, as it is now; it may call outside oxplow.`
    : `${effect} never reacted to ${planned} matching events. Backfilling runs it on each, oldest first, as it is now; it may call outside oxplow for every one.`;
}

/** A backfill's Run button: one run reacts to at most `batch` events. */
export function backfillRunLabel(planned: number, batch: number): string {
  if (planned > batch) return `Run on the first ${batch} of ${planned}`;
  return `Run on ${planned} ${planned === 1 ? "event" : "events"}`;
}

/** How a backfill went, in a sentence or two. */
export function backfillDone(r: BackfillResult): string {
  const also = [
    r.failed > 0 ? `${r.failed} failed` : null,
    r.proposed > 0 ? `${r.proposed} wait for your approval` : null,
    r.skipped > 0 ? `${r.skipped} skipped` : null,
    !r.stopped && r.remaining > 0 ? `${r.remaining} remain — run it again for the rest` : null,
  ].filter((p): p is string => p !== null);
  const head = `Reacted to ${r.ran} of ${r.planned} ${r.planned === 1 ? "event" : "events"}`;
  const body = also.length > 0 ? `${head}; ${also.join(", ")}.` : `${head}.`;
  return r.stopped ? `${body} Stopped: ${r.stopped}.` : body;
}
