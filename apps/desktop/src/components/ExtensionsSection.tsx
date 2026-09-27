/// "Extensions" section body for SettingsPage: every project extension
/// (`oxplow/extensions/`) with its origin, lens count and load errors,
/// its sources' credentials (set here, kept in the keychain); an Update
/// action for git-installed ones; and an install strip for a git URL.
/// Running sources is under Data (DataSection.tsx). See
/// `.context/extensions.md`.
///
/// Usability contract (.context/usability.md): no modals; Enter submits
/// the install strip; failures land in opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  installExtension,
  listExtensions,
  listSources,
  setExtensionEnabled,
  setSourceCredential,
  subscribeOxplowEvents,
  updateExtension,
  type Extension,
  type SourceListing,
} from "../api.js";
import { extensionCredentials, extensionRowModel } from "./extensionRowModel.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function ExtensionsSection() {
  const [exts, setExts] = useState<Extension[] | null>(null);
  const [sources, setSources] = useState<SourceListing[]>([]);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setExts(await listExtensions(null));
      setSources(await listSources());
    } catch (e) {
      recordOpError({ label: "List extensions", message: String(e) });
      setExts([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Scheduled and agent-triggered runs land here too.
    return subscribeOxplowEvents((event) => {
      if (event.kind === "sourceSynced") void refresh();
    });
  }, [refresh]);

  async function install() {
    const gitUrl = url.trim();
    if (!gitUrl || busy) return;
    setBusy("install");
    try {
      const ext = await installExtension(gitUrl, null, null);
      setUrl("");
      showToast({ message: `Installed ${ext.name}. Commit oxplow/extensions/${ext.name} to share it with your team.` });
      await refresh();
    } catch (e) {
      recordOpError({ label: `Install extension from ${gitUrl}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  async function toggle(name: string, enabled: boolean) {
    setBusy(name);
    try {
      setExts(await setExtensionEnabled(name, enabled));
      setSources(await listSources());
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
      await updateExtension(name, null);
      showToast({ message: `Updated ${name}.` });
      await refresh();
    } catch (e) {
      recordOpError({ label: `Update extension ${name}`, message: String(e) });
    } finally {
      setBusy(null);
    }
  }

  return (
    <div data-testid="extensions-section">
      {exts === null ? (
        <div style={mutedStyle}>Loading…</div>
      ) : exts.length === 0 ? (
        <div style={mutedStyle} data-testid="extensions-empty">
          No extensions yet. Ask your agent for a lens, or install one below.
        </div>
      ) : (
        <ul style={{ listStyle: "none", margin: "0 0 12px", padding: 0 }}>
          {exts.map((ext) => {
            const m = extensionRowModel(ext);
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
                      {busy === m.name ? "Updating…" : "Update"}
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
                {extensionCredentials(sources, m.name).map((c) => (
                  <CredentialRow
                    key={c.name}
                    extension={m.name}
                    name={c.name}
                    set={c.set}
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
            if (e.key === "Enter") void install();
            if (e.key === "Escape") setUrl("");
          }}
          style={{ flex: 1 }}
        />
        <button
          type="button"
          data-testid="extension-install"
          disabled={!url.trim() || busy !== null}
          onClick={() => void install()}
        >
          {busy === "install" ? "Installing…" : "Install"}
        </button>
      </div>
    </div>
  );
}

/// One declared credential: set or replace its value (it goes to the OS
/// keychain and never comes back), or clear it.
function CredentialRow({
  extension,
  name,
  set,
  onChanged,
}: {
  extension: string;
  name: string;
  set: boolean;
  onChanged(): void;
}) {
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);
  const id = `${extension}-${name}`;

  async function save(v: string | null) {
    setSaving(true);
    try {
      await setSourceCredential(extension, name, v);
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
      data-testid={`source-credential-${id}`}
      style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) void save(value.trim());
      }}
    >
      <code>{name}</code>
      <span style={mutedStyle}>{set ? "Saved in keychain" : "Not set"}</span>
      <input
        data-testid={`source-credential-input-${id}`}
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
      <button type="submit" data-testid={`source-credential-save-${id}`} disabled={!value.trim() || saving}>
        {saving ? "Saving…" : "Save"}
      </button>
      {set ? (
        <InlineConfirm
          triggerLabel="Clear"
          confirmLabel="Clear"
          testIdPrefix={`source-credential-clear-${id}`}
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
