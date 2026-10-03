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
  addProviderInstance,
  beginOauthSignIn,
  checkProviderInstance,
  effectiveConfig,
  listExtensions,
  listProviderInstances,
  openInSystemBrowser,
  removeProviderInstance,
  runCommand,
  setInstanceCredential,
  setProviderInstance,
  subscribeOxplowEvents,
  type ProviderInstanceView,
} from "../api.js";
import { REPLACEABLE_LABELS } from "../lens/useReplacement.js";
import type { SignInState, UiReplacement } from "../tauri-bridge/generated/bindings.js";
import { CredentialRow } from "./ExtensionsSection.js";
import { InlineConfirm } from "./InlineConfirm.js";
import {
  activeProviderProblem,
  canRemoveInstance,
  collectorLine,
  integrationRow,
  newInstanceProblem,
  providerPrograms,
  signInLine,
  workItemsChoices,
} from "./integrationsModel.js";
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
      // A sign-in finishing changes only the keychain: there is no model
      // to re-read, so the instances are read again.
      if (event.kind === "configChanged" || event.kind === "credentialChanged") void refresh();
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
      <AddInstance views={views} onChanged={setViews} />
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

/** Add another instance of a provider (P9.B6): a second team or
 *  workspace, under its own name — the project's (shared with the team in
 *  its config), or the person's own on this machine, in every project
 *  that has the extension. It starts off: configure it, then Enable. */
function AddInstance({ views, onChanged }: { views: ProviderInstanceView[]; onChanged(views: ProviderInstanceView[]): void }) {
  const programs = providerPrograms(views);
  const [program, setProgram] = useState("");
  const [name, setName] = useState("");
  const [scope, setScope] = useState<"project" | "global">("project");
  const [adding, setAdding] = useState(false);
  if (programs.length === 0) return null;
  const of = programs.find((p) => p.key === program) ?? programs[0];
  const id = name.trim();
  const problem = newInstanceProblem(views, id);

  async function add() {
    if (problem !== null || adding) return;
    setAdding(true);
    try {
      onChanged(await addProviderInstance(`${of.extension}/${id}`, of.provider, scope));
      setName("");
      showToast({ message: `Added ${of.extension}/${id}. Configure it, then Enable.` });
    } catch (e) {
      recordOpError({ label: `Add ${of.extension}/${id}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setAdding(false);
    }
  }

  return (
    <form
      data-testid="integrations-add-instance"
      style={{ ...fieldsetStyle, display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8 }}
      onSubmit={(e) => {
        e.preventDefault();
        void add();
      }}
    >
      <span style={mutedStyle}>Add another instance of</span>
      {programs.length === 1 ? (
        <code>{of.key}</code>
      ) : (
        <select data-testid="integrations-add-provider" value={of.key} onChange={(e) => setProgram(e.target.value)}>
          {programs.map((p) => (
            <option key={p.key} value={p.key}>
              {p.key}
            </option>
          ))}
        </select>
      )}
      <input
        data-testid="integrations-add-name"
        value={name}
        placeholder="its name, e.g. linear_acme"
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setName("");
        }}
      />
      {(["project", "global"] as const).map((s) => (
        <label key={s} style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <input
            type="radio"
            name="integrations-add-scope"
            data-testid={`integrations-add-scope-${s}`}
            checked={scope === s}
            onChange={() => setScope(s)}
          />
          {s === "project" ? "This project's" : "Mine, in every project"}
        </label>
      ))}
      <button type="submit" data-testid="integrations-add-submit" disabled={problem !== null || adding}>
        {adding ? "Adding…" : "Add"}
      </button>
      {problem ? <div style={{ ...errorStyle, flexBasis: "100%" }}>{problem}</div> : null}
    </form>
  );
}

/** A credential the person signs in for (P9.B3): never typed. Sign in
 *  opens the service's page in their own browser; oxplow hears when it is
 *  done (`credentialChanged`) and the section reads the instances again. */
function SignInRow({
  instance,
  name,
  state,
  onChanged,
}: {
  instance: string;
  name: string;
  state: SignInState;
  onChanged(): void;
}) {
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = `${instance}-${name}`;
  const line = signInLine(state);

  useEffect(
    () =>
      subscribeOxplowEvents((event) => {
        if (event.kind !== "credentialChanged" || event.instance !== instance || event.name !== name) return;
        setWaiting(false);
        setError(typeof event.error === "string" ? event.error : null);
      }),
    [instance, name],
  );

  async function signIn() {
    setError(null);
    try {
      await openInSystemBrowser(await beginOauthSignIn(instance, name));
      setWaiting(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function signOut() {
    try {
      await setInstanceCredential(instance, name, null);
      showToast({ message: `Signed out of ${name}.` });
      onChanged();
    } catch (e) {
      recordOpError({ label: `Sign out of ${name}`, message: String(e) });
    }
  }

  return (
    <div data-testid={`sign-in-${id}`} style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4, flexWrap: "wrap" }}>
      <code>{name}</code>
      <span style={line.problem ? errorStyle : mutedStyle}>{line.text}</span>
      {waiting ? <span style={mutedStyle}>Finish signing in in your browser…</span> : null}
      {error ? <span style={errorStyle}>{error}</span> : null}
      <span style={{ flex: 1 }} />
      <button
        type="button"
        data-testid={`sign-in-button-${id}`}
        title="Open the service's sign-in page in your browser; the token it gives is kept in this machine's keychain"
        onClick={() => void signIn()}
      >
        {line.action}
      </button>
      {line.signedIn ? (
        <InlineConfirm triggerLabel="Sign out" confirmLabel="Sign out" testIdPrefix={`sign-out-${id}`} onConfirm={() => void signOut()} />
      ) : null}
    </div>
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
  const [busy, setBusy] = useState<"check" | "toggle" | "sync" | "remove" | null>(null);
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

  // Remove the instance: it stops, and its config entry and its
  // credentials on this machine go.
  async function remove() {
    setBusy("remove");
    try {
      onChanged(await removeProviderInstance(view.instance));
      showToast({ message: `Removed ${view.instance}.` });
    } catch (e) {
      recordOpError({ label: `Remove ${view.instance}`, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(null);
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
        {canRemoveInstance(view) ? (
          <InlineConfirm
            triggerLabel="Remove"
            confirmLabel="Remove"
            testIdPrefix={`integration-remove-${m.key}`}
            disabled={busy !== null}
            onConfirm={() => void remove()}
          />
        ) : null}
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
      {view.credentials.map((c) =>
        c.signIn ? (
          <SignInRow key={c.name} instance={view.instance} name={c.name} state={c.signIn} onChanged={onCredentialChanged} />
        ) : (
          <CredentialRow
            key={c.name}
            owner={view.instance}
            name={c.name}
            set={c.set}
            store={(v) => setInstanceCredential(view.instance, c.name, v)}
            onChanged={onCredentialChanged}
          />
        ),
      )}
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
