import { useEffect, useRef, useState } from "react";

import { listEarlierSessions, type Thread } from "../api.js";
import { agentChoices } from "../agentKinds.js";
import type { EarlierSessions, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { earlierSessionRows, filterEarlierRows, takeable, type EarlierRow } from "../components/earlierSessionRows.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { Page, pageH1Style } from "../tabs/Page.js";

interface NewSessionPageProps {
  thread: Thread | null;
  /** How many sessions the thread has open: none on a new thread, some
   *  when the launcher's New Agent Session… opened the page. */
  openSessions: number;
  /** The registered harnesses in priority order (`useAgentHarnesses`). */
  harnesses: HarnessListing[];
  /** Open the session; resolves once its row exists (its tab follows).
   *  `remember`: new threads start this agent from now on. */
  onStart(harness: string, remember: boolean): Promise<void>;
  /** Leave the thread without a session (the picker closes).
   *  `remember`: new threads start with none from now on. */
  onNoSession(remember: boolean): Promise<void>;
  /** Take an earlier session: reopen it, resume its conversation in a new
   *  session here, or show its tab (`row.action`). Resolves once done (the
   *  picker then gives way to the session). */
  onTakeEarlier?(row: EarlierRow): Promise<void>;
  /** Read the thread's earlier sessions (default: the daemon's). */
  loadEarlier?(threadId: string): Promise<EarlierSessions>;
}

/**
 * The session picker: what a new thread opens with when the person's
 * `newThreadSession` is `ask`. Starting one only opens its slot — the new
 * tab starts its process when it mounts, and nothing is typed into it.
 * It offers no prompts to hand an agent: there is no agent here yet.
 */
export function NewSessionPage({
  thread,
  openSessions,
  harnesses,
  onStart,
  onNoSession,
  onTakeEarlier,
  loadEarlier = listEarlierSessions,
}: NewSessionPageProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remember, setRemember] = useState(false);
  const choices = agentChoices(harnesses);
  const [choice, setChoice] = useState<string>(choices[0]?.value ?? "");
  // Focus the picker when the page opens — unless focus is already
  // somewhere (the launcher a person opened while it loaded): a page that
  // mounts late must not take their typing.
  const agentRef = useRef<HTMLSelectElement>(null);
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body) agentRef.current?.focus();
  }, []);
  // The earlier sessions: read once the page opens; a filter narrows them
  // in the client, and ↑/↓/Enter/Escape work from the filter's input.
  const [earlier, setEarlier] = useState<EarlierSessions | null>(null);
  const [earlierError, setEarlierError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [cursor, setCursor] = useState(0);
  const threadId = thread?.id ?? null;
  useEffect(() => {
    if (!threadId) return;
    let live = true;
    setEarlier(null);
    loadEarlier(threadId)
      .then((listing) => live && setEarlier(listing))
      .catch((e: unknown) => live && setEarlierError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [threadId, loadEarlier]);
  const allRows = earlier ? earlierSessionRows(earlier) : null;
  const rows = allRows ? filterEarlierRows(allRows, filter) : null;
  useEffect(() => setCursor(0), [filter, earlier]);
  const harnessTitle = (id: string) => harnesses.find((h) => h.id === id)?.title ?? id;
  const takeRow = async (row: EarlierRow | undefined) => {
    if (!row || !takeable(row) || busy || !onTakeEarlier) return;
    setBusy(true);
    setError(null);
    try {
      await onTakeEarlier(row);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const onFilterKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      if (filter !== "") {
        e.preventDefault();
        e.stopPropagation();
        setFilter("");
      }
    } else if (e.key === "ArrowDown" && rows?.length) {
      e.preventDefault();
      setCursor((c) => Math.min(rows.length - 1, c + 1));
    } else if (e.key === "ArrowUp" && rows?.length) {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      void takeRow(rows?.[cursor]);
    }
  };
  const picked = choices.some((c) => c.value === choice) ? choice : (choices[0]?.value ?? "");
  return (
    <Page testId="page-new-session" showNavBar={false} titleInBody>
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        <h1 style={pageH1Style}>New agent session</h1>
        <EmptyState
          testId="new-session-empty"
          title={heading(thread, openSessions)}
          text={
            openSessions > 0
              ? "Start another agent, or pick up an earlier session below. Each runs in its own tab."
              : "Pick an agent and start it. It runs in its own tab; a thread can have several."
          }
        >
          <form
            onSubmit={async (e) => {
              e.preventDefault();
              if (!thread || busy) return;
              setBusy(true);
              setError(null);
              try {
                await onStart(picked, remember);
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
            {/* Declining a session is for a thread without one. */}
            {openSessions === 0 ? (
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
            ) : null}
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
        {thread ? (
          <section data-testid="new-session-earlier" style={{ marginTop: 20 }}>
            <h2 style={{ fontSize: "var(--text-md, 14px)", margin: "0 0 8px" }}>Earlier sessions</h2>
            <input
              type="text"
              data-testid="new-session-earlier-filter"
              aria-label="Filter earlier sessions"
              placeholder="Filter by title, prompt, thread or agent"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={onFilterKeyDown}
              style={filterStyle}
            />
            {earlierError ? (
              <div data-testid="new-session-earlier-error" style={{ color: "var(--severity-high)", marginTop: 6 }}>
                {earlierError}
              </div>
            ) : null}
            {rows === null && !earlierError ? (
              <div style={mutedStyle}>Reading saved sessions…</div>
            ) : null}
            {earlier?.unavailable.map((u) => (
              <div key={u.harness} data-testid={`new-session-earlier-unavailable-${u.harness}`} style={mutedStyle}>
                {harnessTitle(u.harness)} couldn't list its saved sessions: {u.reason}
              </div>
            ))}
            {allRows && allRows.length === 0 ? (
              <div data-testid="new-session-earlier-empty" style={mutedStyle}>
                No earlier sessions for this worktree.
              </div>
            ) : null}
            {allRows && allRows.length > 0 && rows?.length === 0 ? (
              <div data-testid="new-session-earlier-nomatch" style={mutedStyle}>
                Nothing matches the filter.
              </div>
            ) : null}
            {rows?.length ? (
              <div role="listbox" aria-label="Earlier sessions" style={{ marginTop: 6 }}>
                {rows.map((row, i) => (
                  <button
                    key={row.key}
                    type="button"
                    role="option"
                    aria-selected={i === cursor}
                    data-testid={`earlier-row-${row.key}`}
                    data-action={row.action}
                    disabled={!takeable(row) || busy}
                    onMouseEnter={() => setCursor(i)}
                    onClick={() => void takeRow(row)}
                    style={{
                      ...rowStyle,
                      background: i === cursor ? "rgba(74, 158, 255, 0.18)" : "transparent",
                      color: takeable(row) ? "var(--text-primary)" : "var(--text-muted)",
                      cursor: takeable(row) ? "pointer" : "default",
                    }}
                  >
                    <span style={tagStyle}>{harnessTitle(row.harness)}</span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{row.label}</span>
                    <span style={{ fontSize: 11, color: "var(--text-muted)" }}>{row.note}</span>
                    <span style={{ fontSize: 11, color: "var(--text-muted)" }}>{[row.when, row.detail].filter(Boolean).join(" · ")}</span>
                  </button>
                ))}
              </div>
            ) : null}
          </section>
        ) : null}
      </div>
    </Page>
  );
}

const filterStyle = {
  width: "100%",
  boxSizing: "border-box",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "5px 8px",
  fontSize: "var(--text-sm)",
} as const;

const mutedStyle = { color: "var(--text-muted)", fontSize: "var(--text-sm)", marginTop: 6 } as const;

const rowStyle = {
  display: "flex",
  width: "100%",
  gap: 10,
  alignItems: "baseline",
  padding: "6px 8px",
  border: "none",
  borderRadius: 4,
  textAlign: "left",
  fontSize: "var(--text-sm)",
} as const;

const tagStyle = {
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.4,
  color: "var(--text-secondary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 3,
  padding: "0 4px",
} as const;

function heading(thread: Thread | null, openSessions: number): string {
  if (!thread) return "No thread selected";
  if (openSessions === 0) return `“${thread.title}” has no agent running here yet`;
  return `“${thread.title}” has ${openSessions} agent session${openSessions === 1 ? "" : "s"} open`;
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
