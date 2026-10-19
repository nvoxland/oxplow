/// "Health and credentials" section body for SettingsPage: what has no lens
/// equivalent. The extension list itself (deployment, permissions, Deploy,
/// Roll Back, Enable, Disable, Share, Remove) is the foundation extension's
/// Extensions lens, mounted above this section.
/// Per extension that has any: its disabled contributions with the reason,
/// **Enable Again** (`oxplow.contribution.enable` as the person) and
/// **Repair with the Agent** (fills the agent's input with a mention of the
/// repair item; never sends), the collectors' credentials and the
/// providers' secrets (set here, kept in the keychain).
/// Running sources is under Data (DataSection.tsx). See
/// `.context/extensions.md` -> "Health, disable and repair".
///
/// Usability contract (.context/usability.md): no modals; failures land in
/// opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  listExtensions,
  listCollectors,
  setCredential,
  saveExtensionSecret,
  extensionSecretIsSet,
  revokeExtensionSecret,
  subscribeOxplowEvents,
  type Extension,
  type CollectorListing,
} from "../api.js";
import { disabledCount, enableAgain, enableAllAgain, healthLine, healthOf, repairWithAgent, useContributionHealth, type ContributionHealth } from "../contributionHealth.js";
import { COLLECTOR_RUNS, extensionCredentials, secretCredentials, secretRowModel, type SecretCredential } from "./extensionRowModel.js";
import { extensionsChanged, useRerunOnChange } from "../lens/lensRerun.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { InlineEdit } from "./InlineEdit.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function ExtensionsSection() {
  const [exts, setExts] = useState<Extension[] | null>(null);
  const [collectors, setCollectors] = useState<CollectorListing[] | null>(null);
  const health = useContributionHealth();

  const loadExtensions = useCallback(async () => {
    try {
      setExts(await listExtensions());
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
      setCollectors([]);
    }
  }, []);
  const refresh = useCallback(async () => {
    await Promise.all([loadExtensions(), loadCollectors()]);
  }, [loadExtensions, loadCollectors]);

  // Each part re-reads on the change that touches it: the extensions when
  // the catalog or config changes, the collectors when a run commits.
  useRerunOnChange(COLLECTOR_RUNS, () => void loadCollectors());
  useEffect(() => {
    void refresh();
    return subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) void refresh();
    });
  }, [refresh]);

  if (exts === null || collectors === null) {
    return (
      <div data-testid="extensions-section" style={mutedStyle}>
        Loading…
      </div>
    );
  }

  const rows = exts
    .map((ext) => ({
      name: ext.name,
      health: healthOf(health, ext.name),
      secrets: secretCredentials(ext),
      credentials: extensionCredentials(collectors, ext.name),
    }))
    .filter((r) => r.health.length > 0 || r.secrets.length > 0 || r.credentials.length > 0);

  return (
    <div data-testid="extensions-section">
      {rows.length === 0 ? (
        <div data-testid="extensions-health-empty" style={mutedStyle}>
          Nothing to repair and no credentials to set.
        </div>
      ) : (
        <>
          <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
            <EnableAllButton count={disabledCount(health)} />
          </div>
          <ul style={{ listStyle: "none", margin: "0 0 12px", padding: 0 }}>
            {rows.map((r) => (
              <li key={r.name} data-testid={`extension-row-${r.name}`} style={rowStyle}>
                <strong>{r.name}</strong>
                {disabledCount(health, r.name) > 1 ? (
                  <div style={{ marginTop: 4 }}>
                    <EnableAllButton extension={r.name} count={disabledCount(health, r.name)} />
                  </div>
                ) : null}
                {r.health.map((h) => (
                  <HealthRow key={`${h.kind}:${h.contribution}`} health={h} />
                ))}
                {r.secrets.map((c) => (
                  <SecretRow key={`${c.provider}/${c.name}`} extension={r.name} credential={c} />
                ))}
                {r.credentials.map((c) => (
                  <CredentialRow
                    key={c.name}
                    owner={r.name}
                    name={c.name}
                    set={c.set}
                    store={(v) => setCredential(r.name, c.name, v)}
                    onChanged={() => void refresh()}
                  />
                ))}
              </li>
            ))}
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
const errorInlineStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const warnStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-medium)" };
