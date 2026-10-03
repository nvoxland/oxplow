/// Settings → Integrations rows (P5.D4, `.context/providers.md`): an
/// extension provider's instance on this machine — where it stands and
/// what the person can do about it.

import type { CollectorView, ProviderInstanceView, SignInState } from "../tauri-bridge/generated/bindings.js";

export interface IntegrationRowModel {
  key: string;
  label: string;
  /** One line: the instance's state in words. */
  status: string;
  /** The toggle's label: Enable, Disable, or Enable again after an automatic disable. */
  enableLabel: "Enable" | "Disable" | "Enable again";
  /** Not approved on this machine: point at Data → Programs. */
  needsApproval: boolean;
  /** A state worth the error colour. */
  problem: boolean;
}

export function integrationRow(v: ProviderInstanceView, now: Date = new Date()): IntegrationRowModel {
  const s = v.health.state;
  let status: string;
  switch (s.state) {
    case "off":
      status = "Off";
      break;
    case "missing":
      status = s.reason;
      break;
    case "unapproved":
      // A global instance runs in every project, but consent is about the
      // code in this one.
      status =
        v.scope === "global"
          ? "Not approved in this project: its program is approved per project, under Data → Programs"
          : "Not approved on this machine: approve it under Data → Programs";
      break;
    case "unconfigured":
      status = `Config problems: ${s.problems.map((p) => `${p.path || "/"} ${p.message}`).join("; ")}`;
      break;
    case "checking":
      status = "Checking…";
      break;
    case "ready":
      status =
        v.health.meanInvokeMs === null ? "Ready" : `Ready · ~${Math.round(v.health.meanInvokeMs)} ms a call`;
      break;
    case "failing":
      status = `Failing (${v.health.consecutiveFailures} in a row): ${s.errors[s.errors.length - 1] ?? ""}`;
      break;
    case "disabled":
      status = `Disabled: ${s.reason}`;
      break;
  }
  // A rate limit says until when, and a read in progress what it's
  // doing — neither is a problem (P7.A4).
  const limited = v.health.rateLimitedUntil;
  if (limited && new Date(limited).getTime() > now.getTime()) {
    status = `${status} · rate limited until ${new Date(limited).toLocaleTimeString()}`;
  }
  if (v.health.activity) status = `${status} · ${v.health.activity}`;
  const enableLabel = s.state === "disabled" ? "Enable again" : v.enabled ? "Disable" : "Enable";
  return {
    key: v.instance,
    label: `${v.instance} · ${v.capability.replace(/_/g, " ")}${
      v.scope !== "global" ? "" : v.overridden ? " · yours, replaced by this project's" : " · yours, in every project"
    }`,
    status,
    enableLabel,
    needsApproval: !v.approved,
    problem: ["missing", "unapproved", "unconfigured", "failing", "disabled"].includes(s.state),
  };
}

/** A credential the person signs in for (P9.B3): where the sign-in stands
 *  and what its button does. */
export function signInLine(s: SignInState): { text: string; action: "Sign in" | "Sign in again"; signedIn: boolean; problem: boolean } {
  switch (s.state) {
    case "not_signed_in":
      return { text: "Not signed in", action: "Sign in", signedIn: false, problem: false };
    case "signed_in":
      return {
        // One that renews itself has no date worth showing.
        text: s.until ? `Signed in until ${new Date(s.until).toLocaleString()}` : "Signed in",
        action: "Sign in again",
        signedIn: true,
        problem: false,
      };
    case "sign_in_again":
      return { text: "Sign in again: the sign-in lapsed or was withdrawn", action: "Sign in again", signedIn: false, problem: true };
  }
}

/** A provider instance the project's work items can be filed on (P7.A2;
 *  by instance since P9.B1). */
export interface WorkItemsChoice {
  /** The instance's id — what `activeProviders.work_items` names
   *  (`oxplow` for oxplow's own tasks). */
  id: string;
  label: string;
  /** Running on this machine: new items can be filed on it now. */
  running: boolean;
}

/** oxplow's own tasks, then every work-items provider instance. */
export function workItemsChoices(views: ProviderInstanceView[]): WorkItemsChoice[] {
  const choices: WorkItemsChoice[] = [{ id: "oxplow", label: "oxplow's tasks", running: true }];
  for (const v of views) {
    if (v.capability !== "work_items" || choices.some((c) => c.id === v.instanceId)) continue;
    choices.push({ id: v.instanceId, label: `${v.instanceId} (${v.instance})`, running: v.health.state.state === "ready" });
  }
  return choices;
}

/** What to say about the active provider when filing on it can't work:
 *  nothing when it can. Never a fallback — a `create` fails naming it. */
export function activeProviderProblem(choices: WorkItemsChoice[], active: string): string | null {
  const choice = choices.find((c) => c.id === active);
  if (!choice) return `No enabled extension declares \`${active}\`: new work items can't be filed until one does, or choose another.`;
  if (!choice.running) return `\`${active}\` isn't running on this machine: new work items can't be filed until it is.`;
  return null;
}

/** One collector's line (P7.A3): what its reads have delivered and when
 *  it last read. */
export function collectorLine(c: CollectorView): { text: string; problem: boolean } {
  const records = `${c.records} ${c.records === 1 ? "record" : "records"}`;
  switch (c.status) {
    case "never":
      return { text: `${c.name}: not read yet`, problem: false };
    case "reading":
      return { text: `${c.name}: reading… · ${records}`, problem: false };
    case "error":
      return { text: `${c.name}: its last read failed (${c.error ?? "unknown"}) · ${records} · ${c.lastReadAt ?? ""}`, problem: true };
    default:
      return { text: `${c.name}: ${records} · last read ${c.lastReadAt ?? ""}`, problem: false };
  }
}
