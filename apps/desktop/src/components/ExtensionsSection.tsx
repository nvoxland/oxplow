/// "Extensions" section body for SettingsPage: every extension the project
/// knows with where its files come from (shipped, deployed, not deployed,
/// not here), its lens count and load errors, and its sources' credentials
/// (set here, kept in the keychain). Enable/Disable turn it on or off for
/// the person (`oxplow.extension.enable` / `disable`); Deploy runs one whose
/// source was never deployed (`oxplow.extension.deploy`).
/// Each provider instance's and collector's health shows on its
/// extension's row: a disabled one with its reason, **Enable Again**
/// (`oxplow.contribution.enable` as the person) and **Repair with the Agent** (fills
/// the agent's input with a mention of the repair item; never sends).
/// Running sources is under Data (DataSection.tsx). See
/// `.context/extensions.md` → "Health, disable and repair".
///
/// Usability contract (.context/usability.md): no modals; failures land in
/// opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  listExtensions,
  listServices,
  listCollectors,
  runCommand,
  setCredential,
  saveExtensionSecret,
  extensionSecretIsSet,
  revokeExtensionSecret,
  subscribeOxplowEvents,
  validateExtension,
  type Extension,
  type ScriptServices,
  type ServiceListing,
  type CollectorListing,
  querySql,
} from "../api.js";
import { NEW_LENS_COMMAND } from "../lens/lensModel.js";
import { useCommandDraft } from "../commandDraft.js";
import { disabledCount, enableAgain, enableAllAgain, healthLine, healthOf, repairWithAgent, useContributionHealth, type ContributionHealth } from "../contributionHealth.js";
import { COLLECTOR_RUNS, extensionCredentials, extensionRowModel, LATEST_RUNS_SQL, lastRuns, type LastRun, type RunRow, secretCredentials, secretRowModel, serviceLines, type SecretCredential } from "./extensionRowModel.js";
import { extensionsChanged } from "../lens/lensRerun.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { InlineEdit } from "./InlineEdit.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";
import { EmptyState } from "./Prompts/EmptyState.js";

export function ExtensionsSection() {
  const newLensPrompt = useCommandDraft(NEW_LENS_COMMAND);
  const [exts, setExts] = useState<Extension[] | null>(null);
  /** What each project extension's check found (bundled ones are checked
   *  where they're built). */
  const [checked, setChecked] = useState<Record<string, string[]>>({});
  /** The services each project extension's scripts are given (its check
   *  reports them), and the titles that name them. */
  const [services, setServices] = useState<Record<string, ScriptServices[]>>({});
  const [serviceListing, setServiceListing] = useState<ServiceListing[]>([]);
  const [collectors, setCollectors] = useState<CollectorListing[]>([]);
  /** Each extension's last run (`v_extension_run`, latest per contribution). */
  const [runs, setRuns] = useState<Record<string, LastRun>>({});
  const [runReads, setRunReads] = useState<Reads>(NO_READS);
  const health = useContributionHealth();
  const [busy, setBusy] = useState<string | null>(null);

  // Each part re-reads on the change that touches it, not on any event
  // (tsk1403): the extensions and their checks when the catalog or config
  // changes, the collectors when a run commits (`v_collector_run`), the
  // last runs when a run row does (`v_extension_run`).
  const loadExtensions = useCallback(async () => {
    try {
      const listed = await listExtensions();
      setExts(listed);
      const reports = await Promise.all(
        listed
          .filter((e) => e.origin !== "bundled" && e.enabled && e.deployment.kind === "deployed")
          .map(async (e) => {
            try {
              const report = await validateExtension(e.name, null);
              return [e.name, report.errors, report.services] as [string, string[], ScriptServices[]];
            } catch (err) {
              return [e.name, [`couldn't check it: ${String(err)}`], []] as [string, string[], ScriptServices[]];
            }
          }),
      );
      setChecked(Object.fromEntries(reports.map(([name, errors]) => [name, errors])));
      setServices(Object.fromEntries(reports.map(([name, , scripts]) => [name, scripts])));
    } catch (e) {
      recordOpError({ label: "List extensions", message: String(e) });
      setExts([]);
    }
  }, []);
  const loadCollectors = useCallback(async () => {
    try {
      setCollectors(await listCollectors());
    } catch (e) {
      recordOpError({ label: "List collectors", message: String(e) });
    }
  }, []);
  const loadRuns = useCallback(async () => {
    // A read that fails costs the line, not the list.
    try {
      const r = await querySql(LATEST_RUNS_SQL, [], null);
      setRunReads(r.reads);
      setRuns(
        lastRuns(
          r.rows.map(
            ([extension, contribution, outcome, error, finished_at, decisions]) =>
              ({ extension, contribution, outcome, error, finished_at, decisions }) as RunRow,
          ),
        ),
      );
    } catch {
      setRuns({});
    }
  }, []);
  /** Everything again: what a deploy or an enable leaves different. */
  const refresh = useCallback(async () => {
    await Promise.all([loadExtensions(), loadCollectors(), loadRuns()]);
  }, [loadExtensions, loadCollectors, loadRuns]);

  useRerunOnChange(runReads, () => void loadRuns());
  useRerunOnChange(COLLECTOR_RUNS, () => void loadCollectors());
  useEffect(() => {
    // A read that fails costs the lines' wording, not the list.
    void listServices().then(setServiceListing).catch(() => setServiceListing([]));
  }, []);
  useEffect(() => {
    void refresh();
    return subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) {
        void loadExtensions();
        void loadCollectors();
      }
    });
  }, [refresh, loadExtensions, loadCollectors]);

  /** Run `command` for extension `name`, then read everything again. */
  async function act(name: string, label: string, command: string, done: string) {
    setBusy(name);
    try {
      await runCommand(command, { name });
      await refresh();
      showToast({ message: done });
    } catch (e) {
      recordOpError({ label: `${label} extension ${name}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div data-testid="extensions-section">
      {exts === null ? (
        <div style={mutedStyle}>Loading…</div>
      ) : exts.length === 0 ? (
        <EmptyState
          testId="extensions-empty"
          title="No extensions yet"
          text="An extension adds lenses, data and checks. Have the agent build one."
          prompts={newLensPrompt ? [newLensPrompt] : []}
        />
      ) : (
        <>
        <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
          <EnableAllButton count={disabledCount(health)} />
        </div>
        <ul style={{ listStyle: "none", margin: "0 0 12px", padding: 0 }}>
          {exts.map((ext) => {
            const m = extensionRowModel(ext, checked[ext.name]);
            return (
              <li key={m.name} data-testid={`extension-row-${m.name}`} style={rowStyle}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <strong>{m.name}</strong>
                  <span style={mutedStyle}>
                    {m.lensCount} {m.lensCount === 1 ? "lens" : "lenses"} · {m.origin}
                  </span>
                  <span style={{ flex: 1 }} />
                  <button
                    type="button"
                    data-testid={`extension-toggle-${m.name}`}
                    disabled={busy !== null}
                    onClick={() =>
                      void (m.enabled
                        ? act(m.name, "Disable", "oxplow.extension.disable", `Disabled ${m.name} for you.`)
                        : act(m.name, "Enable", "oxplow.extension.enable", `Enabled ${m.name}.`))
                    }
                  >
                    {m.toggleLabel}
                  </button>
                  {m.canDeploy ? (
                    <button
                      type="button"
                      data-testid={`extension-deploy-${m.name}`}
                      disabled={busy !== null}
                      onClick={() => void act(m.name, "Deploy", "oxplow.extension.deploy", `Deployed ${m.name}.`)}
                    >
                      {busy === m.name ? "Deploying…" : "Deploy"}
                    </button>
                  ) : null}
                </div>
                {m.description ? <div style={mutedStyle}>{m.description}</div> : null}
                {m.disabledNote ? <div style={mutedStyle}>{m.disabledNote}</div> : null}
                {runs[m.name] ? (
                  <div
                    data-testid={`extension-last-run-${m.name}`}
                    data-warn={runs[m.name]!.warn ? "true" : "false"}
                    style={runs[m.name]!.warn ? { ...mutedStyle, color: "var(--severity-medium)" } : mutedStyle}
                  >
                    {runs[m.name]!.text}
                    {runs[m.name]!.reason ? ` — ${runs[m.name]!.reason}` : ""}
                  </div>
                ) : null}
                {serviceLines(services[m.name] ?? [], serviceListing).map((line) => (
                  <div
                    key={line.script}
                    data-testid={`extension-services-${m.name}`}
                    title="The services this script is given: what it can reach beyond its own input"
                    style={mutedStyle}
                  >
                    {line.script} — {line.text}
                  </div>
                ))}
                {m.errors.map((err, i) => (
                  <div key={i} style={errorStyle}>
                    {err}
                  </div>
                ))}
                {disabledCount(health, m.name) > 1 ? (
                  <div style={{ marginTop: 4 }}>
                    <EnableAllButton extension={m.name} count={disabledCount(health, m.name)} />
                  </div>
                ) : null}
                {healthOf(health, m.name).map((h) => (
                  <HealthRow key={`${h.kind}:${h.contribution}`} health={h} />
                ))}
                {secretCredentials(ext).map((c) => (
                  <SecretRow
                    key={`${c.provider}/${c.name}`}
                    extension={m.name}
                    credential={c}
                  />
                ))}
                {extensionCredentials(collectors, m.name).map((c) => (
                  <CredentialRow
                    key={c.name}
                    owner={m.name}
                    name={c.name}
                    set={c.set}
                    store={(v) => setCredential(m.name, c.name, v)}
                    onChanged={() => void refresh()}
                  />
                ))}
              </li>
            );
          })}
        </ul>
        </>
      )}
    </div>
  );
}

/// Turn every failure-disabled contribution back on in one go: one
/// extension's (`extension`) or all. Shown only when `count` is more than
/// the single Enable Again would cover.
export function EnableAllButton({ extension, count }: { extension?: string; count: number }) {
  const [enabling, setEnabling] = useState(false);
  if (count < 1) return null;
  const key = extension ?? "all";
  return (
    <button
      type="button"
      data-testid={`extension-enable-all-${key}`}
      title={extension ? `Turn ${extension}'s disabled contributions back on, on this machine` : "Turn every disabled contribution back on, on this machine"}
      disabled={enabling}
      onClick={() => {
        setEnabling(true);
        void enableAllAgain(extension).finally(() => setEnabling(false));
      }}
    >
      {enabling ? "Enabling…" : `Enable All Disabled (${count})`}
    </button>
  );
}

/// One contribution's health line; a disabled one offers Enable Again and,
/// when its repair item is open, Repair with the Agent.
export function HealthRow({ health }: { health: ContributionHealth }) {
  const line = healthLine(health);
  const key = `${health.extension}-${health.contribution}`;
  const [enabling, setEnabling] = useState(false);
  return (
    <div data-testid={`extension-health-${key}`} style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}>
      <span style={mutedStyle}>
        {health.kind} <code>{health.contribution}</code>
      </span>
      <span style={line.tone === "error" ? errorInlineStyle : line.tone === "warn" ? warnStyle : mutedStyle}>{line.text}</span>
      <span style={{ flex: 1 }} />
      {line.repairItem ? (
        <button
          type="button"
          data-testid={`extension-repair-${key}`}
          title="Put a request to repair it in the agent's input; nothing is sent until you press Enter"
          onClick={() => repairWithAgent(line.repairItem as string)}
        >
          Repair with the Agent
        </button>
      ) : null}
      {line.canEnable ? (
        <button
          type="button"
          data-testid={`extension-enable-${key}`}
          title="Turn it back on on this machine"
          disabled={enabling}
          onClick={() => {
            setEnabling(true);
            void enableAgain(health).finally(() => setEnabling(false));
          }}
        >
          {enabling ? "Enabling…" : "Enable Again"}
        </button>
      ) : null}
    </div>
  );
}

/// One declared credential — an extension's (its collectors share them)
/// or a provider instance's own: set or replace its value (it goes to the
/// OS keychain and never comes back), or clear it. `owner` is whose it is
/// (the extension, or `<extension>/<instance id>`) and `store` writes it
/// there.
export function CredentialRow({
  owner,
  name,
  set,
  store,
  onChanged,
}: {
  owner: string;
  name: string;
  set: boolean;
  /** Save `value` (null: forget it) where this credential lives. */
  store(value: string | null): Promise<unknown>;
  onChanged(): void;
}) {
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);
  const id = `${owner}-${name}`;

  async function save(v: string | null) {
    setSaving(true);
    try {
      await store(v);
      setValue("");
      showToast({ message: v ? `Saved ${name} to your keychain.` : `Cleared ${name}.` });
      onChanged();
    } catch (e) {
      recordOpError({ label: `Set credential ${name}`, message: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      data-testid={`credential-${id}`}
      style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) void save(value.trim());
      }}
    >
      <code>{name}</code>
      <span style={mutedStyle}>{set ? "Saved in keychain" : "Not set"}</span>
      <input
        data-testid={`credential-input-${id}`}
        type="password"
        autoComplete="off"
        value={value}
        placeholder={set ? "New value to replace it" : "Value"}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setValue("");
        }}
        style={{ flex: 1 }}
      />
      <button type="submit" data-testid={`credential-save-${id}`} disabled={!value.trim() || saving}>
        {saving ? "Saving…" : "Save"}
      </button>
      {set ? (
        <InlineConfirm
          triggerLabel="Clear"
          confirmLabel="Clear"
          testIdPrefix={`credential-clear-${id}`}
          disabled={saving}
          onConfirm={() => void save(null)}
        />
      ) : null}
    </form>
  );
}

/// One credential an extension's provider declares with a host secret
/// reference: presence, Set/Rotate as a masked inline field (Enter
/// commits, Escape cancels) and Revoke behind a confirm. The value goes to
/// the keychain and never comes back to the page.
export function SecretRow({
  extension,
  credential,
}: {
  extension: string;
  credential: SecretCredential;
}) {
  const [isSet, setIsSet] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const id = `${extension}-${credential.provider}-${credential.name}`;

  const check = useCallback(async () => {
    try {
      setIsSet(await extensionSecretIsSet(extension, credential.reference));
    } catch (e) {
      recordOpError({ label: `Read secret ${credential.name}`, message: String(e) });
    }
  }, [extension, credential.reference, credential.name]);

  useEffect(() => {
    void check();
    return subscribeOxplowEvents((event) => {
      if (extensionsChanged(event)) void check();
    });
  }, [check]);

  async function run(label: string, op: () => Promise<void>) {
    setBusy(true);
    try {
      await op();
      showToast({ message: label });
      await check();
    } catch (e) {
      recordOpError({ label: `Secret ${credential.name}`, message: String(e) });
    } finally {
      setBusy(false);
    }
  }

  const m = secretRowModel(isSet);
  return (
    <div
      data-testid={`secret-${id}`}
      style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}
    >
      <code>{credential.name}</code>
      <span style={mutedStyle}>{credential.provider}</span>
      <span data-testid={`secret-status-${id}`} style={mutedStyle}>
        {m.status}
      </span>
      <InlineEdit
        masked
        value=""
        placeholder={`${m.setLabel}…`}
        testId={`secret-set-${id}`}
        ariaLabel={`${m.setLabel} ${credential.name}`}
        onCommit={(v) =>
          void run(`Saved ${credential.name} to your keychain.`, () =>
            saveExtensionSecret(extension, credential.reference, v),
          )
        }
      />
      {m.canRevoke ? (
        <InlineConfirm
          triggerLabel="Revoke"
          confirmLabel="Revoke"
          testIdPrefix={`secret-revoke-${id}`}
          disabled={busy}
          onConfirm={() =>
            void run(`Revoked ${credential.name}.`, () =>
              revokeExtensionSecret(extension, credential.reference),
            )
          }
        />
      ) : null}
    </div>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
const errorInlineStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const warnStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-medium)" };
