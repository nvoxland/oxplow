/// "AI" section body for SettingsPage: the model providers oxplow may call
/// (keys go to the OS keychain and never come back, only whether one is
/// set), which model each role uses, and the last week of calls from
/// `v_ai_call`. See `.context/ai-providers.md`.
///
/// Usability contract (.context/usability.md): no modals; forms submit on
/// Enter and Escape cancels; Remove uses InlineConfirm; failures land in
/// opErrorsStore, not alerts. Changes apply immediately, no Save needed.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import {
  aiSettings,
  querySql,
  removeAiProvider,
  saveAiProvider,
  setAiRole,
  testAiProvider,
  type AiSettings,
  type Role,
} from "../api.js";
import {
  emptyProviderForm,
  kindLabel,
  PROVIDER_KINDS,
  providerFormError,
  roleRows,
  testModelFor,
  usageRows,
  USAGE_SQL,
  type ProviderForm,
  type UsageRow,
} from "./aiSettingsModel.js";
import { formatMetricValue, formatMetricValueExact } from "./format.js";
import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "./opErrorsStore.js";
import { showToast } from "./toastStore.js";

export function AiSection() {
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [usage, setUsage] = useState<UsageRow[]>([]);

  const refresh = useCallback(async () => {
    try {
      setSettings(await aiSettings());
      setUsage(usageRows(await querySql(USAGE_SQL)));
    } catch (e) {
      recordOpError({ label: "Load AI settings", message: String(e) });
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (settings === null) return <div style={mutedStyle}>Loading…</div>;

  return (
    <div data-testid="ai-section">
      <h3 style={subheadStyle}>Providers</h3>
      {settings.providers.length === 0 ? (
        <div style={mutedStyle} data-testid="ai-providers-empty">
          No providers yet. Add one below: an API key for a hosted service, or the URL of a local server.
        </div>
      ) : (
        <ul style={listStyle}>
          {settings.providers.map((p) => (
            <ProviderRow key={p.id} settings={settings} providerId={p.id} onChanged={setSettings} />
          ))}
        </ul>
      )}
      <ProviderEditor settings={settings} onSaved={setSettings} />

      <h3 style={subheadStyle}>Roles</h3>
      <div style={mutedStyle}>Oxplow and extensions ask for a role, never a model. Assign each role you want to use.</div>
      <ul style={listStyle}>
        {roleRows(settings).map((row) => (
          <RoleRowEditor key={row.role} settings={settings} role={row.role} onSaved={setSettings} />
        ))}
      </ul>

      <h3 style={subheadStyle}>Recent Calls</h3>
      {usage.length === 0 ? (
        <div style={mutedStyle} data-testid="ai-usage-empty">
          No calls in the last 7 days.
        </div>
      ) : (
        <table style={tableStyle} data-testid="ai-usage">
          <thead>
            <tr>
              <th style={thStyle}>Role</th>
              <th style={thStyle}>Caller</th>
              <th style={thNumStyle}>Calls</th>
              <th style={thNumStyle}>Failed</th>
              <th style={thNumStyle}>Tokens In</th>
              <th style={thNumStyle}>Tokens Out</th>
              <th style={thStyle}>Last</th>
            </tr>
          </thead>
          <tbody>
            {usage.map((u) => (
              <tr key={`${u.role}/${u.caller}`}>
                <td style={tdStyle}>{u.role}</td>
                <td style={tdStyle}>
                  <code>{u.caller}</code>
                </td>
                <td style={tdNumStyle} title={formatMetricValueExact(u.calls)}>
                  {formatMetricValue(u.calls)}
                </td>
                <td style={tdNumStyle} title={formatMetricValueExact(u.failed)}>
                  {formatMetricValue(u.failed)}
                </td>
                <td style={tdNumStyle} title={formatMetricValueExact(u.inputTokens)}>
                  {formatMetricValue(u.inputTokens)}
                </td>
                <td style={tdNumStyle} title={formatMetricValueExact(u.outputTokens)}>
                  {formatMetricValue(u.outputTokens)}
                </td>
                <td style={tdStyle}>{u.lastAt ? new Date(u.lastAt).toLocaleString() : ""}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

function ProviderRow({
  settings,
  providerId,
  onChanged,
}: {
  settings: AiSettings;
  providerId: string;
  onChanged(s: AiSettings): void;
}) {
  const p = settings.providers.find((x) => x.id === providerId)!;
  const [model, setModel] = useState(() => testModelFor(settings, p.id));
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);

  async function test() {
    if (!model.trim() || testing) return;
    setTesting(true);
    setResult(null);
    try {
      const reply = await testAiProvider(p.id, model.trim());
      setResult({ ok: true, text: `Works. Reply: ${reply}` });
    } catch (e) {
      setResult({ ok: false, text: String(e) });
    } finally {
      setTesting(false);
    }
  }

  async function remove() {
    try {
      onChanged(await removeAiProvider(p.id));
      showToast({ message: `Removed ${p.id} and its key.` });
    } catch (e) {
      recordOpError({ label: `Remove provider ${p.id}`, message: String(e) });
    }
  }

  return (
    <li data-testid={`ai-provider-row-${p.id}`} style={rowStyle}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <strong>{p.id}</strong>
        <span style={mutedStyle}>
          {kindLabel(p.kind)}
          {p.baseUrl ? ` · ${p.baseUrl}` : ""} · {p.keySet ? "Key saved" : "No key"}
        </span>
        <span style={{ flex: 1 }} />
        <form
          style={{ display: "flex", gap: 4 }}
          onSubmit={(e) => {
            e.preventDefault();
            void test();
          }}
        >
          <input
            data-testid={`ai-provider-test-model-${p.id}`}
            value={model}
            placeholder="Model to test"
            onChange={(e) => setModel(e.target.value)}
            style={{ width: 180 }}
          />
          <button type="submit" data-testid={`ai-provider-test-${p.id}`} disabled={!model.trim() || testing}>
            {testing ? "Testing…" : "Test"}
          </button>
        </form>
        <InlineConfirm triggerLabel="Remove" confirmLabel="Remove" testIdPrefix={`ai-provider-remove-${p.id}`} onConfirm={() => void remove()} />
      </div>
      {result ? (
        <div data-testid={`ai-provider-test-result-${p.id}`} style={result.ok ? mutedStyle : errorStyle}>
          {result.text}
        </div>
      ) : null}
    </li>
  );
}

/// Add a provider, or replace an existing one's settings / key by using
/// its name.
function ProviderEditor({ settings, onSaved }: { settings: AiSettings; onSaved(s: AiSettings): void }) {
  const [form, setForm] = useState<ProviderForm>(emptyProviderForm);
  const [saving, setSaving] = useState(false);
  const existing = settings.providers.find((p) => p.id === form.id.trim());
  const editing = { ...form, editing: existing !== undefined };
  const problem = providerFormError(editing, settings.providers);
  const hint = PROVIDER_KINDS.find((k) => k.kind === form.kind)?.baseUrlHint ?? "";

  async function save() {
    if (problem || saving) return;
    setSaving(true);
    try {
      const id = form.id.trim();
      const saved = await saveAiProvider(
        { id, kind: form.kind, baseUrl: form.baseUrl.trim() || null },
        form.key.trim() || null,
      );
      onSaved(saved);
      setForm(emptyProviderForm());
      showToast({ message: form.key.trim() ? `Saved ${id}; its key is in your keychain.` : `Saved ${id}.` });
    } catch (e) {
      recordOpError({ label: "Save AI provider", message: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      data-testid="ai-provider-form"
      style={formStyle}
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") setForm(emptyProviderForm());
      }}
    >
      <input
        data-testid="ai-provider-id"
        value={form.id}
        placeholder="Name, e.g. openrouter"
        onChange={(e) => setForm({ ...form, id: e.target.value })}
        style={{ width: 150 }}
      />
      <select
        data-testid="ai-provider-kind"
        value={form.kind}
        onChange={(e) => setForm({ ...form, kind: e.target.value as ProviderForm["kind"] })}
      >
        {PROVIDER_KINDS.map((k) => (
          <option key={k.kind} value={k.kind}>
            {k.label}
          </option>
        ))}
      </select>
      <input
        data-testid="ai-provider-base-url"
        value={form.baseUrl}
        placeholder={form.kind === "openai-compatible" ? hint : `Base URL (default ${hint})`}
        onChange={(e) => setForm({ ...form, baseUrl: e.target.value })}
        style={{ flex: 1, minWidth: 180 }}
      />
      <input
        data-testid="ai-provider-key"
        type="password"
        autoComplete="off"
        value={form.key}
        placeholder={existing?.keySet ? "Leave blank to keep the saved key" : "API key (optional for local servers)"}
        onChange={(e) => setForm({ ...form, key: e.target.value })}
        style={{ width: 220 }}
      />
      <button type="submit" data-testid="ai-provider-save" disabled={problem !== null || saving} title={problem ?? undefined}>
        {saving ? "Saving…" : existing ? "Update" : "Add"}
      </button>
      {form.id && problem ? <div style={{ ...mutedStyle, width: "100%" }}>{problem}</div> : null}
    </form>
  );
}

function RoleRowEditor({ settings, role, onSaved }: { settings: AiSettings; role: Role; onSaved(s: AiSettings): void }) {
  const row = roleRows(settings).find((r) => r.role === role)!;
  const binding = settings.roles.find((r) => r.role === role)?.binding ?? null;
  const [provider, setProvider] = useState(binding?.provider ?? "");
  const [model, setModel] = useState(binding?.model ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setProvider(binding?.provider ?? "");
    setModel(binding?.model ?? "");
  }, [binding?.provider, binding?.model]);

  const dirty = provider !== (binding?.provider ?? "") || model !== (binding?.model ?? "");
  const valid = provider === "" || model.trim() !== "";

  async function save() {
    if (!dirty || !valid || saving) return;
    setSaving(true);
    try {
      onSaved(await setAiRole(role, provider ? { provider, model: model.trim() } : null));
    } catch (e) {
      recordOpError({ label: `Assign the ${role} role`, message: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <li data-testid={`ai-role-row-${role}`} style={rowStyle}>
      <form
        style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            setProvider(binding?.provider ?? "");
            setModel(binding?.model ?? "");
          }
        }}
      >
        <strong style={{ width: 90 }}>{role}</strong>
        <span style={{ ...mutedStyle, flex: 1, minWidth: 160 }}>
          {row.usedFor}
          {row.note ? ` · ${row.note}` : ""}
        </span>
        <select
          data-testid={`ai-role-provider-${role}`}
          value={provider}
          disabled={!row.editable}
          title={row.lockedReason ?? undefined}
          onChange={(e) => setProvider(e.target.value)}
        >
          <option value="">Not assigned</option>
          {settings.providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.id}
            </option>
          ))}
          {provider && !settings.providers.some((p) => p.id === provider) ? <option value={provider}>{provider} (missing)</option> : null}
        </select>
        <input
          data-testid={`ai-role-model-${role}`}
          value={model}
          disabled={!provider || !row.editable}
          title={row.lockedReason ?? undefined}
          placeholder={provider ? "Model, e.g. openai/gpt-5-mini" : ""}
          onChange={(e) => setModel(e.target.value)}
          style={{ width: 220 }}
        />
        <button type="submit" data-testid={`ai-role-save-${role}`} disabled={!row.editable || !dirty || !valid || saving}>
          {saving ? "Saving…" : "Save"}
        </button>
      </form>
      {row.problem ? <div style={errorStyle}>{row.problem}</div> : null}
    </li>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const errorStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--severity-critical)", marginTop: 4 };
const subheadStyle: CSSProperties = { fontSize: "var(--text-sm)", margin: "12px 0 4px" };
const listStyle: CSSProperties = { listStyle: "none", margin: "0 0 8px", padding: 0 };
const rowStyle: CSSProperties = { padding: "6px 0", borderBottom: "1px solid var(--border-subtle)", fontSize: "var(--text-sm)" };
const formStyle: CSSProperties = { display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center", marginTop: 6 };
const tableStyle: CSSProperties = { borderCollapse: "collapse", fontSize: "var(--text-xs)", width: "100%" };
const thStyle: CSSProperties = { textAlign: "left", padding: "4px 8px 4px 0", color: "var(--text-secondary)", fontWeight: 500 };
const thNumStyle: CSSProperties = { ...thStyle, textAlign: "right" };
const tdStyle: CSSProperties = { padding: "4px 8px 4px 0", borderTop: "1px solid var(--border-subtle)" };
const tdNumStyle: CSSProperties = { ...tdStyle, textAlign: "right", fontVariantNumeric: "tabular-nums" };
