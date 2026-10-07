/// "Integrations" section body for SettingsPage (P5.D4): each extension
/// provider's instance on this machine (which one the project's work
/// items go to is chosen under Settings → Pieces) — its state, its config (a form
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
import { useCallback, useEffect, useRef, useState } from "react";

import {
  addProviderInstance,
  answerSignInRedirect,
  awaitSignInRedirect,
  beginOauthSignIn,
  canCatchSignInRedirect,
  checkProviderInstance,
  cancelOauthSignIn,
  completeOauthSignIn,
  effectiveConfig,
  listExtensions,
  listProviderInstances,
  listenForSignInRedirect,
  stopSignInRedirect,
  openInSystemBrowser,
  removeProviderInstance,
  turnOffProviderInstanceHere,
  runCommand,
  setInstanceCredential,
  setProviderInstance,
  subscribeOxplowEvents,
  type ProviderInstanceView,
} from "../api.js";
import type { SignInCompletion, SignInState, UiReplacement } from "../tauri-bridge/generated/bindings.js";
import { readsOf, useRerunOnChange } from "../lens/lensRerun.js";
import { CredentialRow } from "./ExtensionsSection.js";
import { InlineConfirm } from "./InlineConfirm.js";
import {
  canRemoveInstance,
  canTurnOffHere,
  collectorLine,
  integrationRow,
  newInstanceProblem,
  providerPrograms,
  signInLine,
} from "./integrationsModel.js";
import { SchemaForm } from "./SchemaForm/SchemaForm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function IntegrationsSection() {
  const [views, setViews] = useState<ProviderInstanceView[] | null>(null);
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
    } catch (e) {
      recordOpError({ label: "List integrations", message: String(e) });
      setViews([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return subscribeOxplowEvents((event) => {
      // A sign-in finishing changes only the keychain, and an approval only
      // this machine's approvals: there is no model to re-read, so the
      // instances are read again (tsk1040).
      if (event.kind === "configChanged" || event.kind === "credentialChanged" || event.kind === "approvalsChanged") {
        void refresh();
      }
    });
  }, [refresh]);
  // An instance starting, failing or recovering is recorded in its health
  // (`v_plugin_health`); Enable's own config change can be read before
  // the start finishes, so the rows re-read on this too (tsk1053).
  useRerunOnChange(readsOf("v_plugin_health"), () => void refresh());

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
      <Replacements replaced={replaced} off={off} onChanged={setOff} />
      {views.map((v) => (
        <IntegrationRow key={v.instance} view={v} onChanged={setViews} onCredentialChanged={() => void refresh()} />
      ))}
      <AddInstance views={views} onChanged={setViews} />
    </div>
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
        const label = replaced.find((r) => r.target === target)?.label ?? target;
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
      // Two rows, left-aligned: what it's an instance of, then its name,
      // scope and Add (tsk1045: one wrapped row centred and clipped it).
      style={{ ...fieldsetStyle, display: "flex", flexDirection: "column", alignItems: "stretch", gap: 8, textAlign: "left" }}
      onSubmit={(e) => {
        e.preventDefault();
        void add();
      }}
    >
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8 }}>
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
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8 }}>
      <input
        data-testid="integrations-add-name"
        style={{ flex: "1 1 220px", minWidth: 220 }}
        value={name}
        placeholder="its name, e.g. work_account"
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
      </div>
      {problem ? <div style={errorStyle}>{problem}</div> : null}
    </form>
  );
}

/** A credential the person signs in for (P9.B3): never typed. Sign in
 *  has the shell listen for the redirect (P10), opens the service's page in
 *  the person's own browser, and hands each redirect the shell catches to
 *  the core, answering the browser with its verdict; oxplow hears when it
 *  is done (`credentialChanged`) and the section reads the instances
 *  again. Only the desktop app can sign in: the redirect comes back to
 *  this machine. */
function SignInRow({
  instance,
  name,
  state,
  redirectPort,
  approved,
  onChanged,
}: {
  instance: string;
  name: string;
  state: SignInState;
  /** The port its service has registered for the redirect, if any. */
  redirectPort: number | null;
  /** Its program is approved as it is: where it signs in is part of that. */
  approved: boolean;
  onChanged(): void;
}) {
  const [waiting, setWaiting] = useState(false);
  // A sign-in is being started: a second click would start another.
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The shell's listener for this row's sign-in under way (its id, not
   *  its port — a newer sign-in may listen on the same port). */
  const listening = useRef<number | null>(null);
  /** The core's number for this row's sign-in under way: news of another
   *  sign-in of the credential isn't this row's, and leaving cancels it
   *  (tsk929). */
  const signingIn = useRef<number | null>(null);
  const shell = canCatchSignInRedirect();
  const id = `${instance}-${name}`;
  const line = signInLine(state);

  /** The sign-in under way is over: the shell stops listening — resolved
   *  once its socket is closed (tsk905) — and, unless it ended in the core
   *  already, the core forgets it now (tsk929). */
  async function stopListening() {
    const id = listening.current;
    const signIn = signingIn.current;
    listening.current = null;
    signingIn.current = null;
    if (id !== null) await stopSignInRedirect(id).catch(() => {});
    if (signIn !== null) await cancelOauthSignIn(instance, name, signIn).catch(() => {});
  }
  // Leaving the page ends it.
  useEffect(() => () => void stopListening(), []);

  useEffect(
    () =>
      subscribeOxplowEvents((event) => {
        if (event.kind !== "credentialChanged" || event.instance !== instance || event.name !== name) return;
        // News of another sign-in of this credential (this row's own,
        // replaced; another window's) isn't this row's.
        if (event.signIn !== null && event.signIn !== signingIn.current) return;
        setWaiting(false);
        setError(typeof event.error === "string" ? event.error : null);
      }),
    [instance, name],
  );

  async function signIn() {
    const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
    setError(null);
    setStarting(true);
    // A newer sign-in replaces the one under way (the core abandons it
    // too) — its socket closed before this one listens, on a declared
    // port the same one.
    await stopListening();
    let id: number;
    try {
      const listener = await listenForSignInRedirect(redirectPort);
      id = listener.id;
      listening.current = id;
      const begun = await beginOauthSignIn(instance, name, listener.port);
      signingIn.current = begun.signIn;
      await openInSystemBrowser(begun.url);
      setWaiting(true);
    } catch (e) {
      await stopListening();
      setError(message(e));
      return;
    } finally {
      setStarting(false);
    }
    // Each redirect the shell catches goes to the core, and the browser
    // hears its verdict; one that isn't this sign-in's is refused and the
    // wait goes on. How it ended arrives as `credentialChanged`.
    try {
      for (;;) {
        const redirect = await awaitSignInRedirect(id);
        const outcome: SignInCompletion = await completeOauthSignIn(instance, name, redirect).catch(
          (e: unknown) => ({ outcome: "failed", error: message(e) }),
        );
        await answerSignInRedirect(id, outcome);
        if (outcome.outcome === "not_this_sign_in") continue;
        if (listening.current === id) {
          listening.current = null;
          // Over in the core too: nothing to cancel.
          signingIn.current = null;
        }
        // The row says how it went from what it holds (tsk906): a failure
        // the core didn't announce (no sign-in under way, a lost
        // connection to a remote daemon) must not leave it waiting.
        setWaiting(false);
        if (outcome.outcome === "failed") setError(outcome.error);
        return;
      }
    } catch (e) {
      // Stopped by a newer sign-in or by leaving (no longer this row's
      // concern), or never finished.
      if (listening.current !== id) return;
      listening.current = null;
      signingIn.current = null;
      setWaiting(false);
      setError(message(e));
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
        disabled={!approved || starting || !shell}
        title={
          !shell
            ? "Signing in needs the oxplow desktop app: the service sends your browser back to this machine, where only the app can listen"
            : approved
              ? "Open the service's sign-in page in your browser; the token it gives is kept in this machine's keychain"
              : "Approve its program on Settings → Data → Programs first: where it signs in is part of what you approve"
        }
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
  const [busy, setBusy] = useState<"check" | "toggle" | "sync" | "remove" | "off-here" | null>(null);
  // The draft follows the saved config when it changes, not each re-read
  // of the instances, which brings a fresh copy of the same one and would
  // wipe what the person typed but hasn't saved (tsk1054).
  const savedText = JSON.stringify(saved);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content
  useEffect(() => setConfig(saved), [savedText]);

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

  // Turn the person's global instance off in this project alone: the
  // project gets its own entry, off; Remove on it brings theirs back.
  async function offHere() {
    setBusy("off-here");
    try {
      onChanged(await turnOffProviderInstanceHere(view.instance));
      showToast({ message: `Turned ${view.instance} off in this project.` });
    } catch (e) {
      recordOpError({ label: `Turn off ${view.instance} here`, message: e instanceof Error ? e.message : String(e) });
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
        {canTurnOffHere(view) ? (
          <button
            type="button"
            data-testid={`integration-off-here-${m.key}`}
            disabled={busy !== null}
            title="Turn it off in this project only: this project gets its own entry, off, and every other project keeps running yours. Remove that entry to bring yours back here."
            onClick={() => void offHere()}
          >
            {busy === "off-here" ? "Saving…" : "Off in this project"}
          </button>
        ) : null}
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
        <div style={mutedStyle} data-testid={`integration-needs-approval-${m.key}`}>
          Its program isn&apos;t approved on this machine yet: see Data → Programs.
        </div>
      ) : null}
      {view.credentials.map((c) =>
        c.signIn ? (
          <SignInRow
            key={c.name}
            instance={view.instance}
            name={c.name}
            state={c.signIn}
            redirectPort={c.redirectPort}
            approved={view.approved}
            onChanged={onCredentialChanged}
          />
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
