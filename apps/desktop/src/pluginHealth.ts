/**
 * Each plugin contribution's health on this machine (P7.C1–C3,
 * `.context/extensions.md` → "Health, disable and repair"): read from
 * `v_plugin_health`. Three failures in a row disable a provider instance
 * or collector until a person runs `plugin.enable` (Enable Again); the
 * disable files a repair work item, which Repair with the Agent mentions
 * in the agent's input — filled, never sent.
 */
import { useCallback, useEffect, useState } from "react";

import { formatContextMention } from "./agent-context-ref.js";
import { insertIntoAgent } from "./agent-input-bus.js";
import { querySql } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { personCommands, type PersonCommands } from "./personCommands.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One contribution's `v_plugin_health` row. */
export interface PluginHealth {
  /** The extension. */
  plugin: string;
  /** Its provider's or collector's id. */
  contribution: string;
  /** `provider` or `collector`. */
  kind: string;
  /** `ok`, `failing` or `disabled`. */
  state: string;
  /** Why it's disabled. */
  reason: string | null;
  /** Failures in a row. */
  failures: number;
  lastError: string | null;
  meanMs: number | null;
  /** Events about it no consumer could take. */
  deadLetters: number;
  /** False once a scheduled run is overdue. */
  fresh: boolean;
  /** The open repair work item's ref. */
  repairItem: string | null;
}

/** How Settings → Extensions shows one contribution. */
export interface HealthLine {
  text: string;
  tone: "ok" | "warn" | "error";
  canEnable: boolean;
  repairItem: string | null;
}

export function pluginHealthFromResult(result: SqlQueryResult): PluginHealth[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    const text = (name: string) => (at(name) == null ? null : String(at(name)));
    return {
      plugin: String(at("plugin")),
      contribution: String(at("contribution")),
      kind: String(at("kind")),
      state: String(at("state")),
      reason: text("reason"),
      failures: Number(at("consecutive_failures") ?? 0),
      lastError: text("last_error"),
      meanMs: at("mean_ms") == null ? null : Number(at("mean_ms")),
      deadLetters: Number(at("dead_letters") ?? 0),
      fresh: Boolean(Number(at("fresh") ?? 1)),
      repairItem: text("repair_item"),
    };
  });
}

/** One extension's contributions. */
export function healthOf(rows: readonly PluginHealth[], extension: string): PluginHealth[] {
  return rows.filter((h) => h.plugin === extension);
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function healthLine(h: PluginHealth): HealthLine {
  if (h.state === "disabled") {
    return { text: `Disabled: ${h.reason ?? "no reason recorded"}`, tone: "error", canEnable: true, repairItem: h.repairItem };
  }
  if (h.state === "failing") {
    return {
      text: `Failing (${h.failures} in a row)${h.lastError ? `: ${h.lastError}` : ""}`,
      tone: "warn",
      canEnable: false,
      repairItem: null,
    };
  }
  const notes = [
    h.meanMs != null ? `${Math.round(h.meanMs)} ms on average` : null,
    h.fresh ? null : "missed its schedule",
    h.deadLetters > 0 ? `${plural(h.deadLetters, "event", "events")} undelivered` : null,
  ].filter((n): n is string => n !== null);
  return {
    text: ["OK", ...notes].join(" · "),
    tone: h.fresh && h.deadLetters === 0 ? "ok" : "warn",
    canEnable: false,
    repairItem: null,
  };
}

/** The one line Repair with the Agent puts in the agent's input. */
export function repairMention(repairItem: string): string {
  return `Repair the extension described in ${formatContextMention({ kind: "ref", ref: repairItem }).trimEnd()} — read it first.`;
}

/** Fill the agent's input with the repair mention. It is never sent: the
 *  person reads it and presses Enter. */
export function repairWithAgent(repairItem: string, insert: (text: string) => void = insertIntoAgent): void {
  insert(repairMention(repairItem));
}

/** Enable Again: `plugin.enable` as the person. */
export function enableAgain(h: PluginHealth, commands: PersonCommands = personCommands): Promise<boolean> {
  return commands.run(`Enable ${h.plugin}/${h.contribution}`, "plugin.enable", {
    plugin: h.plugin,
    kind: h.kind,
    contribution: h.contribution,
  });
}

/** Every contribution's health, and what was read. */
export async function readPluginHealth(): Promise<{ health: PluginHealth[]; reads: Reads }> {
  const res = await querySql(
    `SELECT plugin, contribution, kind, state, reason, consecutive_failures, last_error, mean_ms,
            dead_letters, fresh, repair_item
       FROM v_plugin_health ORDER BY plugin, contribution`,
    [],
    1_000,
  );
  return { health: pluginHealthFromResult(res), reads: res.reads };
}

/** Every contribution's health, re-read when it changes. */
export function usePluginHealth(): PluginHealth[] {
  const [health, setHealth] = useState<PluginHealth[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readPluginHealth()
      .then((r) => {
        setHealth(r.health);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read extension health", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return health;
}
