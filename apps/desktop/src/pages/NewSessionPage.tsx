import { useEffect, useState } from "react";

import { listAcpAgents, type Thread } from "../api.js";
import { agentChoices, parseAgentChoice } from "../agentKinds.js";
import type { AcpAgentListing, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page, pageH1Style } from "../tabs/Page.js";

interface NewSessionPageProps {
  thread: Thread | null;
  /** The registered harnesses in priority order (`useAgentHarnesses`). */
  harnesses: HarnessListing[];
  /** Open the session; resolves once its row exists (its tab follows). */
  onStart(harness: string, acpAgent: string | null): Promise<void>;
}

/**
 * The session picker: what a thread with no agent session shows, and
 * where "New session…" lands. Starting one only opens its slot — the new
 * tab starts its process when it mounts, and nothing is typed into it.
 * It offers no prompts to hand an agent: there is no agent here yet.
 */
export function NewSessionPage({ thread, harnesses, onStart }: NewSessionPageProps) {
  const [acpAgents, setAcpAgents] = useState<AcpAgentListing[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const acpEnabled = harnesses.some((h) => h.enabled && h.chat);
  useEffect(() => {
    if (!acpEnabled) return;
    void listAcpAgents()
      .then(setAcpAgents)
      .catch(() => setAcpAgents([]));
  }, [acpEnabled]);
  const choices = agentChoices(harnesses, acpAgents);
  const [choice, setChoice] = useState<string>(choices[0]?.value ?? "");
  const picked = choices.some((c) => c.value === choice) ? choice : (choices[0]?.value ?? "");
  return (
    <Page testId="page-new-session" showNavBar={false} titleInBody>
      <div style={{ padding: "20px 24px", maxWidth: 560 }}>
        <h1 style={pageH1Style}>New agent session</h1>
        <EmptyState
          testId="new-session-empty"
          title={thread ? `“${thread.title}” has no agent running here yet` : "No thread selected"}
          text="Pick an agent and start it. It runs in its own tab; a thread can have several."
        >
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              if (!thread || busy) return;
              setBusy(true);
              setError(null);
              try {
                const { harness, acpAgent } = parseAgentChoice(picked);
                await onStart(harness, acpAgent);
              } catch (err) {
                setError(err instanceof Error ? err.message : String(err));
              } finally {
                setBusy(false);
              }
            }}
            style={{ display: "flex", gap: 8, alignItems: "center" }}
          >
            <select
              data-testid="new-session-agent"
              value={picked}
              disabled={busy || !thread}
              onChange={(e) => setChoice(e.target.value)}
              style={selectStyle}
              autoFocus
            >
              {choices.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
            <button type="submit" data-testid="new-session-start" disabled={busy || !thread || !picked} style={startStyle}>
              Start
            </button>
          </form>
          {error ? (
            <div data-testid="new-session-error" style={{ color: "var(--severity-high)" }}>
              {error}
            </div>
          ) : null}
        </EmptyState>
      </div>
    </Page>
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

const startStyle = {
  background: "var(--accent)",
  color: "var(--text-on-accent, white)",
  border: "none",
  borderRadius: 4,
  padding: "5px 12px",
  fontSize: "var(--text-sm)",
  cursor: "pointer",
} as const;
