import { useEffect, useRef, useState } from "react";
import { usePageMemory } from "../prefs/remembered.js";
import { ArrowLeftRight, CircleDot, ExternalLink, Lock, RotateCcw, type LucideIcon } from "lucide-react";

import { listEarlierSessions, type Thread } from "../api.js";
import { agentChoices } from "../agentKinds.js";
import type { EarlierSessions, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { AgentIcon } from "../components/AgentIcon.js";
import { earlierSessionRows, filterEarlierRows, takeable, type EarlierRow, type EarlierStatus } from "../components/earlierSessionRows.js";
import { numericRowId } from "../lens/lensModel.js";
import { DataTable, type DataColumn } from "../kit/DataTable.js";
import { DocumentLayout } from "../kit/DocumentLayout.js";
import { Loading } from "../kit/Loading.js";
import { Toolbar } from "../kit/Toolbar.js";
import { useListNav } from "../kit/useListNav.js";
import type { TabRef } from "../tabs/tabState.js";

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
  streamId?: string | null;
  onOpenPage?(ref: TabRef): void;
}

/**
 * The session picker, a Document page: start a new session, or take an
 * earlier one. What a new thread opens with when the person's
 * `newThreadSession` is `ask`. Starting one only opens its slot — the new
 * tab starts its process when it mounts, and nothing is typed into it.
 * Extensions add sections through `new-session.sections` (ids `start`,
 * `earlier`; it binds `thread_id`).
 */
export function NewSessionPage({
  thread,
  harnesses,
  onStart,
  onTakeEarlier,
  loadEarlier = listEarlierSessions,
  streamId = null,
  onOpenPage,
}: NewSessionPageProps) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remember, setRemember] = useState(false);
  const choices = agentChoices(harnesses);
  // The agent last chosen is this page's memory, kept for the person.
  const [choice, setChoice] = usePageMemory<string>("agent", choices[0]?.value ?? "");
  // Focus the picker when the page opens — unless focus is already
  // somewhere (the launcher a person opened while it loaded): a page that
  // mounts late must not take their typing.
  const agentRef = useRef<HTMLSelectElement>(null);
  useEffect(() => {
    const active = document.activeElement;
    if (!active || active === document.body) agentRef.current?.focus();
  }, []);
  // The earlier sessions: read once the page opens; a filter narrows them
  // in the client, driving the list from its input.
  const [earlier, setEarlier] = useState<EarlierSessions | null>(null);
  const [earlierError, setEarlierError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
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
  const harness = (id: string) => harnesses.find((h) => h.id === id);
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
  const nav = useListNav({
    count: rows?.length ?? 0,
    onPick: (i) => void takeRow(rows?.[i]),
    onEscape: () => {
      if (filter === "") return false;
      setFilter("");
      return true;
    },
    id: "earlier-sessions",
  });
  useEffect(() => nav.setCursor(0), [filter, earlier]); // eslint-disable-line react-hooks/exhaustive-deps
  const picked = choices.some((c) => c.value === choice) ? choice : (choices[0]?.value ?? "");

  const columns: DataColumn<EarlierRow>[] = [
    {
      key: "agent",
      label: <span className="ox-sr-only">Agent</span>,
      width: 28,
      render: (r) => (
        <span title={harness(r.harness)?.title ?? r.harness} aria-label={harness(r.harness)?.title ?? r.harness} data-harness={r.harness} style={{ display: "inline-flex" }}>
          <AgentIcon icon={harness(r.harness)?.icon} size={14} />
        </span>
      ),
    },
    { key: "status", label: <span className="ox-sr-only">Status</span>, width: 24, render: (r) => <StatusIcon status={r.status} note={r.note} /> },
    { key: "session", label: "Session", truncate: true, render: (r) => r.label, title: (r) => r.label },
    { key: "when", label: "Last active", width: 96, render: (r) => <Faint value={r.when} />, title: (r) => r.whenExact ?? undefined },
    { key: "turns", label: "Turns", width: 52, numeric: true, render: (r) => <Faint value={r.turns} /> },
    { key: "size", label: "Size", width: 72, numeric: true, render: (r) => <Faint value={r.size} /> },
  ];

  const start = (
    <>
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
        style={{ display: "flex", flexWrap: "wrap", gap: "var(--space-3)", alignItems: "center" }}
      >
        <select
          data-testid="new-session-agent"
          className="ox-select"
          value={picked}
          disabled={busy || !thread}
          onChange={(e) => setChoice(e.target.value)}
          ref={agentRef}
        >
          {choices.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
        <button type="submit" data-testid="new-session-start" className="ox-button ox-primary" disabled={busy || !thread || !picked}>
          New Session
        </button>
        <label className="ox-muted" style={{ display: "flex", alignItems: "center", gap: "var(--space-1)", cursor: "pointer" }} title="Change it later in Settings → Agents">
          <input
            type="checkbox"
            data-testid="new-session-remember"
            checked={remember}
            disabled={busy}
            onChange={(e) => setRemember(e.target.checked)}
          />
          Remember this for new threads
        </label>
      </form>
      {error ? (
        <p data-testid="new-session-error" className="ox-error">
          {error}
        </p>
      ) : null}
    </>
  );

  const earlierBody = (
    <>
      <Toolbar
        filter={{
          value: filter,
          onChange: setFilter,
          label: "Filter earlier sessions",
          placeholder: "Filter by title, prompt, thread or agent",
          testId: "new-session-earlier-filter",
          inputProps: nav.inputProps,
        }}
      />
      {earlierError ? (
        <p data-testid="new-session-earlier-error" className="ox-error">
          {earlierError}
        </p>
      ) : null}
      {rows === null && !earlierError ? <Loading what="saved sessions" /> : null}
      {earlier?.unavailable.map((u) => (
        <p key={u.harness} data-testid={`new-session-earlier-unavailable-${u.harness}`} className="ox-empty-line">
          {harness(u.harness)?.title ?? u.harness} couldn't list its saved sessions: {u.reason}
        </p>
      ))}
      {allRows && allRows.length === 0 ? (
        <p data-testid="new-session-earlier-empty" className="ox-empty-line">
          No earlier sessions for this worktree.
        </p>
      ) : null}
      {allRows && allRows.length > 0 && rows?.length === 0 ? (
        <p data-testid="new-session-earlier-nomatch" className="ox-empty-line">
          Nothing matches the filter.
        </p>
      ) : null}
      {allRows?.length ? <StatusKey statuses={allRows.map((r) => r.status)} /> : null}
      {rows?.length ? (
        <DataTable<EarlierRow>
          testId="new-session-earlier-rows"
          columns={columns}
          rows={rows}
          rowKey={(r) => r.key}
          tableProps={{ ...nav.listProps, "aria-label": "Earlier sessions" }}
          rowProps={(row, i) => ({
            ...nav.optionProps(i),
            "data-testid": `earlier-row-${row.key}`,
            "data-action": row.action,
            "aria-disabled": !takeable(row) || busy,
            className: [takeable(row) ? "ox-clickable" : "ox-inert", i === nav.cursor ? "ox-selected" : ""].filter(Boolean).join(" "),
            onClick: () => void takeRow(row),
          })}
        />
      ) : null}
    </>
  );

  return (
    <DocumentLayout
      testId="page-new-session"
      title="New session"
      slot="new-session.sections"
      slotParams={{ thread_id: threadId ? numericRowId(threadId) : null }}
      streamId={streamId}
      onOpenPage={onOpenPage}
      sections={[
        { id: "start", title: "Start a new session", testId: "new-session-new", body: start },
        ...(thread ? [{ id: "earlier", title: "Earlier sessions", testId: "new-session-earlier", body: earlierBody }] : []),
      ]}
    />
  );
}

const STATUS_ICONS: Record<EarlierStatus, LucideIcon> = {
  open_here: CircleDot,
  closed_here: RotateCcw,
  elsewhere: ArrowLeftRight,
  outside: ExternalLink,
  open_elsewhere: Lock,
};

/** What each status icon means, in the key under the filter. */
const STATUS_WORDS: Record<EarlierStatus, string> = {
  open_here: "open in this thread",
  closed_here: "closed here",
  elsewhere: "from another thread",
  outside: "not started in oxplow",
  open_elsewhere: "open in another thread",
};

/** A cell's value, or a dash when there's none to show. */
function Faint({ value }: { value: string | null }) {
  return <span className="ox-faint oxplow-tabular">{value ?? "—"}</span>;
}

/** The key to the status icons the list uses, in the order of `STATUS_ICONS`. */
function StatusKey({ statuses }: { statuses: EarlierStatus[] }) {
  const shown = (Object.keys(STATUS_ICONS) as EarlierStatus[]).filter((s) => statuses.includes(s));
  return (
    <p data-testid="new-session-earlier-legend" className="ox-faint ox-small" style={{ display: "flex", flexWrap: "wrap", gap: "var(--space-1) var(--space-4)", margin: 0 }}>
      {shown.map((s) => {
        const Icon = STATUS_ICONS[s];
        return (
          <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: "var(--space-1)" }}>
            <Icon size={12} aria-hidden />
            {STATUS_WORDS[s]}
          </span>
        );
      })}
    </p>
  );
}

/** Where a row stands, as an icon; its words on hover. */
function StatusIcon({ status, note }: { status: EarlierStatus; note: string }) {
  const Icon = STATUS_ICONS[status];
  return (
    <span title={note} aria-label={note} data-status={status} className="ox-faint" style={{ display: "inline-flex" }}>
      <Icon size={13} aria-hidden />
    </span>
  );
}
