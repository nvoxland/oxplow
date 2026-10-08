import { useEffect, useState } from "react";

import { agentChoices } from "../agentKinds.js";
import { listAcpAgents } from "../api.js";
import { readNewThreadSession, saveNewThreadSession } from "../newThreadSession.js";
import type { AcpAgentListing, HarnessListing } from "../tauri-bridge/generated/bindings.js";

/**
 * Settings → Agents: what a new thread starts with — the person's own
 * `newThreadSession` (`.oxplow/personal.yaml`). Saved when it changes.
 */
export function NewThreadSessionSetting({
  harnesses,
  read = readNewThreadSession,
  save = saveNewThreadSession,
}: {
  harnesses: HarnessListing[];
  read?: () => Promise<string>;
  save?: (choice: string) => Promise<void>;
}) {
  const [choice, setChoice] = useState<string | null>(null);
  const [acpAgents, setAcpAgents] = useState<AcpAgentListing[]>([]);
  const [error, setError] = useState<string | null>(null);
  const acpEnabled = harnesses.some((h) => h.enabled && h.chat);
  useEffect(() => {
    void read()
      .then(setChoice)
      .catch((e) => setError(String(e)));
  }, [read]);
  useEffect(() => {
    if (!acpEnabled) return;
    void listAcpAgents()
      .then(setAcpAgents)
      .catch(() => setAcpAgents([]));
  }, [acpEnabled]);
  const options = [
    { value: "ask", label: "Ask (the session picker)" },
    { value: "none", label: "No session" },
    ...agentChoices(harnesses, acpAgents),
  ];
  if (choice !== null && !options.some((o) => o.value === choice)) {
    options.push({ value: choice, label: `${choice} (not available: new threads ask)` });
  }
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 10, flexWrap: "wrap" }}>
      <label htmlFor="settings-new-thread-session" style={{ fontSize: "var(--text-sm)" }}>
        New threads start with
      </label>
      <select
        id="settings-new-thread-session"
        data-testid="settings-new-thread-session"
        value={choice ?? "ask"}
        disabled={choice === null}
        onChange={async (e) => {
          const next = e.target.value;
          const before = choice;
          setChoice(next);
          setError(null);
          try {
            await save(next);
          } catch (err) {
            setChoice(before);
            setError(err instanceof Error ? err.message : String(err));
          }
        }}
        style={selectStyle}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
        Just for you, in <code>.oxplow/personal.yaml</code>; saved when changed.
      </span>
      {error ? (
        <div data-testid="settings-new-thread-session-error" style={{ color: "var(--severity-high)", width: "100%" }}>
          {error}
        </div>
      ) : null}
    </div>
  );
}

const selectStyle = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "4px 6px",
  fontSize: "var(--text-sm)",
} as const;
