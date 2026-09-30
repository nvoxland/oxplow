/// "Integrations" section body for SettingsPage (P5.D4): each extension
/// provider's instance on this machine — its state, its config (JSON,
/// edited inline), Check, and Enable / Disable. Enabling checks first;
/// an unapproved provider is approved under Data → Programs. See
/// `.context/providers.md`.
///
/// Usability contract (.context/usability.md): inline edits, Escape
/// resets an edit, a disabled action while the JSON is invalid, failures
/// in opErrorsStore.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  checkProviderInstance,
  listProviderInstances,
  setProviderInstance,
  subscribeOxplowEvents,
  type ProviderInstanceView,
} from "../api.js";
import { CredentialRow } from "./ExtensionsSection.js";
import { integrationRow, parseConfig } from "./integrationsModel.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function IntegrationsSection() {
  const [views, setViews] = useState<ProviderInstanceView[] | null>(null);

  const refresh = useCallback(async () => {
    try {
      setViews(await listProviderInstances());
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
      {views.map((v) => (
        <IntegrationRow key={v.instance} view={v} onChanged={setViews} onCredentialChanged={() => void refresh()} />
      ))}
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
  const saved = JSON.stringify(view.config, null, 2);
  const [text, setText] = useState(saved);
  const [checked, setChecked] = useState<ProviderInstanceView | null>(null);
  const [busy, setBusy] = useState<"check" | "toggle" | null>(null);
  useEffect(() => setText(saved), [saved]);

  const shown = checked ?? view;
  const m = integrationRow(shown);
  const parsed = parseConfig(text);
  const missing = view.health.state.state === "missing";

  async function check() {
    const config = parsed.value;
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

  async function toggle() {
    const config = parsed.value;
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
          disabled={busy !== null || missing || parsed.value === null}
          title="Start it with this config and ask it to check the config; nothing is saved or enabled"
          onClick={() => void check()}
        >
          {busy === "check" ? "Checking…" : "Check"}
        </button>
        <button
          type="button"
          data-testid={`integration-toggle-${m.key}`}
          disabled={busy !== null || missing || parsed.value === null}
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
      {m.needsApproval ? (
        <div style={mutedStyle}>Its program isn&apos;t approved on this machine yet: see Data → Programs.</div>
      ) : null}
      {view.credentials.map((c) => (
        <CredentialRow
          key={c.name}
          extension={view.extension}
          name={c.name}
          set={c.set}
          onChanged={onCredentialChanged}
        />
      ))}
      {missing ? null : (
        <>
          <textarea
            data-testid={`integration-config-${m.key}`}
            aria-label={`${view.instance} config (JSON)`}
            value={text}
            rows={Math.min(8, Math.max(2, text.split("\n").length))}
            onChange={(e) => {
              setText(e.target.value);
              setChecked(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setText(saved);
                setChecked(null);
              }
            }}
            style={textareaStyle}
          />
          {parsed.error ? <div style={errorStyle}>{parsed.error}</div> : null}
        </>
      )}
    </div>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)" };
const rowStyle: CSSProperties = {
  padding: "6px 0",
  borderBottom: "1px solid var(--border-subtle)",
  fontSize: "var(--text-sm)",
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
const textareaStyle: CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: "var(--text-xs)",
  width: "100%",
  boxSizing: "border-box",
};
