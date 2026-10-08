/// "Extensions" section body for SettingsPage: every project extension
/// (`oxplow/extensions/`) with its origin, lens count and load errors,
/// its sources' credentials (set here, kept in the keychain); an Update
/// action for git-installed ones; and an install strip for a git URL.
/// Install and Update first show what the extension would bring in (its
/// programs, hosts, credentials, advisories) and install only the commit
/// the person confirmed (tsk378).
/// Each provider instance's and collector's health shows on its
/// extension's row: a disabled one with its reason, **Enable Again**
/// (`oxplow.contribution.enable` as the person) and **Repair with the Agent** (fills
/// the agent's input with a mention of the repair item; never sends).
/// Running sources is under Data (DataSection.tsx). See
/// `.context/extensions.md` → "Health, disable and repair".
///
/// Usability contract (.context/usability.md): no modals; Enter submits
/// the install strip; failures land in opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  installExtension,
  listExtensions,
  listCollectors,
  reviewExtension,
  setExtensionEnabled,
  setCredential,
  subscribeOxplowEvents,
  updateExtension,
  validateExtension,
  type Extension,
  type ExtensionReview,
  type CollectorListing,
} from "../api.js";
import { NEW_LENS_COMMAND } from "../lens/lensModel.js";
import { useCommandDraft } from "../commandDraft.js";
import { enableAgain, healthLine, healthOf, repairWithAgent, useContributionHealth, type ContributionHealth } from "../contributionHealth.js";
import { collectorRan, extensionCredentials, extensionRowModel, reviewModel } from "./extensionRowModel.js";
import { extensionsChanged } from "../lens/lensRerun.js";
import { ImpactReportView } from "./ImpactReportView.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";
import { EmptyState } from "./Prompts/EmptyState.js";

export function ExtensionsSection() {
  const newLensPrompt = useCommandDraft(NEW_LENS_COMMAND);
  const [exts, setExts] = useState<Extension[] | null>(null);
  /** What each project extension's check found (bundled ones are checked
   *  where they're built). */
  const [checked, setChecked] = useState<Record<string, string[]>>({});
  const [collectors, setCollectors] = useState<CollectorListing[]>([]);
  const health = useContributionHealth();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  /** An install or update waiting on the person's go-ahead. */
  const [pending, setPending] = useState<
    { kind: "install"; url: string; review: ExtensionReview } | { kind: "update"; name: string; review: ExtensionReview } | null
  >(null);

  const refresh = useCallback(async () => {
    try {
      const listed = await listExtensions();
      setExts(listed);
      setCollectors(await listCollectors());
      const reports = await Promise.all(
        listed
          .filter((e) => e.origin !== "bundled" && e.enabled)
          .map(async (e) => {
            try {
              return [e.name, (await validateExtension(e.name, null)).errors] as const;
            } catch (err) {
              return [e.name, [`couldn't check it: ${String(err)}`]] as const;
            }
          }),
      );
      setChecked(Object.fromEntries(reports));
    } catch (e) {
      recordOpError({ label: "List extensions", message: String(e) });
      setExts([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Scheduled and agent-triggered runs land here too.
    return subscribeOxplowEvents((event) => {
      if (collectorRan(event) || extensionsChanged(event as Record<string, unknown>)) void refresh();
    });
  }, [refresh]);

  async function review() {
    const gitUrl = url.trim();
    if (!gitUrl || busy) return;
    setBusy("install");
    try {
      setPending({ kind: "install", url: gitUrl, review: await reviewExtension({ gitUrl }, null) });
    } catch (e) {
      recordOpError({ label: `Review extension ${gitUrl}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  async function confirm() {
    if (!pending || busy) return;
    const { review } = pending;
    setBusy(pending.kind === "install" ? "install" : pending.name);
    try {
      if (pending.kind === "install") {
        const ext = await installExtension(pending.url, null, review.sha, null);
        setUrl("");
        showToast({ message: `Installed ${ext.name}. Commit oxplow/extensions/${ext.name} to share it with your team.` });
      } else {
        await updateExtension(pending.name, review.sha, null);
        showToast({ message: `Updated ${pending.name}.` });
      }
      setPending(null);
      await refresh();
    } catch (e) {
      recordOpError({ label: `${pending.kind === "install" ? "Install" : "Update"} extension ${review.extension.name}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  async function toggle(name: string, enabled: boolean) {
    setBusy(name);
    try {
      setExts(await setExtensionEnabled(name, enabled));
      setCollectors(await listCollectors());
      showToast({ message: enabled ? `Enabled ${name}.` : `Disabled ${name} for this project.` });
    } catch (e) {
      recordOpError({ label: `${enabled ? "Enable" : "Disable"} extension ${name}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  async function update(name: string) {
    setBusy(name);
    try {
      setPending({ kind: "update", name, review: await reviewExtension({ name }, null) });
    } catch (e) {
      recordOpError({ label: `Review update of ${name}`, message: String(e) });
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
          text="An extension adds lenses, data and checks. Have the agent build one, or install one below."
          prompts={newLensPrompt ? [newLensPrompt] : []}
        />
      ) : (
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
                    onClick={() => void toggle(m.name, !m.enabled)}
                  >
                    {m.toggleLabel}
                  </button>
                  {m.canUpdate ? (
                    <button
                      type="button"
                      data-testid={`extension-update-${m.name}`}
                      disabled={busy !== null}
                      onClick={() => void update(m.name)}
                    >
                      {busy === m.name ? "Checking…" : "Update…"}
                    </button>
                  ) : null}
                </div>
                {m.description ? <div style={mutedStyle}>{m.description}</div> : null}
                {m.disabledNote ? <div style={mutedStyle}>{m.disabledNote}</div> : null}
                {m.errors.map((err, i) => (
                  <div key={i} style={errorStyle}>
                    {err}
                  </div>
                ))}
                {healthOf(health, m.name).map((h) => (
                  <HealthRow key={`${h.kind}:${h.contribution}`} health={h} />
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
      )}
      <div style={{ display: "flex", gap: 8 }}>
        <input
          data-testid="extension-install-url"
          value={url}
          placeholder="Git URL of an extension, e.g. https://github.com/acme/oxplow-lenses"
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void review();
            if (e.key === "Escape") setUrl("");
          }}
          style={{ flex: 1 }}
        />
        <button
          type="button"
          data-testid="extension-install"
          disabled={!url.trim() || busy !== null || pending !== null}
          onClick={() => void review()}
        >
          {busy === "install" && !pending ? "Checking…" : "Install…"}
        </button>
      </div>
      {pending ? (
        <ReviewPanel
          review={pending.review}
          action={pending.kind === "install" ? "Install" : "Update"}
          busy={busy !== null}
          onConfirm={() => void confirm()}
          onCancel={() => setPending(null)}
        />
      ) : null}
    </div>
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

/// What an install or update would bring in, with the go-ahead. Inline,
/// not a modal: the confirm button takes focus, Escape cancels.
export function ReviewPanel({
  review,
  action,
  busy,
  onConfirm,
  onCancel,
}: {
  review: ExtensionReview;
  action: "Install" | "Update";
  busy: boolean;
  onConfirm(): void;
  onCancel(): void;
}) {
  const m = reviewModel(review);
  return (
    <div
      data-testid="extension-review"
      style={{ ...rowStyle, marginTop: 8 }}
      onKeyDown={(e) => {
        if (e.key === "Escape") onCancel();
      }}
    >
      <div>
        <strong>{m.name}</strong> <span style={mutedStyle}>from {m.from}</span>
      </div>
      {m.description ? <div style={mutedStyle}>{m.description}</div> : null}
      <ul data-testid="extension-review-declares" style={{ margin: "6px 0", paddingLeft: 18 }}>
        {m.declares.length === 0 ? <li style={mutedStyle}>Nothing but its manifest.</li> : null}
        {m.declares.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ul>
      {/* No report when the candidate doesn't load; its errors say why. */}
      {review.impact ? <ImpactReportView report={review.impact} testId="extension-review-impact" /> : null}
      {m.errors.map((err, i) => (
        <div key={`e${i}`} style={errorStyle}>
          {err}
        </div>
      ))}
      {m.problems.map((p, i) => (
        <div key={`p${i}`} style={mutedStyle}>
          {p}
        </div>
      ))}
      <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
        <button
          type="button"
          data-testid="extension-review-confirm"
          autoFocus
          disabled={!m.canInstall || busy}
          onClick={onConfirm}
        >
          {busy ? `${action === "Install" ? "Installing" : "Updating"}…` : action}
        </button>
        <button type="button" data-testid="extension-review-cancel" onClick={onCancel}>
          Cancel
        </button>
        {!m.canInstall ? <span style={errorStyle}>Fix its errors before installing.</span> : null}
      </div>
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

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
const errorInlineStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const warnStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-medium)" };
