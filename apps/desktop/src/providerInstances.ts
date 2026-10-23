// What Settings → Integrations shows of a provider instance, assembled
// from the models `v_provider_instance`, `v_provider_credential` and
// `v_provider_collector` (the instances themselves are changed by the
// `oxplow.provider.*` commands). The Integrations component reads and
// writes through these until it is a foundation component.
import type { SqlCell } from "./tauri-bridge/generated/bindings.js";

export type Scope = "project" | "global";

export type ConfigProblem = { path: string; message: string };

export type InstanceState =
  | { state: "off" }
  | { state: "missing"; reason: string }
  | { state: "refused"; reason: string }
  | { state: "unconfigured"; problems: ConfigProblem[] }
  | { state: "checking" }
  | { state: "ready" }
  | { state: "failing"; errors: string[] }
  | { state: "disabled"; reason: string };

export type SignInState =
  | { state: "not_signed_in" }
  | { state: "signed_in"; until: string | null }
  | { state: "sign_in_again" };

export type InstanceCredential = {
  name: string;
  set: boolean;
  signIn: SignInState | null;
  redirectPort: number | null;
};

export type InstanceHealth = {
  state: InstanceState;
  consecutiveFailures: number;
  lastOkAt: string | null;
  meanInvokeMs: number | null;
  rateLimitedUntil: string | null;
  activity: string | null;
};

export type CollectorView = {
  name: string;
  entity: string;
  status: string;
  error: string | null;
  lastReadAt: string | null;
  records: number;
};

export type ProviderInstanceView = {
  instance: string;
  scope: Scope;
  overridden: boolean;
  extension: string;
  provider: string;
  instanceId: string;
  capability: string;
  enabled: boolean;
  config: unknown;
  configSchema: unknown;
  credentials: InstanceCredential[];
  health: InstanceHealth;
  collectors: CollectorView[];
};

export type BegunSignIn = { url: string; signIn: number };

export const INSTANCES_SQL = "SELECT * FROM v_provider_instance ORDER BY instance";
export const CREDENTIALS_SQL = "SELECT * FROM v_provider_credential ORDER BY instance, name";
export const COLLECTORS_SQL = "SELECT * FROM v_provider_collector ORDER BY instance, name";

type Result = { columns: string[]; rows: SqlCell[][] };

function records(result: Result): Array<Record<string, SqlCell>> {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((c, i) => [c, row[i] ?? null])));
}

const text = (v: SqlCell): string | null => (v === null ? null : String(v));

function parse(v: SqlCell): unknown {
  try {
    return typeof v === "string" ? JSON.parse(v) : null;
  } catch {
    return null;
  }
}

function stateOf(kind: string, detail: string | null): InstanceState {
  const reason = detail ?? "";
  switch (kind) {
    case "missing":
    case "refused":
    case "disabled":
      return { state: kind, reason };
    case "unconfigured":
      return {
        state: "unconfigured",
        problems: reason
          .split("; ")
          .filter((p) => p !== "")
          .map((p) => {
            const at = p.indexOf(": ");
            return p.startsWith("/") && at > 0
              ? { path: p.slice(0, at), message: p.slice(at + 2) }
              : { path: "", message: p };
          }),
      };
    case "failing":
      return { state: "failing", errors: reason.split("; ").filter((e) => e !== "") };
    case "checking":
    case "ready":
    case "off":
      return { state: kind };
    default:
      return { state: "off" };
  }
}

/** The views the three models' rows make. */
export function assembleInstances(instances: Result, credentials: Result, collectors: Result): ProviderInstanceView[] {
  const creds = records(credentials);
  const colls = records(collectors);
  return records(instances).map((r) => {
    const instance = String(r.instance);
    return {
      instance,
      scope: r.scope === "global" ? "global" : "project",
      overridden: r.overridden === 1,
      extension: String(r.extension),
      provider: String(r.provider),
      instanceId: String(r.instance_id),
      capability: String(r.capability),
      enabled: r.enabled === 1,
      config: parse(r.config),
      configSchema: parse(r.config_schema),
      credentials: creds
        .filter((c) => c.instance === instance)
        .map((c) => ({
          name: String(c.name),
          set: c.is_set === 1,
          signIn:
            c.signs_in !== 1
              ? null
              : c.sign_in_state === "signed_in"
                ? { state: "signed_in", until: text(c.signed_in_until) }
                : c.sign_in_state === "sign_in_again"
                  ? { state: "sign_in_again" }
                  : { state: "not_signed_in" },
          redirectPort: typeof c.redirect_port === "number" ? c.redirect_port : null,
        })),
      health: {
        state: stateOf(String(r.state), text(r.detail)),
        consecutiveFailures: Number(r.consecutive_failures ?? 0),
        lastOkAt: text(r.last_ok_at),
        meanInvokeMs: typeof r.mean_invoke_ms === "number" ? r.mean_invoke_ms : null,
        rateLimitedUntil: text(r.rate_limited_until),
        activity: text(r.activity),
      },
      collectors: colls
        .filter((c) => c.instance === instance)
        .map((c) => ({
          name: String(c.name),
          entity: String(c.entity),
          status: String(c.status),
          error: text(c.error),
          lastReadAt: text(c.last_read_at),
          records: Number(c.records ?? 0),
        })),
    };
  });
}
