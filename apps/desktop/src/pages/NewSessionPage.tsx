import { useEffect, useRef, useState } from "react";
import { ArrowLeftRight, CircleDot, ExternalLink, Lock, RotateCcw, type LucideIcon } from "lucide-react";

import { listEarlierSessions, type Thread } from "../api.js";
import { agentChoices } from "../agentKinds.js";
import type { EarlierSessions, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { earlierSessionRows, filterEarlierRows, takeable, type EarlierRow, type EarlierStatus } from "../components/earlierSessionRows.js";
import { Page, pageH1Style } from "../tabs/Page.js";

interface NewSessionPageProps {
  thread: Thread | null;
  /** The registered harnesses in priority order (`useAgentHarnesses`). */
  harnesses: HarnessListing[];
  /** Open the session; resolves once its row exists (its tab follows).
   *  `remember`: new threads start this agent from now on. */
  onStart(harness: string, remember: boolean): Promise<void>;
  /** Take an earlier session: reopen it, resume its conversation in a new
   *  session here, or show its tab (`row.action`). Resolves once done: the
   *  picker gives way to a reopened or resumed session, and stays open
   *  behind one it only showed. */
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
  harnesses,
  onStart,
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
      <div style={{ padding: "20px 24px", maxWidth: 960 }}>
        <h1 style={pageH1Style}>Agent sessions</h1>
        <section data-testid="new-session-new" style={newCardStyle}>
          <h2 style={sectionH2Style}>Start a new session</h2>
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
              New Session
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
          </label>
          {error ? (
            <div data-testid="new-session-error" style={{ color: "var(--severity-high)" }}>
              {error}
            </div>
          ) : null}
        </section>
        {thread ? (
          <section data-testid="new-session-earlier" style={{ marginTop: 32 }}>
            <h2 style={sectionH2Style}>Earlier sessions</h2>
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
                <div aria-hidden style={{ ...rowStyle, ...headerStyle }}>
                  <span>Agent</span>
                  <span />
                  <span>Session</span>
                  <span>Last active</span>
                  <span style={numberStyle}>Turns</span>
                  <span style={numberStyle}>Size</span>
                </div>
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
                    <span>
                      <span style={tagStyle}>{harnessTitle(row.harness)}</span>
                    </span>
                    <StatusIcon status={row.status} note={row.note} />
                    <span style={cellStyle} title={row.label}>
                      {row.label}
                    </span>
                    <span style={metaStyle}>{row.when ?? ""}</span>
                    <span style={{ ...metaStyle, ...numberStyle }}>{row.turns ?? ""}</span>
                    <span style={{ ...metaStyle, ...numberStyle }}>{row.size ?? ""}</span>
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

const STATUS_ICONS: Record<EarlierStatus, LucideIcon> = {
  open_here: CircleDot,
  closed_here: RotateCcw,
  elsewhere: ArrowLeftRight,
  outside: ExternalLink,
  open_elsewhere: Lock,
};

/** Where a row stands, as an icon; its words on hover. */
function StatusIcon({ status, note }: { status: EarlierStatus; note: string }) {
  const Icon = STATUS_ICONS[status];
  return (
    <span title={note} aria-label={note} data-status={status} style={{ display: "inline-flex", alignSelf: "center", color: "var(--text-muted)" }}>
      <Icon size={13} aria-hidden />
    </span>
  );
}

// The new session's controls sit in a card of their own, set off from the
// list of earlier sessions below.
const newCardStyle = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "14px 16px",
  marginTop: 12,
} as const;

const sectionH2Style = { fontSize: "var(--text-md, 14px)", margin: "0 0 10px" } as const;

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

// One column template for the header and every row, so each piece of a
// row lines up with the same piece of the others: agent, status icon,
// session (taking what the others leave), last active, turns, size.
const rowStyle = {
  display: "grid",
  gridTemplateColumns: "60px 16px minmax(0, 1fr) 104px 40px 60px",
  width: "100%",
  columnGap: 12,
  alignItems: "baseline",
  padding: "6px 8px",
  border: "none",
  borderRadius: 4,
  textAlign: "left",
  fontSize: "var(--text-sm)",
} as const;

const headerStyle = {
  fontSize: 11,
  fontWeight: 600,
  color: "var(--text-secondary)",
  borderBottom: "1px solid var(--border-subtle)",
  borderRadius: 0,
  padding: "4px 8px",
} as const;

const cellStyle = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" } as const;

const metaStyle = { fontSize: 11, color: "var(--text-muted)", fontVariantNumeric: "tabular-nums" } as const;

const numberStyle = { textAlign: "right" } as const;

const tagStyle = {
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.4,
  color: "var(--text-secondary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 3,
  padding: "0 4px",
} as const;

const selectStyle = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "4px 6px",
  fontSize: "var(--text-sm)",
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
