/// `terminal` viz (`.context/terminal.md` "The terminal viz and its host"):
/// a terminal per row of the lens (`terminal.of`, a shell's URI), all
/// kept running, the picked one shown. With several, the strip picks one
/// — the lens's selection (a `select` column), so Back un-picks — and its
/// row menu renames (the lens's `rename` action, with the typed title) or
/// closes (its `close` action, which asks first). `+` opens another
/// (`oxplow.terminal.open`); a stream with none opens one the first time
/// the page shows. A row whose URI is an agent session's is that agent:
/// `stopped` and `conversation` (what it is bound to) come from the row,
/// Start and Start Fresh are its commands, Escape logs its interrupt in
/// the row's `thread`, and only real workspace files link.
import { useEffect, useRef, useState } from "react";

import { useLensActionRunner } from "./lensActionRunner.js";
import { recordUserInterrupt, runCommand, startAgentSession, startAgentSessionFresh, type LensRun, type SqlCell } from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { recordOpError } from "../opErrors.js";
import { TerminalTabStrip } from "../terminal/TerminalTabStrip.js";
import { TerminalView, type AgentTerminal } from "../terminal/TerminalView.js";
import { useWorkspaceLinkIndex } from "../useWorkspaceLinkIndex.js";
import { idOf, typeOf } from "../uri/uri.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { fileRefAt } from "../tabs/pageRefs.js";
import { rowRecord } from "./lensActions.js";
import { useLensActions } from "./LensResultView.js";
import { useLensStream } from "./LensStream.js";

const fail = (label: string) => (e: unknown) => recordOpError({ label, message: e instanceof Error ? e.message : String(e) });

export function TerminalViz({ run, onSetParam }: { run: LensRun; onSetParam?(name: string, value: SqlCell): void }) {
  const stream = useLensStream();
  const nav = useOptionalPageNavigation();
  const actions = useLensActions(run, stream?.id ?? null);
  const runLensAction = useLensActionRunner();
  const t = run.lens.terminal;
  const { columns, rows } = run.result;
  const at = (row: SqlCell[], column: string | null | undefined) => (column ? row[columns.indexOf(column)] ?? null : null);
  const list = rows
    .map((row) => ({ row, uri: String(at(row, t?.of) ?? ""), title: String(at(row, t?.title) ?? "") }))
    .filter((r) => r.uri);
  const isLinkablePath = useWorkspaceLinkIndex(list.some((r) => typeOf(r.uri) === "oxplow/agent_session") ? (stream?.id ?? undefined) : undefined);
  const truthy = (v: SqlCell) => v !== null && v !== 0 && v !== "" && v !== false && v !== "0";
  const agentOf = (r: { row: SqlCell[]; uri: string }): AgentTerminal | undefined => {
    const session = idOf(r.uri, "oxplow/agent_session");
    if (session === null) return undefined;
    const conversation = at(r.row, t?.conversation);
    const thread = at(r.row, t?.thread);
    const threadId = typeof thread === "string" ? (idOf(thread, "oxplow/thread") ?? thread) : null;
    return {
      stopped: truthy(at(r.row, t?.stopped)),
      conversation: conversation === null ? null : String(conversation),
      onStart: () => void startAgentSession(session).catch(fail("Start Agent")),
      onStartFresh: () => startAgentSessionFresh(session, true),
      onInterrupt: () => {
        if (threadId) void recordUserInterrupt(session, threadId, stream?.id ?? null);
      },
    };
  };
  // The pick is the lens's selection param; with none, the first.
  const select = run.lens.columns.find((c) => c.select);
  const picked = select?.select ? run.params[select.select] : null;
  const active = list.find((r) => String(at(r.row, select?.key) ?? "") === String(picked ?? "")) ?? list[0] ?? null;

  // A stream with no terminal opens one, once: the person opened the page.
  const [opening, setOpening] = useState(false);
  const seeded = useRef(false);
  const open = () => {
    if (!stream) return;
    setOpening(true);
    void runCommand("oxplow.terminal.open", { stream: stream.id })
      .catch(fail("Open terminal"))
      .finally(() => setOpening(false));
  };
  useEffect(() => {
    // Only a lens of shells opens one: it reads the stream's terminals.
    if (list.length > 0 || seeded.current || !stream || run.params.stream_id === undefined) return;
    seeded.current = true;
    open();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list.length, stream?.id]);

  const action = (id: string) => run.lens.actions.find((a) => a.id === id);
  const pick = (uri: string) => {
    const r = list.find((x) => x.uri === uri);
    if (r && select?.select) onSetParam?.(select.select, at(r.row, select.key));
  };
  const openFile = (abs: string, line?: number) => {
    const wt = stream?.worktree_path.replace(/\/+$/, "") ?? "";
    const rel = wt && abs.startsWith(`${wt}/`) ? abs.slice(wt.length + 1) : abs;
    nav?.navigate(fileRefAt(rel, line && line > 0 ? line : null), { newTab: true });
  };

  if (!stream) return <p className="ox-empty-line">No project is open.</p>;
  return (
    <div data-testid="lens-terminal" style={{ display: "flex", flexDirection: "row", flex: 1, minHeight: 0 }}>
      {actions.pending ? (
        <CommandConfirm
          label={actions.pending.action.label}
          command={actions.pending.action.command}
          details={actions.pending.details}
          onConfirm={actions.confirm}
          onCancel={actions.cancel}
          testIdPrefix="lens-action-confirm"
        />
      ) : null}
      {list.length > 1 ? (
        <TerminalTabStrip
          tabs={list.map((r) => ({ id: r.uri, title: r.title }))}
          activeId={active?.uri ?? ""}
          onActivate={pick}
          onNew={open}
          onClose={(uri) => {
            const r = list.find((x) => x.uri === uri);
            const close = action("close");
            if (r && close) actions.run(close, rowRecord(columns, r.row));
          }}
          onRename={(uri, title) => {
            const r = list.find((x) => x.uri === uri);
            const rename = action("rename");
            if (!r || !rename) return;
            void runLensAction(run.lens.id, rename.id, run.params, rowRecord(columns, r.row), stream.id, false, { title }).catch(fail(rename.label));
          }}
        />
      ) : null}
      <div style={{ position: "relative", flex: 1, minWidth: 0, minHeight: 0 }}>
        {list.length === 0 ? (
          <p className="ox-empty-line" data-testid="lens-terminal-empty">
            {opening ? "Opening a terminal…" : "No terminal is open."}{" "}
            {opening ? null : (
              <button type="button" className="ox-link" data-testid="terminal-new" onClick={open}>
                New Terminal
              </button>
            )}
          </p>
        ) : null}
        {list.map((r) => (
          <div
            key={r.uri}
            style={{ position: "absolute", inset: 0, display: r.uri === active?.uri ? "flex" : "none", flexDirection: "column" }}
          >
            <TerminalView
              uri={r.uri}
              agent={agentOf(r)}
              worktreePath={stream.worktree_path}
              onOpenFile={openFile}
              isLinkablePath={agentOf(r) ? isLinkablePath : undefined}
            />
          </div>
        ))}
      </div>
    </div>
  );
}
