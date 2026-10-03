/// "Integrations" section body for SettingsPage (P5.D4): which provider
/// the project's work items are filed on (P7.A2: `activeProviders`,
/// written with `config.set` as the person), then each extension
/// provider's instance on this machine — its state, its config (a form
/// from the provider's `config_schema`, P6.B2), Check, and Enable /
/// Disable. Between the two, the core components an extension replaces
/// (P9.A1), each with a switch back to oxplow's own (`replacementsOff`).
/// Enabling checks first;
/// an unapproved provider is approved under Data → Programs. See
/// `.context/providers.md`.
///
/// Usability contract (.context/usability.md): inline edits, Escape
/// resets an edit, the actions disabled while a field has a problem,
/// failures in opErrorsStore.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  checkProviderInstance,
  effectiveConfig,
  listExtensions,
  listProviderInstances,
  runCommand,
  setInstanceCredential,
  setProviderInstance,
  subscribeOxplowEvents,
  type ProviderInstanceView,
} from "../api.js";
import { REPLACEABLE_LABELS } from "../lens/useReplacement.js";
import type { UiReplacement } from "../tauri-bridge/generated/bindings.js";
import { CredentialRow } from "./ExtensionsSection.js";
import { activeProviderProblem, collectorLine, integrationRow, workItemsChoices } from "./integrationsModel.js";
import { SchemaForm } from "./SchemaForm/SchemaForm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function IntegrationsSection() {
  const [views, setViews] = useState<ProviderInstanceView[] | null>(null);
  const [active, setActive] = useState<string>("oxplow");
  const [replaced, setReplaced] = useState<UiReplacement[]>([]);
  const [off, setOff] = useState<string[]>([]);

  const refresh = useCallback(async () => {
    try {
      const [listed, settings, extensions] = await Promise.all([
        listProviderInstances(),
        effectiveConfig(),
        listExtensions(null),
      ]);
      setViews(listed);
      setReplaced(extensions.filter((e) => e.enabled).flatMap((e) => e.ui.replacements));
      const turnedOff = settings.find((s) => s.key === "replacementsOff")?.value;
      setOff(Array.isArray(turnedOff) ? turnedOff.map(String) : []);
      const chosen = settings.find((s) => s.key === "activeProviders")?.value as Record<string, unknown> | null;
      setActive(typeof chosen?.work_items === "string" ? chosen.work_items : "oxplow");
    } catch (e) {
      recordOpError({ label: "List integrations", message: String(e) });
      setViews([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") void refresh();
    });
  }, [refresh]);

  if (views === null) return <div style={mutedStyle}>Loading…</div>;
  if (views.length === 0) {
    return (
      <div style={mutedStyle} data-testid="integrations-empty">
        No extension declares a provider. A private extension can bring one (its <code>providers:</code>).
      </div>
    );
  }
  return (
    <div data-testid="integrations-section">
      <ActiveWorkItems views={views} active={active} onChosen={setActive} />
      <Replacements replaced={replaced} off={off} onChanged={setOff} />
      {views.map((v) => (
        <IntegrationRow key={v.instance} view={v} onChanged={setViews} onCredentialChanged={() => void refresh()} />
      ))}
    </div>
  );
}

/** Which provider new work items are filed on: a person's choice
 *  (`activeProviders` is a person-only key), so the click is the
 *  confirmation `config.set` asks for. */
function ActiveWorkItems({
  views,
  active,
  onChosen,
}: {
  views: ProviderInstanceView[];
  active: string;
  onChosen(provider: string): void;
}) {
  const choices = workItemsChoices(views);
  if (choices.length < 2) return null;
  const problem = activeProviderProblem(choices, active);
  async function choose(provider: string) {
    try {
      if (provider === "oxplow") {
        await runCommand("config.unset", { key: "activeProviders" }, true);
      } else {
        await runCommand("config.set", { key: "activeProviders", value: { work_items: provider } }, true);
      }
      onChosen(provider);
      showToast({ message: `New work items are filed on ${provider}.` });
    } catch (e) {
      recordOpError({ label: "Choose where work items are filed", message: e instanceof Error ? e.message : String(e) });
    }
  }
  return (
    <fieldset data-testid="integrations-active-work-items" style={fieldsetStyle}>
      <legend style={mutedStyle}>Active for work items — new items are filed here</legend>
      {choices.map((c) => (
        <label key={c.id} style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <input
            type="radio"
            name="active-work-items"
            data-testid={`integrations-active-${c.id}`}
            checked={active === c.id}
            onChange={() => void choose(c.id)}
          />
          {c.label}
          {c.running ? null : <span style={mutedStyle}>· not running</span>}
        </label>
      ))}
      {problem ? (
        <div style={errorStyle} data-testid="integrations-active-problem">
          {problem}
        </div>
      ) : null}
    </fieldset>
  );
}

/** The core components an extension replaces, each with a switch back to
 *  oxplow's own: `replacementsOff`, a person-only key like
 *  `activeProviders`, so the click is the confirmation `config.set` asks
 *  for. A replacement shows only while its extension's provider is the
 *  active one. */
function Replacements({
  replaced,
  off,
  onChanged,
}: {
  replaced: UiReplacement[];
  off: string[];
  onChanged(off: string[]): void;
}) {
  const targets = [...new Set(replaced.map((r) => r.target))].sort();
  if (targets.length === 0) return null;
  async function set(target: string, keepOxplows: boolean) {
    const next = keepOxplows ? [...new Set([...off, target])].sort() : off.filter((t) => t !== target);
    try {
      if (next.length === 0) {
        await runCommand("config.unset", { key: "replacementsOff" }, true);
      } else {
        await runCommand("config.set", { key: "replacementsOff", value: next }, true);
      }
      onChanged(next);
    } catch (e) {
      recordOpError({ label: "Choose whose component shows", message: e instanceof Error ? e.message : String(e) });
    }
  }
  return (
    <fieldset data-testid="integrations-replacements" style={fieldsetStyle}>
      <legend style={mutedStyle}>Replaced components — an extension's own, while its provider is the active one</legend>
      {targets.map((target) => {
        const label = REPLACEABLE_LABELS[target] ?? target;
        const by = replaced.filter((r) => r.target === target).map((r) => r.extension);
        return (
          <label
            key={target}
            data-testid={`integrations-replacement-${target}`}
            style={{ display: "flex", gap: 6, alignItems: "center" }}
          >
            <input
              type="checkbox"
              data-testid={`integrations-replacement-off-${target}`}
              checked={off.includes(target)}
              onChange={(e) => void set(target, e.target.checked)}
            />
            {`Always use oxplow's own ${label}`}
            <span style={mutedStyle}>· replaced by {by.join(", ")}</span>
          </label>
        );
      })}
    </fieldset>
  );
}

function IntegrationRow({
  view,
  onChanged,
  onCredentialChanged,
}: {
  view: ProviderInstanceView;
  onChanged(views: ProviderInstanceView[]): void;
  onCredentialChanged(): void;
}) {
  const saved = view.config as Record<string, unknown>;
  const [config, setConfig] = useState<Record<string, unknown> | null>(saved);
  const [checked, setChecked] = useState<ProviderInstanceView | null>(null);
  const [busy, setBusy] = useState<"check" | "toggle" | "sync" | null>(null);
  useEffect(() => setConfig(saved), [saved]);

  const shown = checked ?? view;
  const m = integrationRow(shown);
  const missing = view.health.state.state === "missing";

  async function check() {
    if (!config) return;
    setBusy("check");
    try {
      setChecked(await checkProviderInstance(view.instance, config));
    } catch (e) {
      recordOpError({ label: `Check ${view.instance}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  // Read a collector now, as the person (`provider.sync`), then show
  // where its reads stand.
  async function sync(collector: string) {
    setBusy("sync");
    try {
      const out = await runCommand("provider.sync", { instance: view.instance, collector });
      const records = (out.result as { reads?: Array<{ records: number }> } | null)?.reads?.[0]?.records ?? 0;
      showToast({ message: `Synced ${collector}: ${records} ${records === 1 ? "record" : "records"}.` });
    } catch (e) {
      recordOpError({ label: `Sync ${view.instance}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(null);
      onCredentialChanged();
    }
  }

  async function toggle() {
    if (!config) return;
    const enable = m.enableLabel !== "Disable";
    setBusy("toggle");
    try {
      onChanged(await setProviderInstance(view.instance, enable, config));
      setChecked(null);
      showToast({ message: `${enable ? "Enabled" : "Disabled"} ${view.instance}.` });
    } catch (e) {
      recordOpError({ label: `${enable ? "Enable" : "Disable"} ${view.instance}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div data-testid={`integration-row-${m.key}`} style={rowStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span>
          <code>{m.label}</code>
        </span>
        <span style={m.problem ? errorStyle : mutedStyle} data-testid={`integration-status-${m.key}`}>
          {m.status}
        </span>
        <span style={{ flex: 1 }} />
        <button
          type="button"
          data-testid={`integration-check-${m.key}`}
          disabled={busy !== null || missing || config === null}
          title="Start it with this config and ask it to check the config; nothing is saved or enabled"
          onClick={() => void check()}
        >
          {busy === "check" ? "Checking…" : "Check"}
        </button>
        <button
          type="button"
          data-testid={`integration-toggle-${m.key}`}
          disabled={busy !== null || missing || config === null}
          title={
            m.enableLabel === "Disable"
              ? "Stop it and turn it off in the project's config"
              : "Check it, save the config and run it on this machine"
          }
          onClick={() => void toggle()}
        >
          {busy === "toggle" ? "Saving…" : m.enableLabel}
        </button>
      </div>
      {view.collectors.map((c) => {
        const line = collectorLine(c);
        return (
          <div key={c.name} style={{ display: "flex", alignItems: "center", gap: 8 }} data-testid={`integration-collector-${m.key}-${c.name}`}>
            <span style={line.problem ? errorStyle : mutedStyle}>{line.text}</span>
            {view.health.state.state === "ready" ? (
              <button
                type="button"
                data-testid={`integration-sync-${m.key}-${c.name}`}
                disabled={busy !== null}
                title="Read this collector now, from where its last read left off"
                onClick={() => void sync(c.name)}
              >
                {busy === "sync" ? "Syncing…" : "Sync Now"}
              </button>
            ) : null}
          </div>
        );
      })}
      {m.needsApproval ? (
        <div style={mutedStyle}>Its program isn&apos;t approved on this machine yet: see Data → Programs.</div>
      ) : null}
      {view.credentials.map((c) => (
        <CredentialRow
          key={c.name}
          owner={view.instance}
          name={c.name}
          set={c.set}
          store={(v) => setInstanceCredential(view.instance, c.name, v)}
          onChanged={onCredentialChanged}
        />
      ))}
      {missing ? null : (
        <SchemaForm
          schema={view.configSchema as Record<string, unknown>}
          initial={saved}
          onChange={(next) => {
            setConfig(next);
            setChecked(null);
          }}
          testIdPrefix={`integration-config-${m.key}`}
        />
      )}
    </div>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const fieldsetStyle: CSSProperties = {
  border: "none",
  padding: "0 0 8px",
  margin: 0,
  display: "flex",
  flexDirection: "column",
  gap: 4,
  fontSize: "var(--text-sm)",
};
const rowStyle: CSSProperties = {
  padding: "6px 0",
  borderBottom: "1px solid var(--border-subtle)",
  fontSize: "var(--text-sm)",
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
