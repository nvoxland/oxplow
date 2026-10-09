import { useEffect, useRef, useState } from "react";

import { listAcpAgents, type Thread } from "../api.js";
import { agentChoices, parseAgentChoice } from "../agentKinds.js";
import type { AcpAgentListing, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page, pageH1Style } from "../tabs/Page.js";

interface NewSessionPageProps {
  thread: Thread | null;
  /** The registered harnesses in priority order (`useAgentHarnesses`). */
  harnesses: HarnessListing[];
  /** Open the session; resolves once its row exists (its tab follows).
   *  `remember`: new threads start this agent from now on. */
  onStart(harness: string, acpAgent: string | null, remember: boolean): Promise<void>;
  /** Leave the thread without a session (the picker closes).
   *  `remember`: new threads start with none from now on. */
  onNoSession(remember: boolean): Promise<void>;
}

/**
 * The session picker: what a new thread opens with when the person's
 * `newThreadSession` is `ask`. Starting one only opens its slot — the new
 * tab starts its process when it mounts, and nothing is typed into it.
 * It offers no prompts to hand an agent: there is no agent here yet.
 */
export function NewSessionPage({ thread, harnesses, onStart, onNoSession }: NewSessionPageProps) {
  const [acpAgents, setAcpAgents] = useState<AcpAgentListing[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remember, setRemember] = useState(false);
  const acpEnabled = harnesses.some((h) => h.enabled && h.chat);
  useEffect(() => {
    if (!acpEnabled) return;
    void listAcpAgents()
      .then(setAcpAgents)
      .catch(() => setAcpAgents([]));
  }, [acpEnabled]);
  const choices = agentChoices(harnesses, acpAgents);
  const [choice, setChoice] = useState<string>(choices[0]?.value ?? "");
  // Focus the picker when the page opens — unless focus is already
  // somewhere (the launcher a person opened while it loaded): a page that
  // mounts late must not take their typing.
  const agentRef = useRef<HTMLSelectElement>(null);
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body) agentRef.current?.focus();
  }, []);
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
                await onStart(harness, acpAgent, remember);
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
              ref={agentRef}
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
            <button
              type="button"
              data-testid="new-session-none"
              disabled={busy}
              style={noneStyle}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  await onNoSession(remember);
                } catch (err) {
                  setError(err instanceof Error ? err.message : String(err));
                } finally {
                  setBusy(false);
                }
              }}
            >
              No Session in This Thread
            </button>
          </form>
          <label style={rememberStyle} title="Change it later in Settings → Agents">
            <input
              type="checkbox"
              data-testid="new-session-remember"
              checked={remember}
              disabled={busy}
              onChange={(e) => setRemember(e.target.checked)}
            />
            Remember this for new threads
            <span style={{ color: "var(--text-muted)" }}>(Settings → Agents changes it)</span>
          </label>
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

const noneStyle = {
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "4px 12px",
  fontSize: "var(--text-sm)",
  cursor: "pointer",
} as const;

const rememberStyle = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  marginTop: 10,
  fontSize: "var(--text-sm)",
  color: "var(--text-secondary)",
  cursor: "pointer",
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
