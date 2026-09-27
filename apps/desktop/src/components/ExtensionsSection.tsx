/// "Extensions" section body for SettingsPage: every project extension
/// (`oxplow/extensions/`) with its origin, lens count and load errors;
/// an Update action for git-installed ones; and an install strip for a
/// git URL. See `.context/extensions.md`.
///
/// Usability contract (.context/usability.md): no modals; Enter submits
/// the install strip; failures land in opErrorsStore, not alerts.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { installExtension, listExtensions, updateExtension, type Extension } from "../api.js";
import { extensionRowModel } from "./extensionRowModel.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function ExtensionsSection() {
  const [exts, setExts] = useState<Extension[] | null>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setExts(await listExtensions(null));
    } catch (e) {
      recordOpError({ label: "List extensions", message: String(e) });
      setExts([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
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
                {m.errors.map((err, i) => (
                  <div key={i} style={errorStyle}>
                    {err}
                  </div>
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

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
