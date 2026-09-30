/// Settings → Integrations rows (P5.D4, `.context/providers.md`): an
/// extension provider's instance on this machine — where it stands and
/// what the person can do about it.

import type { ProviderInstanceView } from "../tauri-bridge/generated/bindings.js";

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

export function integrationRow(v: ProviderInstanceView): IntegrationRowModel {
  const s = v.health.state;
  let status: string;
  switch (s.state) {
    case "off":
      status = "Off";
      break;
    case "missing":
      status = "No enabled extension declares it";
      break;
    case "unapproved":
      status = "Not approved on this machine: approve it under Data → Programs";
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
  const enableLabel = s.state === "disabled" ? "Enable again" : v.enabled ? "Disable" : "Enable";
  return {
    key: v.instance,
    label: `${v.instance} · ${v.capability.replace(/_/g, " ")}`,
    status,
    enableLabel,
    needsApproval: !v.approved,
    problem: ["missing", "unapproved", "unconfigured", "failing", "disabled"].includes(s.state),
  };
}

/** The config text read as JSON: its object, or why it isn't one. */
export interface ParsedConfig {
  value: Record<string, unknown> | null;
  error: string | null;
}

/// The config textarea's text as the instance's config: a JSON object.
export function parseConfig(text: string): ParsedConfig {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (e) {
    return { value: null, error: `Not JSON: ${e instanceof Error ? e.message : String(e)}` };
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return { value: null, error: "The config must be a JSON object" };
  }
  return { value: value as Record<string, unknown>, error: null };
}
